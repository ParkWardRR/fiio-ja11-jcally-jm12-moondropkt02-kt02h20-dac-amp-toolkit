#!/usr/bin/env bash
# release.sh — build, checksum and sign every ktflash artifact. One command, no CI service,
# no containers (docs/RELEASE-PLAN.md §3).
#
#   ./scripts/release.sh            # full matrix
#   ./scripts/release.sh --dry-run  # gates + plan only, build nothing
#   KT_SKIP_LINUX=1 ./scripts/release.sh    # macOS only (no zig installed yet)
#
# Output lands in dist/<version>/:
#   ktflash-<version>-<target>.tar.gz   one per target
#   SHA256SUMS  SHA256SUMS.minisig      checksums + detached signature
#   ktflash-<version>.cdx.json          SBOM (if cargo-cyclonedx is installed)
#   BUILD-INFO.txt                      toolchain + host, for reproducibility
#
# STATUS: UNTESTED. Written from docs/RELEASE-PLAN.md; never executed end to end. The Linux
# cross-build in particular depends on cargo-zigbuild handling the vendored libusb C sources,
# which is the one unproven link (RELEASE-PLAN §3.2).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE_DIR="$REPO_ROOT/flasher"
DRY_RUN=0
[ "${1:-}" = "--dry-run" ] && DRY_RUN=1

MAC_TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
LINUX_TARGETS=(x86_64-unknown-linux-musl aarch64-unknown-linux-musl)

die() { echo "release: $*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }
step() { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }

# ---------------------------------------------------------------- gates
# Unlike ci.sh, the supply-chain tools are NOT optional here: this is the build that other
# people run against their own hardware.
step "gates"

[ -d "$CRATE_DIR" ] || die "no flasher/ — run from the repo"
git -C "$REPO_ROOT" diff --quiet && git -C "$REPO_ROOT" diff --cached --quiet \
  || die "working tree is dirty — commit or stash first"

VERSION="$(grep -m1 '^version' "$CRATE_DIR/Cargo.toml" | cut -d'"' -f2)"
[ -n "$VERSION" ] || die "could not read version from Cargo.toml"
echo "version: $VERSION"

# If HEAD is tagged, the tag must agree with Cargo.toml.
if TAG="$(git -C "$REPO_ROOT" describe --exact-match --tags HEAD 2>/dev/null)"; then
  [ "${TAG#v}" = "$VERSION" ] || die "tag $TAG disagrees with Cargo.toml version $VERSION"
  echo "tag: $TAG ✓"
else
  echo "tag: (HEAD is untagged — fine for a test build, tag before publishing)"
fi

"$CRATE_DIR/ci.sh"

for tool in cargo-audit cargo-deny; do
  have "$tool" || die "$tool is required for a release (cargo install $tool)"
done
step "cargo audit"; (cd "$CRATE_DIR" && cargo audit)
step "cargo deny";  (cd "$CRATE_DIR" && cargo deny check)

OUT="$REPO_ROOT/dist/$VERSION"

if [ "$DRY_RUN" = 1 ]; then
  step "dry run — would build"
  printf '  %s\n' "${MAC_TARGETS[@]}" "${LINUX_TARGETS[@]}"
  echo "  into $OUT"
  exit 0
fi

rm -rf "$OUT"; mkdir -p "$OUT"

{
  echo "ktflash $VERSION"
  echo "commit:  $(git -C "$REPO_ROOT" rev-parse HEAD)"
  echo "built:   $(date -u +%Y-%m-%dT%H:%M:%SZ) on $(uname -srm)"
  echo
  rustc -Vv
} > "$OUT/BUILD-INFO.txt"

# ---------------------------------------------------------------- helpers
# Every artifact is a tarball of the binary + license + README (+ udev rules on Linux), so
# `tar` in a terminal is the documented install path — which, on macOS, is also the path that
# does not set com.apple.quarantine (RELEASE-PLAN §4.3).
package() {
  local target="$1" bin="$2"
  local stage; stage="$(mktemp -d)/ktflash-$VERSION-$target"
  mkdir -p "$stage"
  cp "$bin" "$stage/ktflash"
  cp "$REPO_ROOT/LICENSE.md" "$REPO_ROOT/README.md" "$stage/"
  case "$target" in
    *linux*) cp "$REPO_ROOT/packaging/99-ktflash.rules" "$stage/" ;;
  esac
  tar -C "$(dirname "$stage")" -czf "$OUT/ktflash-$VERSION-$target.tar.gz" "$(basename "$stage")"
  echo "  packaged ktflash-$VERSION-$target.tar.gz"
}

# ---------------------------------------------------------------- macOS
if [ "$(uname -s)" = "Darwin" ] && [ -z "${KT_SKIP_MACOS:-}" ]; then
  step "macOS universal binary"
  for t in "${MAC_TARGETS[@]}"; do
    rustup target add "$t" >/dev/null 2>&1 || true
    (cd "$CRATE_DIR" && cargo build --release --target "$t")
  done
  UNI="$OUT/ktflash-universal"
  lipo -create -output "$UNI" \
    "$CRATE_DIR/target/aarch64-apple-darwin/release/ktflash" \
    "$CRATE_DIR/target/x86_64-apple-darwin/release/ktflash"

  # MANDATORY, and it must happen AFTER lipo: lipo invalidates any existing signature, and
  # Apple Silicon refuses to execute a binary with no signature at all. Ad-hoc (`-s -`) needs
  # no Apple account. Developer ID signing + notarization is a later milestone (RELEASE-PLAN §4.4).
  codesign --force --sign - "$UNI"
  codesign --verify --verbose "$UNI" 2>&1 | sed 's/^/  /'
  lipo -info "$UNI" | sed 's/^/  /'

  package "universal-apple-darwin" "$UNI"
  rm -f "$UNI"
