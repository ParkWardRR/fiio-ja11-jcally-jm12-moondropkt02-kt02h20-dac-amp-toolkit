#!/usr/bin/env bash
# p2-usb-checks.sh — the observations to make around phases P1/P2 of docs/LINUX-TESTING.md.
#
# This script does NOT run `ktflash unlock` for you — unlock changes device state, so it stays a
# deliberate manual step. What it does is capture the before/after evidence around it, and
# answer the open question from docs/RELEASE-PLAN.md §5.3: does ModemManager actually probe the
# bootloader on this distro?
#
#   ./p2-usb-checks.sh before     # dongle attached, normal mode
#   ktflash unlock                # <- you run this
#   ./p2-usb-checks.sh after      # bootloader present
#
# Writes p2-<stage>-<host>.txt for attaching to the results table.
#
# STATUS: UNTESTED (written on macOS; never run on a Linux guest).
set -uo pipefail

STAGE="${1:-before}"
OUT="p2-${STAGE}-$(hostname).txt"
q() { command -v "$1" >/dev/null 2>&1; }

{
echo "== P2 $STAGE — $(hostname) — $(date -u +%Y-%m-%dT%H:%M:%SZ) =="
echo

echo "-- lsusb --"
q lsusb && lsusb || echo "(usbutils missing)"
echo
echo "-- lsusb -t (topology; use this to pick the hypervisor passthrough PORT) --"
# Passthrough must be bound to the bus/port, NOT to VID:PID: `unlock` re-enumerates the device
# under a new ID, and an ID-keyed rule drops it exactly when it matters
# (docs/LINUX-TESTING.md §2.1).
q lsusb && lsusb -t || true
echo

echo "-- our devices --"
q lsusb && lsusb | grep -iE '31b2|2972|8888' || echo "(none matched)"
echo

echo "-- CDC-ACM ttys --"
ls -l /dev/ttyACM* 2>/dev/null || echo "(none — expected BEFORE unlock)"
echo

if ls /dev/ttyACM* >/dev/null 2>&1; then
  for t in /dev/ttyACM*; do
    echo "-- udev properties: $t --"
    q udevadm && udevadm info -q property -n "$t" | grep -iE 'ID_VENDOR_ID|ID_MODEL_ID|ID_MM|ID_USB' || true
    # This is the question: is ID_MM_DEVICE_IGNORE set? If not, the new udev rules either are
    # not installed or did not match.
    if q udevadm && udevadm info -q property -n "$t" | grep -q 'ID_MM_DEVICE_IGNORE=1'; then
      echo "  => ID_MM_DEVICE_IGNORE is SET (ModemManager will leave this alone)"
    else
      echo "  => ID_MM_DEVICE_IGNORE NOT set (ModemManager may probe this tty mid-flash)"
    fi
    echo
  done
fi

echo "-- ModemManager --"
if q systemctl && systemctl is-active --quiet ModemManager; then
  echo "ACTIVE — this is the actor that can race flash-cdc for the bootloader tty."
  echo
  echo "recent journal (did it probe our device?):"
  q journalctl && journalctl -u ModemManager --since '5 min ago' --no-pager 2>/dev/null | tail -30 \
    || echo "(journalctl unavailable)"
else
  echo "not active — the ModemManager udev rules are defensive only on this host."
fi
echo

echo "-- kernel messages --"
q dmesg && (dmesg 2>/dev/null | tail -30 || sudo dmesg | tail -30) || echo "(dmesg unavailable)"
echo

echo "-- driver bindings --"
for d in /sys/bus/usb/devices/*/; do
  v=$(cat "$d/idVendor" 2>/dev/null) || continue
  case "$v" in
    31b2|2972|8888)
      p=$(cat "$d/idProduct" 2>/dev/null)
      echo "$d ($v:$p)"
      for i in "$d"*:*/; do
        [ -d "$i" ] || continue
        drv=$(basename "$(readlink -f "$i/driver" 2>/dev/null)" 2>/dev/null)
        echo "  $(basename "$i") -> driver=${drv:-none}"
      done
      ;;
  esac
done
} | tee "$OUT"

echo
echo "wrote $OUT"
case "$STAGE" in
  before)
    echo
    echo "Next: run \`ktflash probe\` as a NORMAL USER (no sudo)."
    echo "  If it needs sudo, the udev rules or the logind seat is the problem — record which."
    echo "Then \`ktflash unlock\`, then: ./p2-usb-checks.sh after"
    echo
    echo "Reminder: unlock is recoverable. Unplug/replug returns the dongle to normal mode."
    ;;
  after)
    echo
    echo "Confirm above that 8888:cdc0 is present. If it is NOT, and unlock reported success,"
    echo "the hypervisor dropped the device on re-enumeration — fix the passthrough to bind the"
    echo "PORT rather than the VID:PID (docs/LINUX-TESTING.md §2.1) before going any further."
    echo
    echo "Then: ktflash bootdiag        (libusb path)"
    echo "      ktflash bootdiag --transport serial   (tty path, if the serial work has landed)"
    ;;
esac
