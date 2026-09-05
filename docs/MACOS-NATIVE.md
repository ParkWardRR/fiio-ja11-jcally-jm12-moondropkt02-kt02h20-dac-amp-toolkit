# Plan — native macOS, without OrbStack

**Status:** plan / investigation · **Companion to:** [`RELEASE-PLAN.md`](RELEASE-PLAN.md) (M8)
**Goal:** `ktflash unlock` and `ktflash flash-cdc` run on stock macOS with no OrbStack, no
container, no VM, no kext, and — critically — **no Apple entitlement that would require a paid
Developer account**, so it still ships as the unsigned/ad-hoc build from `RELEASE-PLAN.md` §4.

---

## 1. What macOS actually blocks

Today `orbstack/ktflash-orbstack.sh` exists for one reason, stated in
[`FLASHING.md`](FLASHING.md):

> macOS's `IOHIDFamily`/`AppleUSBAudio` drivers **own** the dongle's interface: libusb returns
> `LIBUSB_ERROR_ACCESS`, and `IOHIDDeviceSetReport` goes down the control pipe the firmware
> ignores.

Both halves are about the **normal-mode HID interface**. libusb on Darwin cannot detach a kernel
driver (there is no `libusb_detach_kernel_driver` on macOS), so `claim_interface` fails whenever
IOHIDFamily has already matched.

**This is worth restating precisely, because it changes the plan:** the blocker applies to the
dongle in **normal mode**. It says nothing about the device *after* `unlock`.

---

## 2. The key insight — split the problem in two

After `unlock`, the dongle re-enumerates as `8888:cdc0`, and per
[`CDC-PROTOCOL.md`](CDC-PROTOCOL.md):

> USB **CDC**: bulk **`0x03` OUT / `0x83` IN** (interface 1); appears as `/dev/ttyACM0`.
> Host asserts DTR/RTS (opens the port) then exchanges framed messages.

That is a **plain serial device**. On macOS, `AppleUSBCDCACM` matches it and publishes
`/dev/cu.usbmodem*` — and those bulk endpoints are exactly what the tty *is*. The whole firmware
download protocol is a byte stream over that link.

**⇒ We do not need to fight macOS for the bootloader at all. We need to stop using libusb for it
and open the tty instead.** macOS owning the CDC interface stops being a problem and becomes the
delivery mechanism.

So:

| Stage | Native macOS today | Native macOS plan | Confidence |
|---|---|---|---|
| **A. `unlock`** (HID report `0x54 "T12345678"` → interrupt-OUT `0x03`) | ❌ blocked | §4 — needs investigation | ⚠️ genuinely uncertain |
| **B. `bootdiag` / `flash-cdc`** (CDC bulk) | ❌ via libusb | §3 — open `/dev/cu.usbmodem*` | ✅ high — it's a serial port |

Stage B is a design change we can make with confidence. Stage A is a bounded investigation. They
are independent: **Stage B is worth doing even if Stage A fails**, because it also fixes the same
problem on Linux (no interface claim, no `cdc_acm` fight, no ModemManager race).

---

## 3. Stage B — a serial `Transport` for the bootloader

### 3.1 It slots into an existing seam

`proto::cdc::Transport` (`flasher/src/proto/cdc.rs:130`) is already the right abstraction —
`send(&[u8])` / `recv(timeout)`, nothing USB-specific. `RusbBootloaderTransport`
(`flasher/src/usbtransport.rs`) is one implementation; a `SerialTransport` is a second.

**But there is a refactor to do first:** `cmd_flash_cdc` does **not** use the trait. It calls
`open_bootloader()` and drives `rusb` `read_bulk`/`write_bulk` directly
(`flasher/src/main.rs:914`, via `open_bootloader` at `flasher/src/main.rs:787`). Step one is routing
`cmd_flash_cdc` and `cmd_bootdiag` through `proto::cdc::Transport` so a transport can be swapped
without touching protocol logic.

### 3.2 Design

```
ktflash flash-cdc --image fw.bin --execute --yes            # auto: tty on macOS, libusb on Linux
ktflash flash-cdc --transport serial --port /dev/cu.usbmodem1101 …
ktflash flash-cdc --transport usb …                         # force the existing path
```

- **Port discovery:** enumerate `/dev/cu.usbmodem*` (macOS) or `/dev/ttyACM*` (Linux) and match
  the one whose IOKit/sysfs parent is `8888:cdc0`. Never guess by index — a user may have other
  serial devices attached.
- **Use `/dev/cu.*`, not `/dev/tty.*`** on macOS. The `tty.*` node blocks on carrier detect;
  `cu.*` (callout) does not. Opening `tty.*` is a classic hang.
- **Line discipline:** raw mode (`cfmakeraw`), 115200 8N1, no flow control — matching the
  `SET_LINE_CODING` the current code already sends (`main.rs:822`,
  `usbtransport.rs:95`). Assert DTR/RTS explicitly with `TIOCMBIS` rather than relying on
  open-time behaviour.