fi

# ---------------------------------------------------------------- Linux
# cargo-zigbuild, not `cross`: no container runtime required. `--features vendored` builds
# libusb from source, and because no libudev headers are present it comes out udev-free, so
# the result is a fully static binary (RELEASE-PLAN §2).
if [ -z "${KT_SKIP_LINUX:-}" ]; then
  step "Linux static musl binaries"
  if ! have cargo-zigbuild || ! have zig; then
    echo "  cargo-zigbuild/zig missing — install with:"
    echo "    brew install zig && cargo install cargo-zigbuild"
    echo "  or build natively on the Linux VMs (docs/LINUX-TESTING.md) and drop the"
    echo "  binaries into $OUT before re-running with KT_SKIP_LINUX=1."
    die "cannot cross-build Linux targets"
  fi
  for t in "${LINUX_TARGETS[@]}"; do
    rustup target add "$t" >/dev/null 2>&1 || true
    (cd "$CRATE_DIR" && cargo zigbuild --release --features vendored --target "$t")
    bin="$CRATE_DIR/target/$t/release/ktflash"
    # A dynamically linked "static" build is the failure mode worth catching here, not on a
    # user's Alma box. `file` should say "statically linked".
    if have file && file "$bin" | grep -q "dynamically linked"; then
      die "$t built dynamically linked — the vendored libusb feature did not take effect"
    fi
    package "$t" "$bin"
  done
fi

# ---------------------------------------------------------------- packages
# cargo-deb and cargo-generate-rpm are pure Rust and run on macOS — no Debian or RPM host, and
# no container, is needed to build packages. Both wrap the static binary, so neither declares a
# libusb dependency (RELEASE-PLAN §5.2).
if have cargo-deb; then
  step "deb packages"
  for arch in amd64 arm64; do
    t=$([ "$arch" = amd64 ] && echo x86_64-unknown-linux-musl || echo aarch64-unknown-linux-musl)
    [ -f "$CRATE_DIR/target/$t/release/ktflash" ] || continue
    (cd "$CRATE_DIR" && cargo deb --no-build --target "$t" --output "$OUT/") || \
      echo "  cargo-deb failed for $arch — check [package.metadata.deb] in Cargo.toml"
  done
else
  echo "  cargo-deb not installed — skipping .deb (cargo install cargo-deb)"
fi

if have cargo-generate-rpm; then
  step "rpm packages"
  for t in "${LINUX_TARGETS[@]}"; do
    [ -f "$CRATE_DIR/target/$t/release/ktflash" ] || continue
    (cd "$CRATE_DIR" && cargo generate-rpm --target "$t" --output "$OUT/") || \
      echo "  cargo-generate-rpm failed for $t — check [package.metadata.generate-rpm]"
  done
else
  echo "  cargo-generate-rpm not installed — skipping .rpm (cargo install cargo-generate-rpm)"
fi

# ---------------------------------------------------------------- SBOM
if have cargo-cyclonedx; then
  step "SBOM"
  (cd "$CRATE_DIR" && cargo cyclonedx --format json) && \
    find "$CRATE_DIR" -maxdepth 1 -name '*.cdx.json' -exec mv {} "$OUT/ktflash-$VERSION.cdx.json" \;
fi

# ---------------------------------------------------------------- checksums + signature
step "checksums"
(cd "$OUT" && shasum -a 256 ./*.tar.gz ./*.deb ./*.rpm 2>/dev/null | sed 's|\./||' > SHA256SUMS)
cat "$OUT/SHA256SUMS"

if have minisign; then
  step "signing SHA256SUMS"
  # Signing only the checksum file means one signature covers every artifact, and users verify
  # with the pubkey committed at packaging/ktflash.pub.
  minisign -Sm "$OUT/SHA256SUMS"
  echo "  wrote SHA256SUMS.minisig"
else
  echo
  echo "  ⚠  minisign not installed — artifacts are UNSIGNED."
  echo "     brew install minisign; minisign -G  (keep the secret key offline)"
  echo "     Do not publish a release without a signature."
fi

step "done"
echo "artifacts in $OUT"
echo
echo "Next:"
echo "  1. Verify on a clean machine: shasum -a 256 -c SHA256SUMS"
echo "  2. Run the Linux matrix — docs/LINUX-TESTING.md"
echo "  3. gh release create v$VERSION $OUT/* --notes-file <notes.md>"
echo
echo "  Release notes MUST state: the macOS build is unsigned (no Apple Developer ID), how to"
echo "  run it (curl install avoids Gatekeeper; xattr -d com.apple.quarantine otherwise), and"
echo "  the honest support boundary for Linux-native flash-cdc."
