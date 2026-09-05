# Releasing ktflash (roadmap Phase 4)

This is a tool that erases real hardware, so a release is a trust event, not just a tag. No
GitHub Actions — everything here runs locally (or on a Linux box you control).

## Pre-release gate

```sh
cd flasher
./ci.sh                 # clippy -D warnings, tests, replay smoke, + cargo audit/deny if installed
cargo install cargo-audit cargo-deny   # once, so the supply-chain gates actually run
```

`ci.sh` skips `cargo audit`/`cargo deny` if they aren't installed — install them for a real
release. `deny.toml` holds the license/advisory policy; extend the `allow` list (with a glance at
the crate) if `cargo deny` reports a new license.

## Reproducibility

- `Cargo.lock` is committed. ✅
- Record the toolchain + target used (`rustc -Vv`).
- Build with a pinned stable toolchain.

## Build matrix

| Target | How |
|---|---|
| macOS Apple Silicon | `cargo build --release` on an arm64 mac |
| macOS Intel | `cargo build --release --target x86_64-apple-darwin` (if supported) |
| Linux x86_64 | `cargo zigbuild --release --target x86_64-unknown-linux-gnu` or `cross` |
| Linux aarch64 | `cargo zigbuild --release --target aarch64-unknown-linux-gnu` or `cross` |

libusb must be available for each target (Linux: `libusb-1.0-0-dev` + `pkg-config`).

## Artifacts

- SHA-256 every binary: `shasum -a 256 ktflash* > SHA256SUMS`.
- Ship the udev rules ([`../packaging/99-ktflash.rules`](../packaging/99-ktflash.rules)) with
  Linux builds; document install (see [`LINUX.md`](LINUX.md)).
- Generate an SBOM if maintainable (e.g. `cargo cyclonedx`).

## macOS notarization (its own milestone)

Signing + notarization needs Apple Developer credentials. Do this **before** publishing a
Homebrew tap; an unnotarized binary is a bad first-run experience. Track it as a discrete task.

## Versioning

- Semver in `flasher/Cargo.toml`; update `CHANGELOG.md`.
- Tag only after the gate passes on every target in the matrix.
- State the support boundary honestly: which devices are `verified`/`restore-verified` vs.
  `experimental` (see the compatibility matrix, `ktflash compat`).
