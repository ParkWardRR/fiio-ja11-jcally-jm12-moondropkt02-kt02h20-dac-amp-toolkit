# `staging/` — release plumbing not yet integrated

> [!WARNING]
> Nothing in this directory is wired into the build, and **none of it has been executed**.
> It is a working draft for [`docs/RELEASE-PLAN.md`](../docs/RELEASE-PLAN.md) and
> [`docs/LINUX-TESTING.md`](../docs/LINUX-TESTING.md). Treat every file as a proposal.

**Start with [`HANDOFF.md`](HANDOFF.md)** for the current state of the whole effort, then
[`APPLY.md`](APPLY.md) for the remaining integration steps.

## What used to be here and has moved out

| Was | Now | Why |
|---|---|---|
| `flasher-src/` | `flasher/src/` | integrated; keeping a copy meant two sources of truth for firmware-erasing code |
| `macos-native/` | [`macos/native/`](../macos/native/) | the `ktmac` Swift package and C probes are real work now, not a proposal |
| `tools/vmatrix/` | [`tools/vmatrix/`](../tools/vmatrix/) | standalone Go tool; needs no integration, only testing |
| `release/rust-toolchain.toml` | [`/rust-toolchain.toml`](../rust-toolchain.toml) (repo root) | pinned channel + release targets |
| `release/release.sh` | [`scripts/release.sh`](../scripts/release.sh) | build matrix → tarballs → SHA256SUMS → minisign |
| `release/install.sh` | [`scripts/install.sh`](../scripts/install.sh) | curl\|sh installer, verifies before installing |
| `release/cargo-toml-additions.toml` | merged into [`flasher/Cargo.toml`](../flasher/Cargo.toml) | `[package.metadata.deb]` / `[package.metadata.generate-rpm]`; the `vendored` feature and `libc` dep had already landed |
| `packaging/99-ktflash.rules` | [`packaging/99-ktflash.rules`](../packaging/99-ktflash.rules) | was already identical to the shipped file — the ModemManager fix had already been merged separately |
| `packaging/deb/postinst` | [`packaging/deb/postinst`](../packaging/deb/postinst) | needed by the `maintainer-scripts` key just added to `flasher/Cargo.toml` |
| `linux-testing/*.sh` | [`scripts/testing/`](../scripts/testing/) | `collect-host-facts.sh`, `p0-smoke.sh`, `p2-usb-checks.sh` |
| `linux-testing/results-template.md` | [`docs/results-template.md`](../docs/results-template.md) | |

## What is left

Everything under `release/`, `packaging/`, and `linux-testing/` has landed. What remains is only:

- **Step 5** (`APPLY.md`) — the `bootdiag` judgement call, still open.
- **Step 9** (`APPLY.md`) — the docs pass (README quickstart, FLASHING.md, LINUX.md, etc.) once
  everything above is confirmed working.
- The minisign key itself: `install.sh` still has the `RWQPLACEHOLDER…` constant, and no key has
  been generated. That is a deliberate supply-chain decision (where the private key lives,
  custody) made separately with explicit sign-off, not a mechanical promotion step.
- Actually running any of this for real: `release.sh --dry-run`, `cargo-deb`,
  `cargo-generate-rpm`, `cargo-zigbuild` have still never been executed — only `bash -n` /
  `cargo metadata` sanity checks have been done.

**Not planned:** AlmaLinux/RHEL hardware validation via remote VM/USB-IP was explicitly dropped
as a project goal. Alma/RHEL remain supported **packaging** targets (the `.rpm` metadata above
targets them); only the "validate over VM passthrough" testing approach was dropped.

## The two udev changes worth reviewing

`packaging/99-ktflash.rules` differs from the shipped one in two ways
([RELEASE-PLAN §5.3](../docs/RELEASE-PLAN.md)):

1. **ModemManager ignore rules** for `8888:cdc0` and its tty. After `unlock` the device exposes
   a CDC-ACM tty, and on both Debian and Alma ModemManager opens new `ttyACM*` to probe for
   modems — a live race against a flash in progress. **Marked UNVERIFIED**: it is not yet
   confirmed that ModemManager actually probes this device. `docs/LINUX-TESTING.md` P2 says to
   test both ways before deciding whether this ships as a fix or a comment.
2. A note that `0x8888` is a squatted vendor ID, so every rule matching it keeps its `cdc0`
   product-ID qualifier.

## Ground rules kept

- No firmware images distributed.
- Destructive gates untouched: `--execute` + `--yes` still required, dry run still the default.
- `install.sh` verifies signature and checksum **before** anything is extracted or made
  executable, and refuses rather than warning.
- Honest labels: nothing claims to work, because nothing has been run.

`release/`, `packaging/` and `linux-testing/` have landed in `scripts/`, `packaging/` and
`scripts/testing/` (plus `docs/results-template.md`). Delete this directory once Step 5 and
Step 9 (`APPLY.md`) are done — it is a scratchpad, not a second source of truth.
