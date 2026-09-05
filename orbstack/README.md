# OrbStack utility — drive the dongle from macOS (fallback path)

**Status (2026‑09‑05): demoted to fallback, not required.** Native macOS now works end‑to‑end
without OrbStack — `ktmac unlock` (a small Swift companion, `IOHIDManager`) plus `ktflash
flash-cdc --transport serial` reflashed a dongle with zero OrbStack involvement, on real
hardware. See [`../docs/MACOS-NATIVE.md`](../docs/MACOS-NATIVE.md). This script stays as the
known‑good, longer‑established route — useful if `ktmac` isn't built yet, or you'd rather not
grant the Input Monitoring consent the native `unlock` needs.

`rusb`/libusb on macOS genuinely can't claim these dongles' HID interrupt endpoints (the kernel
owns the interface) — that part of the original reasoning was correct, and it's still why
`ktflash unlock` itself (not `ktmac`) needs a way around it. [OrbStack](https://orbstack.dev)
fixes it by handing the physical device to a Linux guest, where libusb can detach the kernel
driver and talk to the endpoints — all from your macOS Terminal, no full VM.

## `ktflash-orbstack.sh`

```
./ktflash-orbstack.sh setup     # create the Ubuntu guest + install deps (rust, libusb)
./ktflash-orbstack.sh attach    # pass the plugged-in dongle (VID 0x31B2) to the guest
./ktflash-orbstack.sh probe     # build + run the Rust flasher `probe`
./ktflash-orbstack.sh diag      # build + run the HID probe (via the Rust flasher/handshake)
./ktflash-orbstack.sh unlock    # send T12345678 → reboot into the CDC bootloader (asks first)
./ktflash-orbstack.sh bootdiag  # inspect the 0x8888:0xCDC0 bootloader bulk pipe
./ktflash-orbstack.sh detach    # give the device back to macOS
./ktflash-orbstack.sh status    # machines + usb state
```

Set `KT_MACHINE` to use a different guest name (default `ktflash`).

## Honest scope

This gets you all the way to the bootloader and characterises the device. The end‑to‑end
firmware **write** is fully reversed and proven on hardware — via this OrbStack path (`v1.1.0`),
via Linux‑native, and via the macOS‑native `ktmac`+`ktflash` combination above — see
[`../ROADMAP.md`](../ROADMAP.md) Phases 3–4 for the current state.

## Gotchas

- After `unlock` the device **re‑enumerates** as a new VID:PID, so the script detaches +
  re‑attaches it. If the guest doesn't see it, run `attach` again.
- Flaky USB hubs drop the device mid‑session — use a direct port.
