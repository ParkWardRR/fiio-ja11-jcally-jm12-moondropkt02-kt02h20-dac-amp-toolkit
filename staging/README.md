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

## What is left

```
staging/
├── HANDOFF.md                   ← current state of everything
├── APPLY.md                     remaining integration steps (7-9)
├── release/
│   ├── rust-toolchain.toml        pinned channel + release targets
│   ├── release.sh                 build matrix → tarballs → SHA256SUMS → minisign
│   ├── install.sh                 curl|sh installer, verifies before installing
│   └── cargo-toml-additions.toml  deb/rpm metadata to merge into flasher/Cargo.toml
├── packaging/
│   ├── 99-ktflash.rules           + ModemManager ignore rules (RELEASE-PLAN §5.3)
│   └── deb/postinst
└── linux-testing/
    ├── collect-host-facts.sh
    ├── p0-smoke.sh
    ├── p2-usb-checks.sh
    └── results-template.md
```

## Verification status of what remains

| | |
|---|---|
| ✅ **Shell parses** | `bash -n` / `sh -n` clean. |
| ❌ **Never executed** | `release.sh`, `install.sh` and the test scripts were not run. |
| ❌ **Not shellcheck-verified** | `shellcheck` is not installed on the authoring machine. |
| ❌ **Packaging metadata unbuilt** | `cargo-deb` / `cargo-generate-rpm` were never run. |
| ❌ **No cross-compile attempted** | `zig` / `cargo-zigbuild` are not installed here. RELEASE-PLAN §3.2 flags this as the one unproven link. |

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

Delete this directory once `release/`, `packaging/` and `linux-testing/` have landed in
`scripts/`, `packaging/` and `scripts/testing/`.
