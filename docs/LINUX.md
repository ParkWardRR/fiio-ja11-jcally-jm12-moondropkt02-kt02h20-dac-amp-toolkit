# Linux‑native ktflash

`ktflash` talks to the dongle directly via libusb (or the bootloader's CDC‑ACM serial port) —
no VM, no container, no macOS/OrbStack detour needed on Linux at all. **Proven on hardware**: a
full `flash-cdc --execute` write completed end‑to‑end on a Debian 13 VM, no OrbStack involved
(`ROADMAP.md` Appendix D).

## Install

**Packages (recommended)** — installs the binary and the udev rules together:

```sh
# Debian / Ubuntu
sudo apt install ./ktflash_<ver>_amd64.deb
# AlmaLinux / RHEL / Fedora / Rocky
sudo dnf install ./ktflash-<ver>.x86_64.rpm
```

**Static tarball** (any distro, no package manager needed):

```sh
tar xzf ktflash-<ver>-x86_64-unknown-linux-musl.tar.gz
sudo cp 99-ktflash.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger   # or replug the dongle
```

**From source:**

```sh
sudo apt-get install -y build-essential libusb-1.0-0-dev pkg-config   # Debian/Ubuntu
# or: sudo dnf install -y gcc libusb1-devel pkgconf-pkg-config          # Alma/RHEL/Fedora
# or: sudo pacman -S base-devel libusb pkgconf                          # Arch
cd flasher && cargo build --release
sudo cp ../packaging/99-ktflash.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```

## Permissions (udev)

[`packaging/99-ktflash.rules`](../packaging/99-ktflash.rules) grants access two ways at once:
`uaccess` (systemd‑logind seat‑local access — the modern desktop mechanism) and
`GROUP="plugdev"` (needed on headless machines, which have no logind seat — confirmed by testing
on a real headless VM, where `uaccess` alone did nothing). Both ship uncommented by default. If
`plugdev` doesn't apply automatically: `sudo usermod -aG plugdev $USER`, then log out/in.

**ModemManager**: after `unlock`, the bootloader exposes a CDC‑ACM tty (`/dev/ttyACM*`).
`packaging/99-ktflash.rules` includes `ENV{ID_MM_DEVICE_IGNORE}="1"` rules so ModemManager
doesn't probe it as a potential modem mid‑flash — defensive by default; not yet confirmed
whether ModemManager actually contends for this specific device (`docs/RELEASE-PLAN.md` §5.3).

## Use

```sh
ktflash probe                 # identify the dongle (read-only)
ktflash fingerprint           # structured JSON identity
ktflash unlock                # reboot into the 8888:cdc0 CDC bootloader
ktflash bootdiag               # non-advancing pipe check; --send for a KTM liveness check
ktflash flash-cdc --image fw.bin                          # dry run — prints the plan, writes nothing
ktflash flash-cdc --image fw.bin --execute --yes          # the real write
```

`flash-cdc` auto-selects a transport (`--transport auto|serial|usb`): the bootloader's CDC‑ACM
tty by default, or raw libusb bulk endpoints with `--transport usb`. Both are proven on real
Linux hardware. **Reading firmware back is not possible in software** on this chip — see
[`CDC-PROTOCOL.md`](CDC-PROTOCOL.md).

## Safety model

Every `flash-cdc --execute` writes an operation journal *before* the destructive command
(`KSTA`, which triggers the erase) and does a post-reset reprobe to confirm success:

```sh
ktflash flash-cdc --image fw.bin --expect 2972:0102 --execute --yes   # --expect enables the reprobe check
ktflash recover ktflash-<op>.journal.json                             # if interrupted: the safe next step
```

See [`../flasher/src/proto/journal.rs`](../flasher/src/proto/journal.rs) and
[`../flasher/src/proto/postflash.rs`](../flasher/src/proto/postflash.rs).
