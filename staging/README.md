# `staging/` — untested proposed code

> [!WARNING]
> **Nothing in this directory is wired into the build, and none of it has touched hardware.**
> It is a working draft produced from [`docs/RELEASE-PLAN.md`](../docs/RELEASE-PLAN.md),
> [`docs/LINUX-TESTING.md`](../docs/LINUX-TESTING.md) and
> [`docs/MACOS-NATIVE.md`](../docs/MACOS-NATIVE.md), for another agent to analyse, test, and
> integrate. Treat every file as a proposal, not a deliverable.

## What was and wasn't verified

| | |
|---|---|
| ✅ **Rust builds, fully integrated** | The whole of `flasher-src/` was applied to a throwaway copy of the real crate — dispatch rewired, the old `cmd_flash_cdc` and `open_bootloader` deleted. `cargo clippy --all-targets -- -D warnings` clean. |
| ✅ **Rust tests pass** | **93 total**: the existing 70, plus 23 new (10 driver, 8 journal, 3 serial, 2 transport-selection). |
| ✅ **Swift builds and self-checks pass** | `swift build` clean; `ktmac selftest` → 13 passed, 0 failed, 1 skipped. `list` / `ports` / `port` / `unlock --dry-run` all run. |
| ✅ **Go builds and runs** | `go vet` and `gofmt` clean; `vmatrix init` → `facts` → report exercised end-to-end against unreachable hosts. |
| ✅ **C probes compile and run** | Both build clean under `-Wall -Wextra` and were executed. This produced a real finding — see below. |
| ✅ **Shell scripts parse** | `bash -n` / `sh -n` clean. |
| ❌ **No hardware** | No dongle was attached. Nothing here has driven a real bootloader, opened a real serial port, or sent a real HID report. |
| ❌ **Shell scripts never executed** | `release.sh`, `install.sh` and the test scripts were not run. `shellcheck` is not installed here, so they are **not** shellcheck-verified. |
| ❌ **No cross-compile attempted** | `cargo-zigbuild` and `zig` are not installed here. RELEASE-PLAN §3.2 flags this as the one unproven link. |
| ❌ **Packaging metadata unbuilt** | `cargo-deb` / `cargo-generate-rpm` were never run; the `Cargo.toml` blocks for them are unvalidated. |
| ❌ **No Swift test target** | Neither XCTest nor swift-testing exists with CLT-only (no Xcode.app). The checks are `ktmac selftest` instead, which is why they actually ran. |

### One finding already

Running `e1_hid_setreport` with no device attached returned `0xe00002e2`
(`kIOReturnNotPermitted`) from `IOHIDManagerOpen` — **before any USB device is touched**. That is
macOS TCC: opening an `IOHIDManager` requires **Input Monitoring** consent for the calling app
(for a CLI, the terminal). It gates the entire E1/E2 experiment, and if ktflash ends up shipping
an `IOHIDManager`-based unlock it becomes part of the macOS first-run experience. Captured in the
probe's error path and in [`docs/MACOS-NATIVE.md`](../docs/MACOS-NATIVE.md) §4.1.

## Layout

```
staging/
├── HANDOFF.md                   ← START HERE if you are picking this up
├── APPLY.md                     ordered integration guide
├── flasher-src/                 Rust intended for flasher/src/
│   ├── proto/ktcdc_driver.rs      the flash-cdc state machine, lifted out of main.rs
│   │                              onto proto::cdc::Transport — hardware-free + unit-tested
│   ├── proto/ktcdc_journal.rs     operation journaling for flash-cdc (it had none)
│   ├── serialtransport.rs         SerialTransport: /dev/cu.usbmodem* | /dev/ttyACM*
│   ├── boottransport.rs           transport auto-selection (serial vs libusb)
│   └── cmd_flash_cdc.rs           replacement for main.rs cmd_flash_cdc
├── macos-native/                MACOS-NATIVE.md §4.1
│   ├── ktmac/                     Swift package — the real macOS-native work
│   │   ├── Sources/KTMacKit/        USB enumeration, serial→USB resolution,
│   │   │                            HID unlock (E1/E2), self-tests
│   │   └── Sources/ktmac/           CLI: list | ports | port | watch | unlock | selftest
│   ├── e1_hid_setreport.c         E1/E2 in C (superseded by ktmac; kept as a minimal probe)
│   ├── e3_interface_seize.c       E3 — USBInterfaceOpenSeize + WritePipe (stayed in C)
│   └── Makefile
├── tools/vmatrix/               Go — SSH runner for the Linux test matrix + report generator
├── release/                     RELEASE-PLAN.md workstream A
│   ├── rust-toolchain.toml
│   ├── release.sh                 build matrix → tarballs → SHA256SUMS → minisign
│   ├── install.sh                 curl|sh installer, verifies before installing
│   └── cargo-toml-additions.toml  fragments to merge into flasher/Cargo.toml
├── packaging/
│   ├── 99-ktflash.rules           + ModemManager ignore rules (RELEASE-PLAN §5.3)
│   └── deb/postinst
└── linux-testing/               LINUX-TESTING.md, executable
    ├── collect-host-facts.sh
    ├── p0-smoke.sh
    ├── p2-usb-checks.sh
    └── results-template.md
```

