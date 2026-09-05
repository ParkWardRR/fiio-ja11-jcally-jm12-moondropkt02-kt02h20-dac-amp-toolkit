# Flashing — macOS + OrbStack

> [!CAUTION]
> **BACK UP YOUR FIRMWARE BEFORE YOU FLASH.** `ktflash` **cannot read firmware off the
> device yet** (the `0x08` read path does not reach flash — confirmed on hardware), so there
> is **no automatic backup**. If a cross‑flash goes wrong, your **only** way back is your
> device's **original manufacturer firmware image** plus a working reflash. Obtain and save
> that image *now* (the vendor's updater package, or a copy from someone with the same
> dongle). `ktflash flash-cdc` refuses to write without `--yes` for exactly this reason.

> [!WARNING]
> Flashing **overwrites the dongle's firmware**. It's recoverable via the bootloader, but
> only with a matching image (see [`REVERT.md`](REVERT.md)). Same chip ≠ guaranteed
> compatible — read [`COMPATIBILITY.md`](COMPATIBILITY.md).

## Native write (`ktflash flash-cdc`) — proven on hardware 2026‑09‑05

```bash
# 0. BACK UP first (see the CAUTION above — there is no read-back).
orb usb attach <id>                       # from `orb usb list`
ktflash flash-cdc --image fw.bin          # DRY RUN — prints the packet plan, touches nothing
ktflash unlock                            # → fresh CDC bootloader (8888:cdc0)
orb usb attach <id>                       # re-attach: it re-enumerated
ktflash flash-cdc --image fw.bin --execute --yes   # writes; --yes = "I have a backup"
```
`flag` (write base / handshake path) is auto‑derived from image byte `0x0F`; override with
`--flag`. A same‑image reflash is non‑destructive; a wrong/mismatched image can leave the
dongle bootloader‑only until you reflash a correct one.

This project is **macOS‑first**, using **OrbStack** to reach the USB endpoints macOS itself
won't. There is intentionally **no Windows path here** — if you just want to flash today
without any of this, FiiO ships their own official Windows updater. This toolkit is about
doing it natively from a Mac (and, later, Linux).

## If you need OrbStack (and when you don't)

**Corrected 2026‑09‑05 — you may not need OrbStack at all.** The original reasoning here was
that macOS's `IOHIDFamily`/`AppleUSBAudio` drivers own the dongle's interface — libusb returns
`LIBUSB_ERROR_ACCESS`, and (this part turned out to be wrong) `IOHIDDeviceSetReport` supposedly
went down a control pipe the firmware ignores. Confirmed on real hardware: `IOHIDDeviceSetReport`
via `IOHIDManager` **does** reach the device natively, no interface claim needed — see
[`docs/MACOS-NATIVE.md`](MACOS-NATIVE.md). `ktflash unlock` itself now auto-detects the `ktmac`
companion tool (`macos/native/ktmac`) and shells out to it, then `ktflash flash-cdc --transport
serial` does the write — no OrbStack, no VM, over the bootloader's CDC‑ACM tty, one command
either way.

**You still need OrbStack if:** `ktmac` isn't built/available yet on your machine, you'd rather
not grant Input Monitoring consent (native `unlock` requires it — macOS TCC, since 10.15), or
you're troubleshooting and want the known‑good, longer‑established path. [OrbStack](https://orbstack.dev)
hands the physical device to a lightweight Linux guest with `orb usb attach`, where libusb can
detach the kernel driver and use the real interrupt/bulk endpoints — all from your macOS
Terminal, no full VM. It remains a fully supported fallback, not a dead end.

## What works today

| Step | Native macOS | macOS + OrbStack |
|---|---|---|
| **Identify** (`ktflash` TUI / `probe`) | ✅ | ✅ |
| **handshake** the HID control channel | ❌ (claim blocked) | ✅ |
| **unlock** → reboot into the bootloader | ❌ | ✅ (recoverable) |
| **bootdiag** the CDC bootloader pipe | ❌ | ✅ |
| **write** new firmware end‑to‑end (`flash-cdc`) | ❌ | ✅ proven on hardware |

The CDC bootloader's **download** protocol is fully reversed and implemented in
`ktflash flash-cdc` (byte‑exact framing in `proto::ktcdc`) — proven by a complete reflash of
the stock `JA11_V2.2.bin` on 2026‑09‑05. **Reading firmware back is not possible in software**
(no read command exists on this chip — see [`CDC-PROTOCOL.md`](CDC-PROTOCOL.md)), so keep your
original image.

## Do it

```bash
# 0. identify what you have — pretty, read-only, native macOS
ktflash                        # live TUI dashboard
ktflash probe                  # one-shot report

# 1. set up the OrbStack Linux guest + pass the dongle through
./orbstack/ktflash-orbstack.sh setup
./orbstack/ktflash-orbstack.sh attach

# 2. talk to it from the guest (reaches the real endpoints)
./orbstack/ktflash-orbstack.sh probe
./orbstack/ktflash-orbstack.sh handshake
./orbstack/ktflash-orbstack.sh unlock     # → reboots into CDC bootloader (recoverable)
./orbstack/ktflash-orbstack.sh bootdiag   # inspect the bootloader pipe

# 3. write (in the guest) — BACK UP YOUR FIRMWARE FIRST (there is no read-back):
ktflash flash-cdc --image fw.bin                   # dry run: prints the packet plan
ktflash unlock                                     # fresh bootloader
ktflash flash-cdc --image fw.bin --execute --yes   # writes; --yes = "I have a backup"
```

After `unlock` the dongle **re‑enumerates** as a new USB device (`0x8888:0xCDC0`); the
script re‑attaches it to the guest automatically.

## Gotchas

- **Flaky USB dropouts** on hubs/adapters — the dongle disappears mid‑session. Use a direct
  port. OrbStack's `usb attach` also needs a re‑attach after the unlock re‑enumeration.
- **Already on JA11?** `probe`/the TUI will show `2972:0102 "JadeAudio JA11"`. `unlock` still
  works from there (JA11's own firmware honours the same reboot‑to‑ISP), so you can reflash
  or revert.

## Roadmap

1. **macOS + OrbStack** — identify, handshake, unlock, bootdiag, **native write** ✅
2. **Linux‑native** — same Rust binary, no OrbStack layer (⏳ testing + prebuilt binaries).
