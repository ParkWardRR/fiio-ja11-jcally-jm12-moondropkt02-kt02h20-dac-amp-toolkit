# How the protocol was extracted from the vendor binaries

Everything in `PROTOCOL.md` was recovered by statically reverse‑engineering
the **Windows‑only vendor package**, not from any datasheet. This page documents the
method so it's reproducible and auditable.

## Inputs (the "JadeAudio JA11 Firmware V2.2" package)

| File | What it is |
| --- | --- |
| `JadeAudio JA11 Upgrade Tool.exe` | 20 MB **PE32** GUI app — Qt 5 + **statically‑linked `hidapi`** + Qt **QSerialPort**. This is the flasher whose protocol we want. |
| `Firmware/JadeAudio JA11_V2.2.bin` | 67,312‑byte firmware image (see structure below). |
| `KT_USB_BOOT_UI … 1.2.10.ini` | Config: `HomeVID=0x31b2`, `BinPath=…`. The tool is KTMicro's generic **`KT_USB_BOOT`** UI; the `.exe`'s real name is `KT_USB_UPGRADE_UI_021X_0211L_02H0X_02F0X.exe`. |

## Tools used

- **rizin** — headless disassembly + analysis (`rizin -q -c '…'`, saved project). Did the
  heavy lifting: it recognised the statically‑linked `hidapi` functions by signature.
- **Ghidra** (headless `analyzeHeadless`) with **Java** post‑scripts — clean C decompilation
  of the command builders and the orchestrator.
- `xxd` / `strings` — firmware structure and UTF‑16 QString scraping.

> Dead end worth noting: a first pass with a linear capstone sweep found nothing, because
> the HID functions are **resolved at runtime via `GetProcAddress`**, so there are no
> static import thunks to anchor on, and linear disasm desyncs on Qt data. Recursive
> analysis (rizin/Ghidra) + string‑xref anchoring is what worked. (No Python in the
> reproducible path.)

## Step 1 — Firmware image structure

```
xxd 'JadeAudio JA11_V2.2.bin' | head
```

| Offset | Bytes | Meaning |
| --- | --- | --- |
| `0x00` | `KT_Helios_v1b___` | bootloader magic |
| `0x10` | `KT02H20B` | **target chip** |
| `0x18` | `Size` + `f0 06 01 00` | `0x000106F0` = 67,312 = the file length |
| `0x20` | `75503ea` | build git hash |
| `0x30` | `2025-06-30 V:1.0` | date / version |
| `0x40` | `ENTY` + `00 30 08 00` + `88 d1 01 00` | entry table: load `0x00083000`, size `0x1D188` |

Per‑4 KB entropy ≈ 6.5 (normal compiled code) → the image is **plaintext, not encrypted or
compressed**. `strings` also shows `FIIO`, `JadeAudio JA11`, and the serial
`2020-02-20-0000-0000-0000` embedded — i.e. the image carries the device's USB identity.

## Step 2 — Find the transport (it's hidapi over HID)

```
# HID function-name strings are consecutive in .rdata …
rizin -q -c 'izz~HidD_' "$EXE"
# … referenced from ONE resolver function that GetProcAddress's them into a global table:
rizin -q -c '/v4 0x0113e136' "$EXE"     # xref to the "HidD_SetFeature" string
rizin -q -c 'pd 90 @ 0x01090250' "$EXE" # the resolver: fills g_HidD_* pointers
```

With the binary analysed (`aac; aar`), rizin **recognised the statically‑linked hidapi**:
`hid_write`, `hid_read_timeout`, `hid_send_feature_report`, `hid_open`, `hid_enumerate`…
Key finding: `hid_send_feature_report` has **no callers** — the tool flashes with
`hid_write`/`hid_read_timeout` (interrupt reports), *not* feature reports. This is exactly
why macOS `IOHIDDeviceSetReport` (control pipe) elicits no response and the real work needs
the interrupt endpoints.

## Step 3 — Recover the command builders (the 0x4B frame)

```
rizin -q -c 'Po project.rzdb; axt @ <hid_write_addr>' "$EXE"   # all call sites
# disassemble each call site's buffer setup: mov byte [esp+off], imm
```

Every `hid_write` site builds an **11‑byte report, ID `0x4B`**, of the form
`4b | addr[4] | cmd | 00 | data[4]`:

| cmd | op | seen at |
| --- | --- | --- |
| `0x33` / `0x32` | status / handshake (expect reply word `3`) | `FUN_0054d0e0` |
| `0x08` | read 32‑bit word @ addr | `FUN_0054d330` |
| `0x21` | erase region (data = size) | `FUN_006246c0` |
| `0x88` | write 32‑bit word (no reply) | `FUN_005583a0` |

Separately, `FUN_006260b0` builds **report `0x54` = ASCII `"T12345678"`** — the unlock.

## Step 4 — Clean C via Ghidra headless (Java)

```
analyzeHeadless <proj> ja11 -import "$EXE" -scriptPath . -postScript DecompHID.java
```

`DecompHID.java` opens a `DecompInterface`, resolves functions by address, and prints
`getDecompiledFunction().getC()`. (Java, because headless Ghidra here has no PyGhidra —
a `.py` post‑script errors out.) This gave the orchestrator in readable C:

```c
FUN_00b8ed20():                 // "UpGrade" handler
  if (firmware.isEmpty()) error("bin is empty");
  ok = FUN_00624680(vid, pid);  // hid_open(0x31b2, …)
  if (ok) FUN_006260b0();        // send "T12345678"  ← reboots to bootloader
  FUN_00cd3f00();                // Qt-threaded flash worker
```

## Step 5 — The bootloader is a *different* protocol (serial)

`FUN_006260b0` ("T12345678") reboots the device; empirically it re‑enumerates as a **USB
CDC device `0x8888:0xCDC0`**. Scraping UTF‑16 QStrings and the `OUT:` /`IN:`  log prefixes
exposed the download engine:

```
strings "$EXE" | grep -i serial     # QSerialPort, SerialRecive(), ReadSerial
```

The flasher drives the bootloader over **`QSerialPort`** with a named state machine
(`Shake hand`, `Erase`, `Program`, `UPGRADE FIRMWARE SUCCESS`). `FUN_00b8c380` is the
`SerialRecive` parser, `FUN_00b8f200` the idle/timer states, `FUN_0054d7d0` the data‑packet
builder, `FUN_005733a0` the final packet, `FUN_00d2aa60` the ACK check.

**Update — this is now reversed** (statically, no capture): the download uses `[0x69 header |
≤1024 B payload | CRC‑32]` data packets plus fixed 4‑byte command tokens (`KTM`/`VER`/`KEY`/
`CHP`/`PWO`/`KSTA`/`STP`/`INF`) and ACK bytes `0x78`/`0xa5`/`0x03`. Full spec →
[`CDC-PROTOCOL.md`](CDC-PROTOCOL.md). Implementation target: `cdc::KtCdcCodec`.

## Reproduce it

The `.exe` is not redistributed here (it's FiiO/JadeAudio's). Get the "JadeAudio JA11
Firmware V2.2" package from FiiO, then run the rizin/Ghidra steps above. The Ghidra Java
script and rizin command log live in [`../research/`](../research/).
