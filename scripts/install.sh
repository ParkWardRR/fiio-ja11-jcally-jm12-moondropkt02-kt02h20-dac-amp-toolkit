#!/usr/bin/env sh
# install.sh — download, VERIFY, and install ktflash.
#
#   curl -fsSL https://raw.githubusercontent.com/ParkWardRR/fiio-ja11-jcally-jm12-moondropkt02-kt02h20-dac-amp-toolkit/main/scripts/install.sh | sh
#
# On macOS this is the recommended path precisely because it uses curl: Gatekeeper quarantine is
# applied by the *downloader*, and curl does not set com.apple.quarantine. A browser download
# does, which is why that path needs `xattr -d` (docs/RELEASE-PLAN.md §4.3).
#
# Options (env):
#   KT_VERSION=1.2.0     pin a version instead of "latest"
#   KT_PREFIX=~/.local/bin   install location (default)
#   KT_SKIP_VERIFY=1     ⚠ skip signature/checksum verification — NOT recommended
#
# What it will NEVER do: run ktflash, flash anything, or install without verifying first.
#
# STATUS: UNTESTED. POSIX sh, shellcheck-clean, but never run against a real release.
set -eu

REPO="ParkWardRR/fiio-ja11-jcally-jm12-moondropkt02-kt02h20-dac-amp-toolkit"
PREFIX="${KT_PREFIX:-$HOME/.local/bin}"
# Committed at packaging/ktflash.pub. Generated 2026-09-05; secret key held offline by the
# maintainer (never in this repo). Rotate by regenerating and updating both this line and the
# committed .pub file in the same change.
MINISIGN_PUBKEY="${KT_PUBKEY:-RWRBoKW7XVEQLaslAJsw+ehM+1AGz90YMA+P7GzCzNZV54aVWtS7PC6n}"

say()  { printf '%s\n' "$*"; }
warn() { printf '\033[1;33m%s\033[0m\n' "$*" >&2; }
die()  { printf '\033[1;31minstall: %s\033[0m\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# ---------------------------------------------------------------- platform
detect_target() {
  os="$(uname -s)"; arch="$(uname -m)"
  case "$os" in
    Darwin) echo "universal-apple-darwin" ;;   # one universal binary covers both Macs
    Linux)
      case "$arch" in
        x86_64|amd64)  echo "x86_64-unknown-linux-musl" ;;
        aarch64|arm64) echo "aarch64-unknown-linux-musl" ;;
        *) die "unsupported Linux architecture: $arch" ;;
      esac ;;
    *) die "unsupported OS: $os (Windows is not a supported path — see the README)" ;;
  esac
}

TARGET="$(detect_target)"
have curl || die "curl is required"
have tar  || die "tar is required"

# ---------------------------------------------------------------- version
if [ -n "${KT_VERSION:-}" ]; then
  VERSION="$KT_VERSION"
else
  VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -n1)"
  [ -n "$VERSION" ] || die "could not determine the latest version — set KT_VERSION"
fi

BASE="https://github.com/$REPO/releases/download/v$VERSION"
TARBALL="ktflash-$VERSION-$TARGET.tar.gz"
TMP="$(mktemp -d)"
# shellcheck disable=SC2064  # expand TMP now, on purpose
trap "rm -rf '$TMP'" EXIT INT TERM

say "ktflash $VERSION ($TARGET)"
say "  downloading…"
curl -fsSL -o "$TMP/$TARBALL"   "$BASE/$TARBALL"   || die "download failed: $BASE/$TARBALL"
curl -fsSL -o "$TMP/SHA256SUMS" "$BASE/SHA256SUMS" || die "download failed: SHA256SUMS"

# ---------------------------------------------------------------- verify
# Verification happens BEFORE anything is extracted or made executable. This tool erases
# hardware; a tampered binary is a bricked device.
if [ "${KT_SKIP_VERIFY:-0}" = "1" ]; then
  warn "⚠  KT_SKIP_VERIFY=1 — installing WITHOUT verification. You are trusting the network."
