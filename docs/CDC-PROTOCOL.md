# The CDC bootloader download protocol (reversed)

> **Status: reversed statically** from the vendor tool with Ghidra, then **fully confirmed on
> real hardware** — a complete native reflash of the stock `JA11_V2.2.bin` succeeded on
> 2026‑09‑05 (see *Hardware validation* below). Implementation: `proto::ktcdc` (packet
> framing, unit-tested) + `ktflash flash-cdc` (the state-machine driver in `main.rs`).

## Hardware validation (2026‑09‑04)

Confirmed against a live dongle (a Moondrop‑branded KT02H20 on the Windows box), driver‑free,
by opening the bootloader's CDC COM port from PowerShell `System.IO.Ports.SerialPort` and
sending the reversed tokens as the *first* bytes after a fresh unlock:

| Claim (from static RE) | Observed on hardware | Verdict |
| --- | --- | --- |
| unlock → re‑enumerates as `0x8888:0xCDC0` CDC serial | device instance id `USB\VID_8888&PID_CDC0\KT_VIRTUAL_COM_PORT`, appears as `COM3` | ✅ |
| `KTM` handshake = `1e 4b 54 4d` | on a **fresh** bootloader, `KTM` → reply `78` | ✅ |
| `0x78` (`'x'`) is the accept/ACK byte | handshake reply is exactly `0x78`; `KEY` → `78 00 78` | ✅ |
| `KEY` = `f0 4b 45 59` | ACKs `0x78` | ✅ |
| the bootloader is a **one‑shot sequential** state machine | after `KTM` is consumed, a repeated `KTM` gets no reply until re‑unlock — the parser has moved past ShakeHand | ✅ (matches the state model) |

`VER`/`CHP` did not answer standalone in this ordering — consistent with the state machine
gating them (they answer in a specific state / with data, not as bare ACKs); ordering to be
pinned when `KtCdcCodec` drives the full `Session`. **CRC‑32** was separately proven by a
byte‑exact match of the table at `DAT_0120e020` to the canonical CRC‑32 table (poly
`0xEDB88320`). No erase/PWO/KSTA/data packets were sent — flash was never touched.

### Hardware validation (2026‑09‑05, this Mac + OrbStack — no Mac mini)

Driven from the OrbStack Linux guest on the primary Mac (the dongle passed through with
`orb usb attach`); the whole flow ran on one machine, so the handoff's "move to a Mac mini"
step was unnecessary.

| Claim | Observed on hardware | Verdict |
| --- | --- | --- |
| `KTM` handshake → `0x78` on a fresh bootloader | `1e 4b 54 4d` → `78` | ✅ (re-confirmed) |
| `CHP` returns a chip-info blob | `d2 43 48 50` → `b2 85 40 12` + `"KT02H20B"` + `76` (13 B) | ✅ **new** |
| bootloader is one-shot (parser does not reset on port re-open) | after `KTM` consumed, re-open + `KTM` → no reply | ✅ |
| normal-mode `0x08` word-read reaches flash (Task B backup) | `dump` @ `0x0`/`0x80000`/`0x83000` → all `0x00`; `0x33` handshake times out | ❌ **disproven** — reads unmapped space, not flash |
| `RESET` (`ZRST`) from a *dirtied* mid-state | `5a 52 53 54` → no reply, stays in bootloader | ⚠️ only honored in the right state / from a fresh bootloader |
| **full native flash** (`ktflash flash-cdc --execute`, same `JA11_V2.2.bin`) | KTM→`78`, CHP→13B, ERASE→`78`, PWO→`78`, KSTA→`78`, **all 67 packets→`a5`**, STP→`78`, RESET→`78`, device re-enumerated to `2972:0102` and enumerates as a full JA11 | ✅ **PROVEN 2026‑09‑05** |

