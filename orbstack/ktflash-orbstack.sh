#!/usr/bin/env bash
# ktflash-orbstack.sh — drive a KTMicro USB-C DAC dongle from macOS via an OrbStack
# Linux guest, which (unlike macOS) can reach the HID interrupt endpoints and claim
# the USB interface.
#
# WHY OrbStack: macOS's IOHIDFamily/AppleUSBAudio own the device; libusb gets
# LIBUSB_ERROR_ACCESS and IOHIDDeviceSetReport goes down the control pipe the
# firmware ignores. OrbStack `usb attach` hands the physical device to Linux, where
# hidraw/libusb work normally.
#
# WHAT THIS DOES (all read-only-safe unless you pass `unlock`):
#   setup     create/start the Ubuntu guest + install deps
#   attach    pass the dongle (VID 0x31b2) through to the guest
#   probe     build/run `ktflash probe` in the guest
#   diag      build/run `ktflash handshake` in the guest
#   unlock    send the "T12345678" reboot-to-bootloader (DESTRUCTIVE INTENT: puts the
#             device into ISP/bootloader mode; recoverable, but only run deliberately)
#   bootdiag  after unlock, `ktflash bootdiag` — probe the CDC bootloader (8888:cdc0)
#   flash <f> native firmware write of <f> (DESTRUCTIVE): dry-run, then unlock + write.
#             ⚠ BACK UP FIRST — there is NO way to read firmware off the device, and a bad
#             flash can brick it (recoverable only with your original image).
#   detach    return the device to macOS
#   status    show guest + device state
#
# NOTE: the end-to-end firmware *write* (the CDC serial download protocol) is fully reversed
# and implemented in `ktflash flash-cdc` — proven on hardware. Reading firmware back is NOT
# possible in software; see ../docs/CDC-PROTOCOL.md.
set -euo pipefail

MACHINE="${KT_MACHINE:-ktflash}"
VID="0x31b2"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GUEST_REPO="/Users/$(whoami)/…"   # informational; the guest mounts your Mac home at the same path

die(){ echo "error: $*" >&2; exit 1; }
have(){ command -v "$1" >/dev/null 2>&1; }

need_orb(){ have orb || die "OrbStack not found. Install from https://orbstack.dev"; }

macpath(){ # absolute mac path -> same path inside the guest (OrbStack mounts it 1:1)
  echo "$1"
}

cmd_setup(){
  need_orb
  if ! orb list 2>/dev/null | awk '{print $1}' | grep -qx "$MACHINE"; then
    echo ">> creating OrbStack machine '$MACHINE' (ubuntu)…"
    orb create ubuntu "$MACHINE"
  fi
  echo ">> installing guest deps…"
  orb -m "$MACHINE" bash -lc 'sudo DEBIAN_FRONTEND=noninteractive apt-get update -qq && \
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq build-essential libusb-1.0-0-dev pkg-config usbutils >/dev/null && \
    (command -v cargo >/dev/null || (curl -fsSL https://sh.rustup.rs | sh -s -- -y >/dev/null)); echo deps-ok'
  echo ">> done."
}

usb_id(){ orb usb list 2>/dev/null | awk -v v="${VID#0x}" 'tolower($0) ~ v {print $1; exit}'; }

cmd_attach(){
  need_orb
  local id; id="$(usb_id)" || true
  [ -n "${id:-}" ] || die "no KTMicro ($VID) device in 'orb usb list'. Plug the dongle into the Mac."
  echo ">> attaching $id to $MACHINE…"; orb usb attach "$id"
  orb -m "$MACHINE" bash -lc 'lsusb | grep -iE "31b2|8888|KTMicro" || echo "(not visible yet — replug or re-run attach)"'
}

cmd_detach(){ need_orb; local id; id="$(usb_id)"; [ -n "${id:-}" ] && orb usb detach "$id" && echo "detached $id" || echo "nothing attached"; }

# build the Rust `ktflash` in the guest once, then run it (as root) with the given args
guest_ktflash(){
  orb -m "$MACHINE" bash -lc "source ~/.cargo/env 2>/dev/null; cd '$(macpath "$REPO_ROOT")/flasher' && cargo build --release -q && sudo ./target/release/ktflash $*"
}

cmd_probe(){ echo ">> ktflash probe (guest)…"; guest_ktflash probe; }
cmd_diag(){ echo ">> ktflash handshake (guest)…"; guest_ktflash handshake; }

cmd_unlock(){
  echo ">> WARNING: unlock reboots the dongle into its CDC bootloader (8888:cdc0)."
  read -r -p "   type YES to continue: " ok; [ "$ok" = "YES" ] || die "aborted"
  guest_ktflash unlock || true
  echo ">> re-attaching (device re-enumerated as a new VID:PID)…"; sleep 1
  local id; id="$(usb_id)"; [ -n "${id:-}" ] && orb usb detach "$id" 2>/dev/null || true; sleep 1; id="$(usb_id)"; [ -n "${id:-}" ] && orb usb attach "$id" || true
  orb -m "$MACHINE" bash -lc 'lsusb | grep -iE "8888:cdc0" && echo ">> bootloader present" || echo ">> bootloader not seen (power-cycle to recover to normal mode)"'
}

cmd_bootdiag(){ echo ">> ktflash bootdiag (guest)…"; guest_ktflash bootdiag; }

cmd_flash(){
  local img="${1:-}"; [ -n "$img" ] || die "usage: $0 flash <path-to-fw.bin>"
  [ -f "$img" ] || die "no such file: $img"
  local gimg; gimg="$(cd "$(dirname "$img")" && pwd)/$(basename "$img")"   # absolute; guest sees the same path
  echo ">> ⚠ NO firmware backup is possible, and flashing can BRICK the dongle."
  echo ">>   Recovery needs your ORIGINAL image. Proceed entirely at your own risk."
  echo ">> dry-run (packet plan, no hardware write):"
  orb -m "$MACHINE" bash -lc "source ~/.cargo/env 2>/dev/null; cd '$(macpath "$REPO_ROOT")/flasher' && cargo build --release -q && sudo ./target/release/ktflash flash-cdc --image '$gimg'"
  read -r -p "   Have you saved your original firmware? type FLASH to unlock + write: " ok
  [ "$ok" = "FLASH" ] || die "aborted"
  cmd_unlock
  echo ">> writing (flash-cdc --execute --yes)…"
  orb -m "$MACHINE" bash -lc "source ~/.cargo/env 2>/dev/null; cd '$(macpath "$REPO_ROOT")/flasher' && sudo ./target/release/ktflash flash-cdc --image '$gimg' --execute --yes"
}

cmd_status(){ need_orb; echo "== machines =="; orb list; echo "== usb =="; orb usb list; }

case "${1:-}" in
  setup) cmd_setup;;
  attach) cmd_attach;;
  detach) cmd_detach;;
  probe) cmd_probe;;
  diag) cmd_diag;;
  unlock) cmd_unlock;;
  bootdiag) cmd_bootdiag;;
  flash) shift; cmd_flash "${1:-}";;
  status) cmd_status;;
  all) cmd_setup; cmd_attach; cmd_probe;;
  *) grep -E '^#( |$)' "$0" | sed 's/^# \{0,1\}//'; echo; echo "usage: $0 {setup|attach|probe|diag|unlock|bootdiag|flash <fw.bin>|detach|status|all}";;
esac
