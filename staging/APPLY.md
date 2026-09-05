# APPLY.md — remaining integration steps

Steps 1–6 have **landed**; see [`HANDOFF.md`](HANDOFF.md). What follows is what is left.

- ~~Step 1 — driver refactor + journal~~ → `flasher/src/proto/ktcdc_driver.rs`, `ktcdc_journal.rs`
- ~~Step 2 — `libc` + the `vendored` feature~~ → `flasher/Cargo.toml`
- ~~Step 3 — serial transport~~ → `flasher/src/serialtransport.rs`, `boottransport.rs`
- ~~Step 4 — rewire `flash-cdc`~~ → `flasher/src/cmd_flash_cdc.rs`, plus the post-reset reprobe
  in `flasher/src/proto/postflash.rs`
- Step 5 — `bootdiag` over either transport: still open, still needs the judgement call below
- ~~Step 6 — the macOS experiments~~ → moved to `macos/native/`, and now driven by `ktmac flow`

**Before anything:** `./flasher/ci.sh` must be green on a clean tree, so you can tell your
breakage from mine. Current baseline: clippy clean, 102 tests.

---

## Step 5 — `bootdiag` over either transport (optional, needs review)

`cmd_flash_cdc.rs` also contains `bootdiag_live()`, currently `#[allow(dead_code)]`.

⚠ **It is not a drop-in replacement.** The existing `cmd_bootdiag` only *reads* the pipe;
`bootdiag_live` *sends* `KTM`, which is a better liveness test but **advances the one-shot state
machine**. Decide whether `bootdiag` should be non-advancing before wiring it, and if you keep
the send, document that `bootdiag` must be followed by a re-`unlock` before flashing.

---

## Step 7 — packaging and release scripts

```sh
cp staging/packaging/99-ktflash.rules packaging/     # adds the ModemManager entries
mkdir -p packaging/deb && cp staging/packaging/deb/postinst packaging/deb/
cp staging/release/release.sh scripts/ && chmod +x scripts/release.sh
cp staging/release/install.sh scripts/ && chmod +x scripts/install.sh
cp staging/release/rust-toolchain.toml .            # REPO ROOT, not flasher/
```

Then merge the `[package.metadata.deb]` and `[package.metadata.generate-rpm]` blocks from
`cargo-toml-additions.toml`.

Before the first real run:

- [ ] Generate a minisign key (`minisign -G`), keep the secret **offline**, commit the public key
      to `packaging/ktflash.pub`, and replace the `RWQPLACEHOLDER…` constant in `install.sh`.
- [ ] Replace the `maintainer` email in the deb metadata.
- [ ] `./scripts/release.sh --dry-run` first — it runs every gate and builds nothing.
- [ ] `brew install zig && cargo install cargo-zigbuild` for the Linux targets.
- [ ] **Verify the one unproven link** (RELEASE-PLAN §3.2): that `cargo-zigbuild` builds the
      vendored libusb C sources. If it doesn't, build natively on the VMs and re-run with
      `KT_SKIP_LINUX=1`.

`install.sh`'s udev prompt reads from stdin, which does not exist under `curl | sh`. It is
guarded with `[ -t 0 ]` so it degrades to printing the manual command — confirm that is the
behaviour you want before publishing the one-liner.

---

## Step 8 — Linux validation

```sh
mkdir -p scripts/testing && cp staging/linux-testing/*.sh scripts/testing/
chmod +x scripts/testing/*.sh
cp staging/linux-testing/results-template.md docs/
cp -R staging/tools/vmatrix tools/
```

Two ways to drive it. **By hand**, per guest: `collect-host-facts.sh` → `p0-smoke.sh` →
(dongle) `p2-usb-checks.sh before` → `ktflash unlock` → `p2-usb-checks.sh after`.

**Or with the Go runner**, which does facts/deploy/install/P0/P1 across all guests
concurrently and generates the results markdown:

```sh
cd tools/vmatrix && go build
./vmatrix init                       # writes hosts.json — edit the ssh targets
./vmatrix run -dist ../../dist/1.2.0 -version 1.2.0
```

It deliberately **will not** run P2 (unlock) or P3 (flash): those change device state and
destroy firmware, and should never happen because someone typed a command that looked like it
only ran tests.

**Configure passthrough by bus/port, not VID:PID, before you start** — `unlock` re-enumerates
the device and an ID-keyed rule drops it exactly when it matters
([`LINUX-TESTING.md`](../docs/LINUX-TESTING.md) §2.1).

---

## Step 9 — docs

Once the above lands, these need updating (they currently describe a world where macOS needs
OrbStack for everything and there are no prebuilt binaries):

| File | Change |
|---|---|
| `ROADMAP.md` Appendix A | `flash-cdc` now journals — the safety model claim is finally true for the path users actually run |
| `README.md` | download-first Quickstart; "Where it runs" table |
| `docs/FLASHING.md` | "Why macOS needs OrbStack" → conditional; unsigned-binary run instructions |
| `docs/PROTOCOL.md` | the macOS caveat, per the E1 result |
| `docs/LINUX.md` | package install first; `--transport serial` |
| `docs/MACOS-NATIVE.md` | fold in E1–E3 results; the TCC finding from step 6 |
| `flasher/src/usbtransport.rs` | module docs still say "use OrbStack" |
| `orbstack/README.md` | demote to fallback — **do not delete**, it is the known-good path |
| `ROADMAP.md`, `CHANGELOG.md` | Phase 4 progress, v1.2.0 |

## Step 10 — delete `staging/`

It is a scratchpad, not a second source of truth. Once the code lives in `flasher/src/`,
`scripts/`, and `packaging/`, delete this directory in the same PR that lands the last piece.