**Task A is done — the native CDC write works end-to-end on hardware.** `ktflash flash-cdc`
(`flasher/src/main.rs`) reflashed the stock `JA11_V2.2.bin` over the CDC bootloader (flag=0
path, base 0, auto-derived from `image[0x0F]`), all driven from the OrbStack guest on the
primary Mac. Real-world ACK detail: each data packet's reply is `78 a5` — a status `0x78`
precedes the `0xa5` block ACK — so the driver **drains the RX buffer before each send and
accumulates reads until the expected byte** (mirroring the vendor's `indexOf` + buffer-clear);
a single fixed-length read races the batched bytes and mis-reads the ACK. `CHP` returns a
13-byte chip-info blob whose lead/trail bytes vary per boot (`a2/b2 … 66/76`) with a fixed
`85 40 12 "KT02H20B"` core.

### Firmware READ / backup — not possible in software (definitive, 2026‑09‑05)

There is **no software way to read/back up the firmware off a KT02H20** through any vendor
command (verified by full static RE — see [`../research/cdc-re-findings.md`](../research/cdc-re-findings.md) §5):

- **CDC bootloader:** no read/dump/upload token exists (`FUN_00b8c380` has only the 10 known
  commands). `INF` is **not** a reader — its request is fixed (`f0 49 4e 46`, no address) and
  its reply is only the **(size, CRC‑32) of the whole image** (a verify/fingerprint), and it's
  gated to the `flag=1` path so it isn't even reachable standalone on a `flag=0` (JA11) device.
- **Normal‑mode `0x08` word‑read:** the vendor exe contains a real arbitrary reader
  (`FUN_0054d170`, cmd `0x08` on the `0x4b` channel) but it is **inert on this silicon** — the
  runtime `0xFF01` HID collection is an audio/EQ dispatcher (`W/R/S/C`; `R` reads *EQ
  coefficients*, not flash), with no `0x08` case, so `0x33` times out and `0x08` reads zeros.
  There is no ISP‑enter that keeps the device in normal mode with reads enabled (`0x54` always
  reboots into the bootloader).
- **The only reliable live reads** are `CHP` (chip‑ID `KT02H20B`) and, on a `flag=1` flash,
  the `INF` (size, CRC‑32) fingerprint. Neither returns flash contents.

**A true backup requires hardware access** to the resident `KT_USB_BOOT` ROM / lower‑flash
region (code below load `0x80000`, absent from the app image): JTAG/SWD, chip‑off, glitching,
or a KTMicro factory ISP tool. **Therefore: keep the manufacturer's original firmware image
before flashing — it cannot be recovered off the device afterward.**

After the normal‑mode `0x54 "T12345678"` unlock (see `PROTOCOL.md`), the dongle
re‑enumerates as a **USB CDC serial device `0x8888:0xCDC0`** and is driven over that serial
link by a state machine. Sources: `FUN_00b8f200` (idle/timer states), `FUN_00b8c380`
(response‑driven states), `FUN_0054d7d0` (data packet), `FUN_005733a0` (final packet),
`FUN_00d2aa60` (`QByteArray::indexOf` ACK check), `FUN_006cf5b0`/`FUN_006cc3d0`/`FUN_006cbaf0`
(byte‑array build/prepend/append).

## Transport
- USB **CDC**: bulk **`0x03` OUT / `0x83` IN** (interface 1); appears as `/dev/ttyACM0`.
- Host asserts DTR/RTS (opens the port) then exchanges framed messages.

## Commands (fixed tokens)
Sent as raw byte arrays via the send wrapper. Each is a lead byte + 3 ASCII chars, except the
data packets and the binary erase command.

| State | Token | Bytes | Meaning (inferred) |
| --- | --- | --- | --- |
| shake | `KTM` | `1e 4b 54 4d` | handshake / hello |
| 1 | `VER` | `f0 56 45 52` | get version |
| 1 | `CHP` | `d2 43 48 50` | chip id |
| 2 | `KEY` | `f0 4b 45 59` | key / auth |
| 4 | *(erase/setup)* | `2d 29 00 10 0e 15 00 60 00 bc` (10 B) | region/erase params |
| 5 | `PWO` | `3c 50 57 4f` | power / prepare |
| 6 | `KSTA` | `4b 53 54 41` | start programming |
| 9 | `STP` | `96 53 54 50` | stop |
| 10 | `INF` | `f0 49 4e 46` | info / verify (flag=1 path only) |
| 10→ | `RESET` | `5a 52 53 54` (`"ZRST"`) | reset into new firmware (acks `0x78`, then "UPGRADE FIRMWARE SUCESS") |

## Data packet framing

Firmware is programmed in **1024‑byte payload packets** (`param_3 * 0x400` offsets), grouped in
`bank<<15` (32 KB) banks. Each packet, built by `FUN_0054d7d0`:

```
+----------------+------------------+----------------+
| 6-byte header  |  payload ≤1024 B |  CRC-32 (4 LE) |
+----------------+------------------+----------------+
```

- **Header (6 B)** — *byte-exact, decompiled from `FUN_0054d7d0`/`FUN_005733a0`* (implemented +
  unit-tested in `flasher/src/proto/ktcdc.rs`):

  | Byte | Value |
  |---|---|
  | H[0] | `0x69` (constant marker) |
  | H[1] | `L & 0xFF` (payload length low 8 bits) |
  | H[2] | `((L >> 8) & 0x1F) \| (top3 << 5)` |
  | H[3] | `addr & 0xFF` |
  | H[4] | `(addr >> 8) & 0xFF` |
  | H[5] | `(addr >> 16) & 0xFF` |

  `L` is a 13-bit length (≤0x400). `addr` is a 24-bit LE flash address = `base + block*0x400`
  (block 0 special: `base+0x10`, payload = image[0x10..0x400] = 1008 B). `base = flag<<15`.
  `top3` (H[2] bits 5-7) = the bank id on a region-boundary packet (`block % 0x20 == 0`, value
  `1` for the JA11 image), else `0b111`. The **final packet** (`FUN_005733a0`) is fixed
  `69 10 E0 <addr>` — it writes the image's **first 16 bytes last**, making the image bootable.
- **CRC‑32** (table at `DAT_0120e020`, `table[1] == 0x77073096`, poly `0xEDB88320`, reflected)
  — ⚠️ **computed over `header(6) + payload`, NOT payload-only** (the header is prepended
  *before* the CRC loop), with **init = 0 and no final XOR** (differs from the standard
  zip/PNG CRC‑32's `0xFFFFFFFF` init/xorout). Appended little‑endian. *(Corrected 2026-09-05
  from an earlier "over the payload" claim.)*

## Response / ACK
The parser checks the received buffer with `QByteArray::indexOf(byte)`:

| Byte | Where | Meaning (inferred) |
| --- | --- | --- |
| `0x78` (`'x'`) | states 5–7 | command accepted / ready |
| `0xa5` | state 8 (per data block) | block ACK |
| `0x03` | state 8 | status / done |

A block is re‑sent / the flow stalls until the expected byte appears; `ShakeHand out
TSTEP_Idle `/` serial not found` are the timeout messages.

## Sequence (state machine)

```
find serial (VID 8888 / PID cdc0)
  → KTM (handshake)
  → VER / KEY / CHP     (version, key, chip; responses memcmp-verified)
  → erase/setup (10-byte 2d 29 …)
  → PWO  (prepare)
  → KSTA (start)
  → [ data packet × N ]  1 KB payload, CRC-32, wait ACK 0xa5   ← the program loop
  → final packet (FUN_005733a0, bank 0xE0)
  → STP → INF → RESET → "UPGRADE FIRMWARE SUCESS"
```

## Implementing `KtCdcCodec`
1. Encode the fixed tokens + the 10‑byte erase command as constants.
2. Encode data packets: build the 6‑byte header (nail the bit layout with tests), append the
   1 KB payload, append CRC‑32. Chunk the `KT_Helios` image into 1 KB packets across 32 KB banks.
3. Decode responses via `indexOf` of the ACK bytes; drive the `Session` state machine.
4. Validate with `bootdiag --replay` golden fixtures + the `FakeBootloader` **before** any
   hardware write. A single confirming USB capture (if one can be obtained) is the belt‑and‑
   suspenders check on the header bit‑packing.
