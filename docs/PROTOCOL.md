# The KTMicro `KT_USB_BOOT` HID ISP protocol

Reverse-engineered from the **JadeAudio JA11 Upgrade Tool** (`JadeAudio JA11 Upgrade
Tool.exe`, Windows-only PE32, statically-linked **hidapi** + Qt5) using
[rizin](https://rizin.re/) and [Ghidra](https://ghidra-sre.org/) headless decompilation.

Everything here is recovered by static analysis (see [`EXTRACTION.md`](EXTRACTION.md) for
the method). The reference implementation is the Rust CLI in [`../flasher/`](../flasher/)
(`probe`/`handshake`/`unlock`/`bootdiag`, via `rusb`). Where the original tool hid logic
behind Qt's threaded‑functor indirection (the chunked write loop), that is called out.

> [!NOTE]
> This page documents the **normal‑mode HID protocol**. The actual firmware **download**
> happens *after* the unlock, in a separate **CDC serial bootloader** (`0x8888:0xCDC0`) —
> see [Bootloader](#bootloader-the-actual-download-target) below. **Corrected 2026‑09‑05:**
> earlier revisions of this page claimed macOS couldn't drive the unlock natively
> (`IOHIDDeviceSetReport` supposedly going down a control pipe the firmware ignores). That was
> wrong for the JA11 — confirmed on real hardware via `IOHIDManager`, see the macOS caveat below.

---

## Transport

- Plain **HID output reports** (`hid_write`, interrupt‑OUT ep `0x03`) and **input reports**
  (`hid_read_timeout`, interrupt‑IN ep `0x83`). Not feature reports, not `DeviceIoControl`.
  The reference CLI (`ktflash`) uses **`rusb`/libusb** interrupt transfers (Linux/OrbStack —
  `rusb` on macOS still can't claim this interface). A **second, native macOS path** exists via
  `IOHIDManager` (the `ktmac` companion tool, `staging/macos-native/ktmac`) — same wire bytes,
  different API, no interface claim needed. Not yet merged into `ktflash` itself.
- Commands live on the vendor HID collection at **usage page `0xFF01`**, report IDs
  `0x4B` and `0x54` — **not** the media-key collection (usage page `0x0C`).
- **macOS caveat (corrected 2026‑09‑05):** this page previously claimed
  `IOHIDDeviceSetReport` couldn't reach the device natively on macOS. **That was wrong**,
  confirmed by direct experiment (`staging/macos-native/ktmac`, E1 in
  [`MACOS-NATIVE.md`](MACOS-NATIVE.md) §4): `IOHIDDeviceSetReport(dev, kIOHIDReportTypeOutput,
  0x54, buf, 10)` via `IOHIDManager`, matching the `0xFF01` collection, works — the device
  re‑enumerates as `8888:cdc0` exactly as it does via Linux/OrbStack. The only prerequisite is
  **Input Monitoring** consent for the calling app (macOS TCC, since 10.15) — without it,
  `IOHIDManagerOpen` fails with `kIOReturnNotPermitted` before any USB device is even touched,
  which is what produced the original false negative. libusb still can't claim the
  kernel‑owned *normal‑mode* interface — that part was correct — but the HID route sidesteps
  it entirely, and `ktflash` never needed libusb for `unlock` in the first place.
- Device: **VID `0x31B2`** (KTMicro). The JA11's runtime PID is `0x0111`.

## Commands

### Unlock / enter ISP

Report **`0x54`** followed by the ASCII string `"12345678"` and a trailing `0x00`
— 10 bytes total, i.e. on the wire `54 31 32 33 34 35 36 37 38 00` ("T12345678").

Origin: `FUN_006260b0`.

### Uniform command frame (11 bytes)

```
 byte:  0    1  2  3  4    5     6     7  8  9  10
        4B   <-- addr -->  cmd   00    <-- data -->
             (u32 LE)                  (u32 LE)
```

The response mirrors the frame; the **32-bit result is at `resp[7..11]` (LE)**.

| cmd    | meaning                         | data field | reply                    | source          |
|--------|---------------------------------|------------|--------------------------|-----------------|
| `0x33` | handshake / status (addr 0)     | 0          | result `== 3` when ready | `FUN_0054d0e0`  |
| `0x32` | handshake / status (addr 0)     | 0          | status word              | `FUN_0054d0e0`  |
| `0x08` | read 32-bit word `@ addr`       | 0          | result = the word        | —               |
| `0x21` | erase region `@ addr`           | size       | ack                      | —               |
| `0x88` | write 32-bit word `@ addr`      | value      | **no reply**             | `FUN_005583a0`  |

### Orchestration

`FUN_00624680` opens `hid_open(VID=0x31b2, PID)` into a global handle. The top-level
flash routine `FUN_00b8ed20`: check image non-empty → connect → unlock → hand off to
`FUN_00cd3f00` (the Qt-threaded flash worker) → cleanup.

---

## Bootloader: the actual download target

The `0x54 "T12345678"` unlock does **not** flash over HID — empirically it **reboots the
device**, which **re‑enumerates as a new USB device `0x8888:0xCDC0`**: a **CDC serial**
device (interface 1 = bulk EP `0x03` OUT / `0x83` IN; appears as `/dev/ttyACM0`, product
`"KTMicro 2021-07-15-…"`). So the normal‑mode `0x4B` frame set above is the *app/EQ* channel;
the firmware **download** is a serial protocol to this bootloader.

The vendor tool drives it over Qt **`QSerialPort`** with a named state machine
(`Shake hand` → `Erase` → `Program` → `UPGRADE FIRMWARE SUCCESS`): `FUN_00b8c380` is the
`SerialRecive` parser, `FUN_00b8bfd0` the `OUT:`‑logging send wrapper. That serial byte framing
is now **fully reversed** and documented in [`CDC-PROTOCOL.md`](CDC-PROTOCOL.md) — implemented
in `ktflash flash-cdc` (framing in [`../flasher/src/proto/ktcdc.rs`](../flasher/src/proto/ktcdc.rs))
and proven by a full hardware reflash.

---

## Firmware image format — `KT_Helios`

From `JadeAudio JA11_V2.2.bin` (67,312 bytes). Plaintext — entropy ≈ 6.5, not
encrypted or compressed.

| offset  | field                                             |
|---------|---------------------------------------------------|
| `0x00`  | magic `KT_Helios_v1b`                             |
| `0x10`  | chip string `KT02H20B`                            |
| `0x18`  | `Size` = `0x106F0`                                |
| `0x20`  | build hash                                        |
| `0x30`  | build date                                        |
| `0x40`  | `ENTY` table (load addr `0x00083000`, size `0x1D188`) |

The image embeds the strings `FIIO` / `JadeAudio JA11` and a serial
`2020-02-20-0000-0000-0000`.

The flasher validates the `KT_Helio` magic before it will touch a device, so it
refuses to write a foreign blob.

---

## Applying this to other dongles

The protocol above was recovered from the **FiiO / JadeAudio JA11** (KTMicro `KT02H20`,
VID `0x31B2`, PID `0x0111`). It is the map, not the territory — each other dongle needs
its own confirmation before a write is trusted.

**Confirmed same family — KZ Acoustics C04.** A KTMicro USB-C DAC, same vendor `0x31B2`,
same HID control convention (vendor page `0xFF01`, report IDs `0x4B`/`0x54`, here in a
10-byte form), PID `0x0313`. That shared convention is *why* the JA11 protocol is the
natural starting point for it. Still unproven for the C04 specifically: whether it enters
the same ISP mode with the same unlock string, and its flash **base**/**erase geometry**.

**Named target — JCALLY JM12.** A popular USB-C DAC/amp dongle in the same class, on the
toolkit's roster but **not yet probed** — its silicon and control channel still need to be
fingerprinted (`probe`) before any protocol claim.

For **any** device, the safe path is identical: `probe` → `handshake` → `unlock` →
`bootdiag`, and (to help finish the serial protocol) capture the vendor's own tool once —
a USB capture, or an Android USB/HCI snoop — to pin down the download framing. See
[`dongle-investigation.md`](dongle-investigation.md) for the full device survey.
