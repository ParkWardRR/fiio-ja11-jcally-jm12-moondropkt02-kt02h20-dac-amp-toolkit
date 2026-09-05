# Linux test results — ktflash v____ — filled in ____-__-__

Copy this per test round. Paste `collect-host-facts.sh` output under each guest.
Phases and pass criteria: [`docs/LINUX-TESTING.md`](../../docs/LINUX-TESTING.md) §3.

**Legend:** ✅ pass · ❌ fail · ⏳ not run · n/a not applicable

## Setup

| | |
|---|---|
| Hypervisor | |
| Dongle attached to | VM host / Windows box via usbipd / other |
| Passthrough bound by | **port** (correct) / VID:PID (will break on unlock) |
| Passthrough survives re-enumeration? | ⏳ — verify with `lsusb -t` before and after `unlock` |
| Artifacts under test | `ktflash-____-x86_64-unknown-linux-musl.tar.gz`, `.deb`, `.rpm` |
| Build provenance | zigbuild cross / native-on-VM |
| Sacrificial dongle for P3? | |
| Known-good image saved? | **required before P3** |

## Matrix

| Guest | Arch | Artifact | P0 | P1 probe | P2 unlock | P2 bootdiag | P3 flash | Notes |
|---|---|---|:--:|:--:|:--:|:--:|:--:|---|
| V1 Debian 13 | x86_64 | musl tar | ⏳ | ⏳ | ⏳ | ⏳ | ⏳ | |
| V2 Debian 12 | x86_64 | .deb | ⏳ | ⏳ | ⏳ | ⏳ | ⏳ | |
| V3 AlmaLinux 10 | x86_64 | .rpm | ⏳ | ⏳ | ⏳ | ⏳ | ⏳ | |
| V4 AlmaLinux 9 | x86_64 | .rpm | ⏳ | ⏳ | ⏳ | ⏳ | ⏳ | |
| V5 Ubuntu 22.04 | x86_64 | musl tar | ⏳ | ⏳ | ⏳ | ⏳ | ⏳ | |
| V6 Arch (opt) | x86_64 | musl tar | ⏳ | ⏳ | ⏳ | ⏳ | ⏳ | |
| V7 Debian 12 (opt) | aarch64 | musl tar | ⏳ | ⏳ | ⏳ | ⏳ | ⏳ | |
| V8 Alma 9 (opt) | aarch64 | .rpm | ⏳ | ⏳ | ⏳ | ⏳ | ⏳ | |
| source-build control | x86_64 | `cargo build` | ⏳ | ⏳ | ⏳ | ⏳ | ⏳ | |

## Questions this round must answer

These are the open items the plans are blocked on. Answer them explicitly — "it worked" is not
an answer to any of them.

| # | Question | Answer |
|---|---|---|
| 1 | Is the musl static build genuinely static everywhere? (`file` / `ldd`) | |
| 2 | Does `probe` work **without sudo** once the udev rules are installed? | |
| 3 | Same question on a **headless** guest (no logind seat — does `uaccess` still apply, or is the `plugdev` fallback needed)? | |
| 4 | **Does ModemManager actually probe the `8888:cdc0` tty?** Run P2 both with and without the `ID_MM_DEVICE_IGNORE` rules. Decides whether they ship as a fix or a comment. | |
| 5 | Does the hypervisor keep the device across `unlock`'s re-enumeration? | |
| 6 | Does `flash-cdc --execute` complete Linux-natively? (**gates ROADMAP Phase 4.1**) | |
| 7 | If the serial transport landed: does `--transport serial` work on Linux, and is it more or less reliable than libusb? | |
| 8 | Any distro where the TUI misrenders or leaves the terminal in raw mode? | |

## P3 record (destructive — one row per attempt)

| Guest | Dongle | Image (name + sha256) | Result | Audio after? | Journal file |
|---|---|---|---|---|---|
| | | | | | |

If a flash fails, attach `ktflash-<op>.journal.json` and the output of
`ktflash recover <journal>`. Do not retry blind — the journal states the safe next step.

## Descriptors captured (ROADMAP Phase 6)

- [ ] before-flash descriptors (`lsusb -v` / `ktflash fingerprint`)
- [ ] after-flash descriptors
- [ ] `ktflash compat --template` record filled in and validated