- **Framing:** a tty gives a byte *stream*, not packet boundaries. `recv` must accumulate until a
  full frame is decodable instead of returning one 64-byte bulk read. `usbtransport.rs:13-15`
  already flags this as a known rough edge in the USB path — fixing it once in the `Transport`
  layer serves both.
- **Timeouts:** `VMIN=0`/`VTIME`, or `poll()` on the fd. Must honour the existing
  `RetryPolicy::read_timeout`.
- **Dependencies:** implement with `libc` directly (already in the tree transitively, MIT/Apache —
  no new `cargo deny` question) rather than pulling in `serialport`. `proto/` stays dependency-free.

### 3.3 Permissions

`/dev/cu.usbmodem*` on macOS is `crw-rw-rw- root:wheel` — **usable without `sudo`**. On Linux the
existing udev rules already cover the tty
(`packaging/99-ktflash.rules`, the `SUBSYSTEM=="tty"` line). So Stage B needs no privilege
escalation on either OS.

### 3.3b Resolving the port: solved, in Swift

Matching a `/dev/cu.*` node back to its USB device is the one piece the Rust side cannot yet do
properly. On Linux it is exact via sysfs (`/sys/class/tty/ttyACM0/device/../idVendor`); on macOS
`serialtransport.rs` falls back to "accept any `/dev/cu.usbmodem*`", which is not good enough —
a wrong guess aims a firmware write at the wrong device.

[`macos/native/ktmac`](../macos/native/ktmac/Sources/KTMacKit/SerialPortFinder.swift) does it
correctly: enumerate `IOSerialBSDClient` services, read `kIOCalloutDeviceKey` for the path, then
`IORegistryEntrySearchCFProperty` with `kIORegistryIterateParents` to pull `idVendor`/`idProduct`
down from the owning `IOUSBHostDevice` several levels up the IOService plane. Either port that
into Rust or shell out to `ktmac port`.

### 3.4 Bonus: this improves Linux too

The serial transport sidesteps `cdc_acm` detachment, the interface claim, **and** the
ModemManager race described in [`RELEASE-PLAN.md`](RELEASE-PLAN.md) §5.3 — instead of racing
ModemManager for the interface, we use the same tty it would have opened, and the
`ID_MM_DEVICE_IGNORE` rule keeps it out of the way. Consider making serial the default transport
on both platforms once proven.

---

## 4. Stage A — native `unlock`

The one genuinely hard piece: get the 10 bytes `54 31 32 33 34 35 36 37 38 00`
(report `0x54`, `"T12345678"` — [`PROTOCOL.md`](PROTOCOL.md)) onto **interrupt-OUT endpoint
`0x03`** of the vendor HID collection (usage page `0xFF01`), on a device IOHIDFamily owns.

### 4.0 First: resolve a contradiction in our own docs

Two files in this repo disagree, and the plan hinges on which is right:

- [`PROTOCOL.md`](PROTOCOL.md) — *"`IOHIDDeviceSetReport` sends output reports down the control
  pipe, which the firmware ignores (it reads the interrupt-OUT endpoint)."*
- [`dongle-investigation.md`](dongle-investigation.md) §Cross-device conclusions #3 — *"Mac-side
  transport is fully proven. Open (shared), GET_REPORT, and SetReport all work natively via
  `IOHIDManager` — no Windows needed for the transport."*

These are reconcilable — the investigation shows the **API call returns success**, `PROTOCOL.md`
claims the **device never acts on it** — but only one of them can be true about where the bytes
land. The investigation was also done against a KZ C04 (`31b2:0313`), not the JA11. **Resolving
this is experiment E1 and it costs almost nothing.**

### 4.0b Already found: TCC gates E1/E2 before USB is even reached

Running the E1 probe (`macos/native/e1_hid_setreport.c`) on macOS 15 with **no device
attached** returns `0xe00002e2` = `kIOReturnNotPermitted` from `IOHIDManagerOpen` — before any
USB device is touched.

That is **macOS TCC, not a USB problem**: since 10.15, opening an `IOHIDManager` requires
**Input Monitoring** consent for the calling application (for a CLI, whichever terminal app
launched it). Grant it in System Settings → Privacy & Security → Input Monitoring, then restart
the terminal.

Two consequences:

1. **E1/E2 cannot be evaluated at all until this is granted** — a "no HID devices found" result
   before granting it means nothing.
2. If `ktflash` ships an `IOHIDManager`-based unlock, **a TCC consent prompt becomes part of the
   macOS first-run experience** and must be documented. E3 (`IOUSBLib`) does not go through
   IOHIDFamily and so likely avoids this, trading a consent prompt for a `sudo` requirement —
   worth weighing when choosing between them.

### 4.1 Experiments, cheapest first

Each has a clear pass/fail: **pass = the dongle re-enumerates as `8888:cdc0`.** That is an
unambiguous, observable signal, which makes this investigation cheap to run and impossible to
fool ourselves about. All are recoverable — a power-cycle returns the device to normal mode.

