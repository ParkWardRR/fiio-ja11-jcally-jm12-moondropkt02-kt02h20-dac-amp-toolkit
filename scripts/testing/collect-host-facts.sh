#!/usr/bin/env bash
# collect-host-facts.sh — record what a test guest actually is, before testing anything.
#
# Run once per VM. Output is a markdown block to paste into the results table
# (docs/LINUX-TESTING.md §4). Read-only; touches no hardware.
#
#   ./collect-host-facts.sh > facts-debian13.md
#
# STATUS: UNTESTED (written on macOS; never run on a Linux guest).
set -uo pipefail   # deliberately NOT -e: a missing tool is a finding, not a failure

q() { command -v "$1" >/dev/null 2>&1; }
val() { "$@" 2>/dev/null | head -n1 || echo "(n/a)"; }

echo "## Host facts — $(hostname) — $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo
echo '| fact | value |'
echo '|---|---|'

# Distro identity decides which artifact (.deb vs .rpm) and which quirks apply.
if [ -r /etc/os-release ]; then
  # shellcheck disable=SC1091
  . /etc/os-release
  echo "| distro | ${PRETTY_NAME:-$NAME $VERSION_ID} |"
  echo "| id | ${ID:-?} ${VERSION_ID:-?} |"
else
  echo "| distro | (no /etc/os-release) |"
fi

echo "| arch | $(uname -m) |"
echo "| kernel | $(uname -r) |"

# glibc floor matters only for the fallback (non-musl) build.
echo "| glibc | $(val ldd --version | sed 's/.*) //') |"

echo "| udev | $(val udevadm --version) |"

# ModemManager is the actor that races flash-cdc for the bootloader tty
# (docs/RELEASE-PLAN.md §5.3). Its presence decides whether the new udev rules matter.
if q systemctl; then
  echo "| ModemManager | $(systemctl is-active ModemManager 2>/dev/null || echo absent) |"
  echo "| logind | $(systemctl is-active systemd-logind 2>/dev/null || echo absent) |"
else
  echo "| ModemManager | (no systemctl) |"
fi

# SELinux is the Alma/RHEL-specific gate.
if q getenforce; then
  echo "| SELinux | $(getenforce) |"
else
  echo "| SELinux | absent |"
fi

# Headless guests have no logind seat, so udev `uaccess` may not grant access — that is the
# case where the plugdev fallback in 99-ktflash.rules matters. Test at least one of each.
seat="$(loginctl show-session "$(loginctl 2>/dev/null | awk 'NR==2{print $1}')" -p Seat --value 2>/dev/null)"
echo "| logind seat | ${seat:-none (headless?)} |"
echo "| session type | ${XDG_SESSION_TYPE:-unknown} |"

echo "| ktflash rules | $(ls /etc/udev/rules.d/99-ktflash.rules /usr/lib/udev/rules.d/99-ktflash.rules 2>/dev/null | tr '\n' ' ' || echo 'not installed') |"

if q ktflash; then
  echo "| ktflash | $(command -v ktflash) |"
  echo "| linkage | $(val file -L "$(command -v ktflash)" | sed 's/.*: //' | cut -c1-80) |"
else
  echo "| ktflash | not installed |"
fi

echo
echo "### USB devices"
echo '```'
if q lsusb; then
  lsusb | grep -iE '31b2|2972|8888|ktmicro' || echo "(no KTMicro/FiiO/bootloader device attached)"
else
  echo "usbutils not installed (apt install usbutils / dnf install usbutils)"
fi
echo '```'