## The three things worth reviewing first

1. **`flasher-src/proto/ktcdc_journal.rs`** — closes a real safety gap. `flash-cdc` was writing
   flash with **no operation journal at all**, so an interrupted flash left `ktflash recover`
   nothing to read, despite ROADMAP Appendix A's "destructive writes ship only with recovery
   semantics". It records `Erased` **before `KSTA` goes on the wire**, because KSTA triggers the
   erase and the flash is gone whether or not we see the ACK.

2. **`flasher-src/proto/ktcdc_driver.rs`** — lifts the bootloader state machine out of
   `main.rs:907-995`, where it is welded to `rusb` closures and cannot be tested, into `proto/`
   where it is generic over `Transport` and driven by a fake. The sequence is transcribed
   step-for-step; **diff it against `main.rs` before trusting it.** Any behavioural difference
   is a bug in this file, not an intentional change.

3. **`macos-native/ktmac/Sources/KTMacKit/SerialPortFinder.swift`** — resolves `/dev/cu.*` back
   to its USB VID/PID via `IORegistryEntrySearchCFProperty` with `kIORegistryIterateParents`.
   This is the correct answer to the biggest `UNVERIFIED:` in `serialtransport.rs`, which
   currently accepts any `/dev/cu.usbmodem*` on macOS. Port it into Rust, or shell out to
   `ktmac port`.

## Verification

The Rust was checked against the **real crate**, not stubs. A throwaway copy of `flasher/` was
made, every file in `flasher-src/` dropped in, and the integration from
[`APPLY.md`](APPLY.md) steps 1–4 applied in full — modules declared, dispatch rewired to the new
`cmd_flash_cdc`, and the now-dead inline `cmd_flash_cdc` and `open_bootloader` deleted from
`main.rs`. Then:

```sh
cargo clippy --all-targets -- -D warnings   # clean
cargo test                                  # 93 passed; 0 failed
```

Clippy caught four real defects in this code on the first pass (a manual `Default` impl, a
never-read assignment, and two dead-code paths); all are fixed. So `APPLY.md` steps 1–4 are a
rehearsed sequence rather than a guess.

The other languages:

```sh
cd macos-native/ktmac && swift build && .build/debug/ktmac selftest   # 13 passed, 1 skipped
cd macos-native && make                                              # -Wall -Wextra, no warnings
cd tools/vmatrix && gofmt -l . && go vet ./... && go build            # clean
```

`vmatrix` was exercised end-to-end (`init` → `facts` → report) against deliberately unreachable
hosts, which surfaced two reporting bugs — both fixed, and the reason the "questions" table now
distinguishes "asked and the answer is no" from "never asked".

What none of this proves: that any of it works against hardware. Every I/O path —
`SerialTransport`'s termios setup, port discovery, the driver against a real bootloader, the
HID unlock, SSH to a real guest — is exercised only by fakes, by `/dev` on a Mac with no dongle,
and by IOKit queries that return no dongle.

## Ground rules that were kept

- No new runtime dependency except `libc` (already in `Cargo.lock` transitively; MIT/Apache-2.0,
  so no new `cargo deny` question). `serialport` was deliberately not used.
- `proto/` stays free of `rusb` and OS I/O — `ktcdc_driver.rs` respects that.
- Destructive gates untouched: `--execute` + `--yes` still required, dry-run still the default.
- Nothing here distributes a firmware image.
