# Linux‑native ktflash (roadmap M3)

The whole point of M3 is to drop the macOS→OrbStack detour on Linux: the same Rust binary talks
to the dongle directly via libusb, with a udev rule for non‑root access.

## Build

```sh
sudo apt-get install -y libusb-1.0-0-dev pkg-config   # Debian/Ubuntu
# or: sudo pacman -S libusb pkgconf                    # Arch
cd flasher && cargo build --release
```

## Permissions (udev)

```sh
sudo cp packaging/99-ktflash.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
# replug the dongle
```

This uses `uaccess` (seat‑local access for the logged‑in user). If your setup doesn't honor it,
uncomment the `GROUP="plugdev"` lines in the rules file and `sudo usermod -aG plugdev $USER`
(then log out/in).

## Use

```sh
ktflash probe            # identify the dongle (read‑only)
ktflash fingerprint      # structured JSON identity
ktflash unlock           # reboot into the 8888:cdc0 CDC bootloader
ktflash bootdiag         # prove the bootloader bulk pipe is live
```

The bootloader **download** protocol (the actual firmware write) is fully reversed and
implemented in `ktflash flash-cdc` (byte‑exact framing in
[`../flasher/src/proto/ktcdc.rs`](../flasher/src/proto/ktcdc.rs)) — proven on hardware via
OrbStack. The same binary should work Linux‑native (⏳ untested there). The native transport is
[`../flasher/src/usbtransport.rs`](../flasher/src/usbtransport.rs) (`RusbBootloaderTransport`).
**Reading firmware back is not possible in software** on this chip — see
[`CDC-PROTOCOL.md`](CDC-PROTOCOL.md).

## Safe flashing model (once the codec lands)

```sh
ktflash flash --plan JA11_V2.2.bin --manifest manifest.json   # emits a plan; gate must say PROCEED
ktflash flash --apply <plan.json> --execute                   # journaled, recoverable flash
ktflash recover ktflash-<op>.journal.json                     # if interrupted: the safe next step
```

`--apply` refuses anything the gate didn't clear to PROCEED (reject‑by‑default). Every flash
writes an operation journal so an interrupted write has a defined recovery path — see
[`../flasher/src/proto/journal.rs`](../flasher/src/proto/journal.rs).
