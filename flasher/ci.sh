#!/usr/bin/env bash
# Local CI for ktflash — the trust gate for a tool that erases real hardware.
# No GitHub Actions: run this locally (and it runs automatically on `git push` via the
# repo's pre-push hook — see scripts/hooks/pre-push).
#
#   ./flasher/ci.sh          # from the repo root, or
#   cd flasher && ./ci.sh
#
# Gates: clippy (warnings denied), the hardware-free unit tests, and a `bootdiag --replay`
# smoke test over the capture fixture. No dongle required — the protocol core (src/proto/) is
# transport-independent. (Release builds are validated at release time, not on every push.)
#
# NOTE: `cargo fmt --check` is intentionally NOT a gate yet — the crate predates a rustfmt
# pass. Add it once the whole crate has been formatted in one dedicated commit.
set -euo pipefail

# Make cargo reachable even from a hook's minimal environment.
if ! command -v cargo >/dev/null 2>&1; then
  [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
fi
if ! command -v cargo >/dev/null 2>&1; then
  export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
fi
command -v cargo >/dev/null 2>&1 || { echo "ci: cargo not found on PATH" >&2; exit 127; }

cd "$(dirname "$0")"

echo "==> clippy (deny warnings)"
cargo clippy --all-targets -- -D warnings

echo "==> test"
cargo test

echo "==> replay fixtures (no hardware)"
cargo run --quiet -- bootdiag --replay fixtures/synthetic-success.json >/dev/null

# Supply-chain gates run only when the tools are installed, so they never block a contributor
# who hasn't installed them. Install with: cargo install cargo-audit cargo-deny
if command -v cargo-audit >/dev/null 2>&1; then
  echo "==> cargo audit (RustSec advisories)"
  cargo audit
else
  echo "==> cargo audit — skipped (cargo-audit not installed)"
fi
if command -v cargo-deny >/dev/null 2>&1; then
  echo "==> cargo deny (licenses / bans / sources / advisories)"
  cargo deny check
else
  echo "==> cargo deny — skipped (cargo-deny not installed)"
fi

echo "local CI passed ✅"
