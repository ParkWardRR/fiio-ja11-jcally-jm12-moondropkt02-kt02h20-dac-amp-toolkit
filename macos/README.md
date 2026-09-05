# macOS‑native path — no OrbStack

The plan is [`docs/MACOS-NATIVE.md`](../docs/MACOS-NATIVE.md); this is the code.

> [!WARNING]
> **Untested against hardware.** Everything here builds and self‑checks on a Mac with no dongle
> attached. Nothing has sent a real HID report or opened a real bootloader.
> [`orbstack/`](../orbstack/) remains the known‑good path until this is proven.

## Why

`orbstack/ktflash-orbstack.sh flash` creates an Ubuntu guest, installs build deps, passes the
dongle through, builds `ktflash` inside the guest, unlocks, re‑attaches the device because it
re‑enumerated under a new USB ID, and flashes over libusb. Every one of those steps exists to
work around a single fact: macOS will not let libusb claim the dongle's HID interface.

Two findings make a native path plausible:

1. **The bootloader is a serial device.** [`docs/CDC-PROTOCOL.md`](../docs/CDC-PROTOCOL.md)
   describes `8888:cdc0` as USB CDC, bulk `0x03`/`0x83`, appearing as a tty. On macOS that is
   `/dev/cu.usbmodem*`, which is world‑writable. So the *flash* needs no interface claim at all —
   `ktflash --transport serial` just opens the port.
2. **Only `unlock` is genuinely blocked**, and even that is an open question rather than a known
   impossibility — see `MACOS-NATIVE.md` §4, and the contradiction between `PROTOCOL.md` and
   `dongle-investigation.md` that experiment E1 exists to settle.

## Layout

```
macos/
└── native/
    ├── ktmac/                  Swift package (SwiftPM, dependency-free)
    │   ├── Sources/KTMacKit/     USB enumeration, serial→USB resolution, HID unlock,
    │   │                         the orchestrated flow, self-tests
    │   └── Sources/ktmac/        the CLI
    ├── e1_hid_setreport.c      E1/E2 in C — a minimal probe, superseded by `ktmac unlock`
    ├── e3_interface_seize.c    E3 — USBInterfaceOpenSeize + WritePipe
    └── Makefile                builds the two C probes
```

E3 stayed in C on purpose: the `IOUSBLib` plug‑in dance (`IOCreatePlugInInterfaceForService` +
`QueryInterface` into a C function table) is considerably worse from Swift, and the C version
already compiled cleanly. Everything else is Swift.

## Build

```sh
cd macos/native/ktmac && swift build
.build/debug/ktmac selftest        # 20 hardware-free checks
```

Needs only the Xcode **Command Line Tools**. There is no test target because neither XCTest nor
swift‑testing ships with CLT‑only — the checks are `ktmac selftest`, which is why they actually
get run. See `Sources/KTMacKit/SelfTest.swift`.

For the C probes: `cd macos/native && make`.

## Use

```sh
ktmac doctor                      # is this Mac ready? reports dry-run and execute gates separately
ktmac list                        # KTMicro / FiiO / bootloader devices
ktmac ports                       # serial ports with their resolved USB VID:PID
ktmac port                        # just the bootloader's /dev/cu.* path (for scripts)
ktmac unlock                      # dry run: find the device, send nothing
ktmac unlock --send               # attempt the unlock — RECOVERABLE, power-cycle undoes it
ktmac watch                       # wait for 8888:cdc0 to appear
```

### The whole flow

```sh
ktmac flow --image fw.bin                              # dry run — nothing is written
ktmac flow --image fw.bin --execute --expect 2972:0102 # unlock + write + confirm
```

`flow` runs: preflight → dry run → typed `FLASH` confirmation → native unlock → resolve the
bootloader's `/dev/cu.*` → `ktflash flash-cdc --transport serial --port … --execute --yes` →
ktflash's own post‑reset reprobe.

**It shells out to the Rust `ktflash` for the write.** The byte‑exact framing, the operation
journal and the safety gates all live there; a second implementation of the thing that erases
firmware is the last thing this project needs. `flow` prints the exact command it runs.

A dry run needs only `ktflash` on `PATH` — not TCC consent, not an attached dongle — because it
reads a file and prints a packet plan. Gating that on hardware would defeat the point of having
a dry run.

## The wall you will hit first

`IOHIDManagerOpen` returns `0xe00002e2` (`kIOReturnNotPermitted`) on a stock Mac, **before any
USB device is touched**. That is macOS TCC, not USB: since 10.15, opening an `IOHIDManager`
needs **Input Monitoring** consent for the calling application — for a CLI, the terminal.

```
System Settings > Privacy & Security > Input Monitoring > +   (add Terminal/iTerm)
```

Restart the terminal afterwards. Nothing about the dongle can be learned until this is granted,
so a "no devices found" result beforehand means nothing. `ktmac doctor` detects and explains it.

If `ktflash` ever ships an `IOHIDManager`‑based unlock, this consent prompt becomes part of the
macOS first‑run experience and has to be documented. E3 goes via `IOUSBLib` and probably avoids
it, trading a consent prompt for a `sudo` requirement.

## Safety

`unlock` is **recoverable** — it reboots the dongle into its CDC bootloader, and a power‑cycle
returns it to normal mode. It erases nothing.

`flow --execute` is **not**. It erases and rewrites firmware, and `ktflash` cannot read firmware
off a KT02H20 first, so there is no backup. The `KT_USB_BOOT` ROM survives an app‑flash, so a
bad write can be retried — but only with a compatible image already on disk. Save one before you
start.
