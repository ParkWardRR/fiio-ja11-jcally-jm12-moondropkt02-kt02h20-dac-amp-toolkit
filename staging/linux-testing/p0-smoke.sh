#!/usr/bin/env bash
# p0-smoke.sh — phase P0 of docs/LINUX-TESTING.md: certify that an artifact RUNS on a distro.
#
# NO DONGLE REQUIRED and nothing destructive: every command here is either read-only or a
# documented dry run. This is the bulk of the test matrix and can be run the day the VMs exist.
#
#   ./p0-smoke.sh                      # test whatever `ktflash` is on PATH
#   ./p0-smoke.sh /path/to/ktflash     # test a specific binary
#   KT_IMAGE=/path/fw.bin ./p0-smoke.sh   # also exercise the image-parsing + dry-run paths
#
# Exit 0 = P0 pass. Any failure prints what to check.
#
# STATUS: UNTESTED (written on macOS; never run on a Linux guest).
set -uo pipefail

KT="${1:-$(command -v ktflash || true)}"
FIXTURE="${KT_FIXTURE:-fixtures/synthetic-success.json}"
pass=0 fail=0 skip=0

ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; pass=$((pass+1)); }
no()   { printf '  \033[31m✗\033[0m %s\n' "$*"; fail=$((fail+1)); }
skip() { printf '  \033[33m–\033[0m %s (skipped: %s)\n' "$1" "$2"; skip=$((skip+1)); }
head_() { printf '\n\033[1m%s\033[0m\n' "$*"; }

[ -n "$KT" ] && [ -x "$KT" ] || { echo "no ktflash binary — pass a path or install it first"; exit 2; }
echo "P0 smoke — $KT"

# --- 1. portability -----------------------------------------------------------------------
# A "static" build that turns out dynamically linked is the failure we want to catch here,
# not on a user's Alma box with a different glibc.
head_ "1. binary shape"
if command -v file >/dev/null; then
  desc="$(file -L "$KT")"
  echo "  $desc"
  case "$desc" in
    *"statically linked"*)  ok "statically linked" ;;
    *"dynamically linked"*) no "DYNAMICALLY linked — the vendored/musl build did not take effect" ;;
    *) skip "linkage" "could not parse \`file\` output" ;;
  esac
else
  skip "linkage" "\`file\` not installed"
fi

if command -v ldd >/dev/null; then
  if ldd "$KT" 2>&1 | grep -q "not a dynamic executable"; then
    ok "ldd: not a dynamic executable"
  else
    echo "  ldd:"; ldd "$KT" 2>&1 | sed 's/^/    /'
    # Not automatically a failure: the glibc fallback build is a legitimate artifact.
    skip "ldd" "dynamic — expected only for the glibc fallback build"
    if command -v objdump >/dev/null; then
      echo "  glibc symbol floor (must be <= the OLDEST supported distro, Alma 9 = 2.34):"
      objdump -T "$KT" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -5 | sed 's/^/    /'
    fi
  fi
fi

# --- 2. packaging -------------------------------------------------------------------------
head_ "2. packaging"
rules="$(ls /etc/udev/rules.d/99-ktflash.rules /usr/lib/udev/rules.d/99-ktflash.rules 2>/dev/null | head -n1)"
if [ -n "$rules" ]; then
  ok "udev rules installed at $rules"
  # The ModemManager rules are new (RELEASE-PLAN §5.3); confirm the shipped file has them.
  if grep -q ID_MM_DEVICE_IGNORE "$rules"; then
    ok "rules include the ModemManager ignore entries"
  else
    no "rules are the OLD version — no ID_MM_DEVICE_IGNORE entries"
  fi
else
  skip "udev rules" "not installed (expected if testing a bare tarball)"
fi

# --- 3. functional, no hardware -----------------------------------------------------------
head_ "3. hardware-free commands"
run() { # run <label> <expect-success:0|1> <cmd...>
  local label="$1" want="$2"; shift 2
  local out; out="$("$@" 2>&1)"; local rc=$?
  if [ "$rc" -eq 0 ] && [ "$want" -eq 0 ]; then ok "$label"
  elif [ "$rc" -ne 0 ] && [ "$want" -ne 0 ]; then ok "$label (correctly refused)"
  else
    no "$label (exit $rc)"; echo "$out" | head -5 | sed 's/^/      /'
  fi
}

run "--help" 0 "$KT" --help
run "compat --template" 0 "$KT" compat --template

# The replay fixture is the closest thing to an end-to-end test without a dongle.
if [ -f "$FIXTURE" ]; then
  run "bootdiag --replay" 0 "$KT" bootdiag --replay "$FIXTURE"
else
  skip "bootdiag --replay" "fixture not found at $FIXTURE (set KT_FIXTURE, or run from flasher/)"
fi

# compat --validate round-trip: the template must satisfy the validator.
tmp="$(mktemp)"; trap 'rm -f "$tmp"' EXIT
if "$KT" compat --template > "$tmp" 2>/dev/null; then
  run "compat --validate (round-trip)" 0 "$KT" compat --validate "$tmp"
fi

if [ -n "${KT_IMAGE:-}" ] && [ -f "$KT_IMAGE" ]; then
  run "image <fw.bin>" 0 "$KT" image "$KT_IMAGE"
  # Dry run: builds and prints the packet plan, touches no hardware. Must NOT need a device.
  run "flash-cdc dry run" 0 "$KT" flash-cdc --image "$KT_IMAGE"
  # And the safety gate must still refuse a write without --yes, even with no device present.
  run "flash-cdc --execute without --yes" 1 "$KT" flash-cdc --image "$KT_IMAGE" --execute
else
  skip "image / flash-cdc dry run" "set KT_IMAGE=/path/fw.bin to include these"
fi

# --- 4. TUI -------------------------------------------------------------------------------
head_ "4. TUI"
# The TUI needs a tty and must restore the terminal on exit; a panic here corrupts the user's
# shell, which is why it is worth checking on every distro rather than assuming.
if [ -t 1 ]; then
  echo "  MANUAL: run \`$KT\`, confirm it draws and quits cleanly with 'q',"
  echo "          and that your terminal is not left in raw mode afterwards."
  skip "TUI render" "manual step"
else
  skip "TUI render" "not a tty"
fi

# --- summary ------------------------------------------------------------------------------
printf '\n\033[1mP0: %d passed, %d failed, %d skipped\033[0m\n' "$pass" "$fail" "$skip"
[ "$fail" -eq 0 ] || { echo "P0 FAILED — do not proceed to P1 until these are resolved."; exit 1; }
echo "P0 pass. Next: P1 (read-only USB, needs the dongle) — docs/LINUX-TESTING.md §3."