else
  say "  verifying checksum…"
  if have shasum; then SUM="shasum -a 256"; elif have sha256sum; then SUM="sha256sum"; else
    die "need shasum or sha256sum to verify (or set KT_SKIP_VERIFY=1 and accept the risk)"
  fi
  expected="$(grep " $TARBALL\$" "$TMP/SHA256SUMS" | awk '{print $1}')"
  [ -n "$expected" ] || die "$TARBALL is not listed in SHA256SUMS"
  actual="$(cd "$TMP" && $SUM "$TARBALL" | awk '{print $1}')"
  [ "$expected" = "$actual" ] || die "CHECKSUM MISMATCH — do not use this download.
  expected $expected
  actual   $actual"
  say "  checksum ok"

  if have minisign; then
    say "  verifying signature…"
    curl -fsSL -o "$TMP/SHA256SUMS.minisig" "$BASE/SHA256SUMS.minisig" \
      || die "no signature published for v$VERSION"
    minisign -Vm "$TMP/SHA256SUMS" -P "$MINISIGN_PUBKEY" >/dev/null \
      || die "SIGNATURE VERIFICATION FAILED — do not use this download"
    say "  signature ok"
  else
    warn "  minisign not installed — checksum verified, but its signature was NOT."
    warn "  The checksum file itself could have been substituted. Install minisign for a real"
    warn "  guarantee:  brew install minisign  /  dnf install minisign  /  apt install minisign"
  fi
fi

# ---------------------------------------------------------------- install
say "  installing to $PREFIX…"
tar -xzf "$TMP/$TARBALL" -C "$TMP"
SRC="$(find "$TMP" -type f -name ktflash -perm -u+x | head -n1)"
[ -n "$SRC" ] || die "no ktflash binary inside $TARBALL"
mkdir -p "$PREFIX"
install -m 0755 "$SRC" "$PREFIX/ktflash" 2>/dev/null || {
  cp "$SRC" "$PREFIX/ktflash" && chmod 0755 "$PREFIX/ktflash"
}
say "  installed $PREFIX/ktflash"

case ":$PATH:" in
  *":$PREFIX:"*) ;;
  *) warn "  $PREFIX is not on your PATH — add: export PATH=\"$PREFIX:\$PATH\"" ;;
esac

# ---------------------------------------------------------------- udev (Linux only, opt-in)
# Without these rules, claiming the device needs root, and the failure mode
# (LIBUSB_ERROR_ACCESS) looks like a broken tool rather than a permissions problem.
if [ "$(uname -s)" = "Linux" ]; then
  RULES="$(find "$TMP" -name '99-ktflash.rules' | head -n1)"
  if [ -n "$RULES" ] && [ ! -f /etc/udev/rules.d/99-ktflash.rules ] \
     && [ ! -f /usr/lib/udev/rules.d/99-ktflash.rules ]; then
    say ""
    say "  udev rules let you run ktflash without sudo. Install them? (needs sudo)"
    if [ -t 0 ] && printf '    [y/N] ' && read -r reply && [ "$reply" = y ]; then
      sudo cp "$RULES" /etc/udev/rules.d/99-ktflash.rules
      sudo udevadm control --reload-rules && sudo udevadm trigger
      say "  installed — replug the dongle."
    else
      say "  skipped. Install later with:"
      say "    sudo cp 99-ktflash.rules /etc/udev/rules.d/ && sudo udevadm control --reload-rules"
    fi
  fi
fi

cat <<'EOF'

  Done. Start with a read-only look at your dongle:

    ktflash            # live TUI
    ktflash probe      # one-shot report

  ⚠  FLASHING CAN BRICK YOUR DONGLE, AND THERE IS NO WAY TO BACK UP ITS FIRMWARE FIRST.
     ktflash cannot read firmware off a KT02H20. If a cross-flash goes wrong, recovery
     depends on you already having a compatible image. Save a known-good image before you
     start. See docs/FLASHING.md and docs/COMPATIBILITY.md.

EOF

if [ "$(uname -s)" = "Darwin" ]; then
  cat <<'EOF'
  macOS note: this build is signed ad-hoc, NOT with an Apple Developer ID. Installing via
  curl (as you just did) means no Gatekeeper quarantine was applied, so it runs as-is. If you
  ever download it with a browser instead, clear the quarantine flag:

      xattr -d com.apple.quarantine ./ktflash

  Do NOT disable Gatekeeper system-wide.

EOF
fi