| # | Approach | Cost | Notes |
|---|---|---|---|
| **E1** | `IOHIDDeviceSetReport(dev, kIOHIDReportTypeOutput, 0x54, buf, 10)` via `IOHIDManager`, matching the `0xFF01` collection | ~1h | Settles §4.0. Apple's USB HID driver has historically preferred the interrupt-OUT pipe for output reports *when the interface declares one* — and this device does (`0x03` OUT). Worth testing before believing the pessimistic doc |
| **E2** | E1 + `IOHIDDeviceOpen(dev, kIOHIDOptionsTypeSeizeDevice)` | +1h | Documented IOKit way to take exclusive ownership from other clients. May need root |
| **E3** | `IOUSBInterfaceInterface` + **`USBInterfaceOpenSeize()`** → `WritePipe` on `0x03` | ~half day | The classic macOS route to seize an interface from a kernel driver, **no kext and no DriverKit entitlement**. Likely needs root. Whether macOS still permits seizing from IOHIDFamily on current releases is the open question |
| **E4** | `IOUSBHostInterface` (IOUSBHost.framework) | ~half day | More modern API; may require entitlements — check before investing |
| **E5** | DriverKit DEXT with `com.apple.developer.driverkit.transport.usb` | weeks | **Last resort.** Needs Apple to grant a restricted entitlement *and* Developer ID signing — which contradicts the unsigned-release goal. Only if E1–E4 all fail and it's still worth it |

Prototype E1–E3 as a small standalone C or Rust program against IOKit before touching `ktflash` —
faster to iterate, and a negative result costs nothing.

### 4.2 If Stage A fails

Be honest about the outcome rather than forcing it:

- **Stage B still lands.** macOS goes from *"OrbStack for everything"* to *"OrbStack for one
  command"*, and `flash-cdc` — the long, risky, interruptible part — runs natively. That is most
  of the value.
- A **`sudo` requirement** for `unlock` (if E3 needs root) is an acceptable outcome. It's still
  native, still no OrbStack, and destructive-command friction is not a bad thing here.
- Do **not** ship a codeless kext or ask users to disable SIP. Those are worse than OrbStack.

---

## 5. Milestones

| # | Milestone | Needs hardware? |
|---|---|---|
| **N0** | Confirm `8888:cdc0` publishes `/dev/cu.usbmodem*` on macOS (unlock via OrbStack once, then look) | ✅ yes — 10 minutes |
| **N1** | Refactor `cmd_flash_cdc` + `cmd_bootdiag` onto `proto::cdc::Transport` | ❌ no — unit-testable |
| **N2** | `SerialTransport` (`--transport serial --port …`), frame accumulation, `libc` termios | ❌ to write, ✅ to prove |
| **N3** | Prove `bootdiag` then `flash-cdc` over the tty on macOS | ✅ yes |
| **N4** | E1/E2 — native HID `unlock` via `IOHIDManager`; settles §4.0 either way | ✅ yes |
| **N5** | E3 — `USBInterfaceOpenSeize` fallback, only if N4 fails | ✅ yes |
| **N6** | Auto transport selection; docs rewritten; `orbstack/` demoted to a fallback appendix | — |

**N1 and N2 need no dongle** — they are ordinary refactoring against an existing trait with
existing fixtures, and they are on the critical path. Start there.

> **N1-N3 have landed** in `flasher/src/` (driver refactor, serial transport, journal,
> post-reset reprobe). **N4/N5 live in [`macos/native/`](../macos/native/)** — the `ktmac`
> Swift package plus the two C probes. Everything builds and self-checks; none of it has
> touched hardware.

N0 is the single cheapest high-information step in this document: if `/dev/cu.usbmodem*` does not
appear, the whole Stage B design needs rethinking, and we'd rather know in ten minutes.

---

## 6. Risks

| Risk | Mitigation |
|---|---|
| `AppleUSBCDCACM` doesn't match `8888:cdc0` (malformed CDC descriptors) | N0 checks this first, before any code is written |
| tty layer mangles bytes (`0x11`/`0x13` flow control, `0x1a`, CR/LF translation) | `cfmakeraw` + explicitly clear `IXON`/`IXOFF`/`ICRNL`/`OPOST`; the protocol's tokens include `0x1e`, `0x2d`, `0x3c`, `0x96` — **binary-safe termios is mandatory, not optional** |
| Frame-boundary bugs in stream mode corrupt a flash | Frame accumulation lands in `proto/` where it is unit-testable against `fixtures/`; exercise via `bootdiag --replay` before touching hardware |
| Stage A is genuinely impossible without entitlements | §4.2 — partial win is still a real win; don't force E5 |
| Divergence between USB and serial transports | Keep both behind `Transport`; run the same replay fixtures through each |
| macOS TCC (Input Monitoring) blocks HID access | **Confirmed** (§4.0b) — affects E1/E2 only; detected and explained by the probe |

---

## 7. What this would change in the docs

`README.md` ("Where it runs" table), `FLASHING.md` ("Why macOS needs OrbStack" — becomes "if you
need OrbStack"), `PROTOCOL.md` §macOS caveat (rewrite per the E1 result),
`usbtransport.rs` module docs, `ROADMAP.md`, and `orbstack/README.md` (demoted, not deleted — it
stays as the known-good fallback).
