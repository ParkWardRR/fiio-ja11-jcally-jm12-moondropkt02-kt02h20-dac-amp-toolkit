# OrbStack utility — drive the dongle from macOS

macOS can't reach these dongles' HID interrupt endpoints (the kernel owns the interface).
[OrbStack](https://orbstack.dev) fixes that: `orb usb attach` hands the physical device to a
Linux guest, where libusb can detach the kernel driver and talk to the endpoints — all from
your macOS Terminal, no full VM.

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

This gets you **all the way to the bootloader** and characterises the device — which is the
hard, macOS‑blocked part. The end‑to‑end firmware **write** isn't reversed yet (the CDC
download protocol; see [`../docs/EXTRACTION.md`](../docs/EXTRACTION.md) §5). PRs welcome.

## Gotchas

- After `unlock` the device **re‑enumerates** as a new VID:PID, so the script detaches +
  re‑attaches it. If the guest doesn't see it, run `attach` again.
- Flaky USB hubs drop the device mid‑session — use a direct port.
