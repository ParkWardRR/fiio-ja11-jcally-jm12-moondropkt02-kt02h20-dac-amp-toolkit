# APPLY.md — integration guide

Ordered steps to move `staging/` into the real tree. Each step ends with a check. Steps 1–4 need
no hardware; steps 5–8 need a dongle.

**Before anything:** `./flasher/ci.sh` must be green on a clean tree, so you can tell your
breakage from mine.

---

## Step 1 — the driver refactor (no hardware, no behaviour change)

The highest-value, lowest-risk change: the flash state machine becomes testable.

```sh
cp staging/flasher-src/proto/ktcdc_driver.rs  flasher/src/proto/
cp staging/flasher-src/proto/ktcdc_journal.rs flasher/src/proto/
# add to flasher/src/proto/mod.rs (alphabetically):
#   pub mod ktcdc_driver;
#   pub mod ktcdc_journal;
```

`ktcdc_journal.rs` is the operation journal `flash-cdc` never had — see
[HANDOFF](HANDOFF.md#2-flash-cdc-was-writing-flash-with-no-recovery-record-at-all). Its
ordering discipline is the part to review: `Erased` is recorded *before* `KSTA` is sent.

**Then diff the sequence against the original before trusting it.** `main.rs:907-995` (in the
pre-change tree) is the reference. Confirm, in order: `KTM` → (`VER`,`KEY` if flag≠0) → `CHP`
(≥13 bytes, not an ACK) → `ERASE` → `PWO` → `KSTA` (long timeout) → data packets → `STP` →
(`INF` if flag≠0) → `RESET`. Confirm the drain-before-send and accumulate-until-ACK behaviour
survived. Any difference is a bug in my file.

```sh
cd flasher && cargo test proto::ktcdc   # 18 tests (driver + journal)
./ci.sh
```

---

## Step 2 — `libc` and the `vendored` feature

Merge from `staging/release/cargo-toml-additions.toml` into `flasher/Cargo.toml`:

- `[features] vendored = ["rusb/vendored"]`
- `libc = "0.2"` into the existing `[dependencies]`

Leave the `[package.metadata.deb]` / `[package.metadata.generate-rpm]` blocks until step 7.

```sh
cd flasher && cargo build --release && ./ci.sh
```

---

## Step 3 — the serial transport (no hardware to compile)

```sh
cp staging/flasher-src/serialtransport.rs flasher/src/
cp staging/flasher-src/boottransport.rs   flasher/src/
# in main.rs, next to `mod usbtransport;`:
#   mod boottransport;
#   mod serialtransport;
```

**Review the `UNVERIFIED:` markers before hardware testing.** The weakest is macOS port→device
matching: `matches_bootloader` returns `true` for any `/dev/cu.usbmodem*`, and
`open_bootloader()` refuses when there is more than one candidate rather than guessing. The
proper fix (IOKit `IOSerialBSDClient` + parent walk for `idVendor`/`idProduct`) is described in
the doc comment. Linux matching is exact via sysfs and needs no work.

Second thing to check: the `tio::` ioctl constants. They are hard-coded rather than taken from
`libc` so the build doesn't depend on per-platform constant availability. Verify
`TIOCMBIS = 0x5416` (Linux) and `0x8004746c` (macOS) against your headers.

```sh
cd flasher && cargo clippy --all-targets -- -D warnings && cargo test
```

---

## Step 4 — rewire `flash-cdc`

```sh
cp staging/flasher-src/cmd_flash_cdc.rs flasher/src/
# main.rs: add `mod cmd_flash_cdc;`
```

Then in `main.rs`:

1. **Delete** the existing `fn cmd_flash_cdc` (and its doc comment).
2. **Delete** `fn open_bootloader` — it becomes fully dead. (Verified: after step 4 nothing else
   calls it.)
3. Change the dispatch arm to `Some("flash-cdc") => cmd_flash_cdc::cmd_flash_cdc(&args[2..])`.
4. Add `--transport` / `--port` to the `HELP` string.

This exact sequence was rehearsed — see [README](README.md#verification). Expect
`cargo clippy --all-targets -- -D warnings` clean and 93 tests passing.

`cmd_flash_cdc` now also opens an operation journal and, on failure, prints
`ktflash recover <journal>` as the next step. Check the journal path in the output points
somewhere sensible on your machine (`KTFLASH_DATA_DIR` overrides it).

**Dry-run check (no hardware):** `ktflash flash-cdc --image fw.bin` must print a byte-identical
plan to the pre-change binary. Diff the two outputs.

---

## Step 5 — `bootdiag` over either transport (optional, needs review)

`cmd_flash_cdc.rs` also contains `bootdiag_live()`, currently `#[allow(dead_code)]`.

⚠ **It is not a drop-in replacement.** The existing `cmd_bootdiag` only *reads* the pipe;
`bootdiag_live` *sends* `KTM`, which is a better liveness test but **advances the one-shot state
machine**. Decide whether `bootdiag` should be non-advancing before wiring it, and if you keep
the send, document that `bootdiag` must be followed by a re-`unlock` before flashing.

---

## Step 6 — the macOS unlock experiments (needs a dongle)

Start with the Swift tool — it is the maintained one; the C probes are a minimal fallback.

```sh
cd staging/macos-native/ktmac
swift build
.build/debug/ktmac selftest        # 13 hardware-free checks, no dongle needed
.build/debug/ktmac list            # is the dongle enumerating?
.build/debug/ktmac ports           # do any serial ports resolve to a USB VID:PID?
.build/debug/ktmac unlock          # dry run — finds the device, sends nothing
.build/debug/ktmac unlock --send   # the actual experiment (recoverable)
```

The C probes remain for E3, which stayed in C because the `IOUSBLib` plug-in dance is far worse
from Swift and the C version already compiled:

```sh
cd staging/macos-native && make && sudo ./e3_interface_seize
```

**Known blocker, already observed here:** `IOHIDManagerOpen` returns `0xe00002e2`
(`kIOReturnNotPermitted`) on macOS 15 before any device is touched. That is TCC, not USB —
grant **Input Monitoring** to your terminal in System Settings → Privacy & Security, then
restart the terminal. Nothing about the dongle can be learned until this is granted.

Run order and what each result means: [`docs/MACOS-NATIVE.md`](../docs/MACOS-NATIVE.md) §4.1.
Pass/fail is unambiguous — the dongle re-enumerates as `8888:cdc0`, or it does not. `unlock` is
recoverable; a power-cycle undoes it.

**Whatever the outcome, correct the docs.** `docs/PROTOCOL.md` and
`docs/dongle-investigation.md` currently contradict each other about macOS `SetReport`; E1
settles it and one of them needs fixing.

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
