# HANDOFF — what has landed, what is left

**Updated:** 2026-09-05 (session 2) · **From:** a no-hardware session
**Read order:** this file → [`APPLY.md`](APPLY.md) (what still needs integrating) → [`README.md`](README.md)

---

## Where things stand

Three plans ([`RELEASE-PLAN`](../docs/RELEASE-PLAN.md), [`LINUX-TESTING`](../docs/LINUX-TESTING.md),
[`MACOS-NATIVE`](../docs/MACOS-NATIVE.md)) were written and implemented as far as a machine with
**no dongle** can take them. Two thirds of it has now graduated out of `staging/` into the real
tree. **Nothing has touched hardware.**

| | Where it lives now | State |
|---|---|---|
| **Rust** — driver refactor, serial transport, journal, **post-reset reprobe** | `flasher/src/` | ✅ integrated · clippy clean · **102 tests** |
| **Swift** — `ktmac`: IOKit, unlock experiments, **the whole native flow** | `macos/native/ktmac/` | ✅ graduated · builds · **20 selftest checks** |
| **C** — E3 `USBInterfaceOpenSeize` probe | `macos/native/` | ✅ graduated · compiles `-Wall -Wextra` |
| **Go** — `vmatrix` SSH test-matrix runner | `tools/vmatrix/` | ✅ graduated · `go vet`/`gofmt` clean |
| **Shell** — release, install, host-facts, P0/P2 | `staging/release/`, `staging/linux-testing/` | ⏳ **still in staging**, never executed |
| **Packaging** — udev rules, deb postinst, Cargo metadata | `staging/packaging/`, `staging/release/` | ⏳ **still in staging**, never built |

`staging/flasher-src/` was deleted once its contents landed: two copies of the code that erases
firmware is exactly the situation the ground rules were meant to prevent.

---

## What changed this session

### 1. ROADMAP Phase 3.4 is closed (in code)

The item read: *"⏳ Still to harden: journal each stage before its destructive command and
confirm success by a post-reset reprobe."*

- **Journal before the destructive command** — [`proto/ktcdc_journal.rs`](../flasher/src/proto/ktcdc_journal.rs).
  `Erased` is recorded **before `KSTA` goes on the wire**: KSTA triggers the erase, so the flash
  is gone whether or not we live to see the ACK. Journalling on the success edge would tell a
  crashed operator that nothing destructive happened.
- **Post-reset reprobe** — [`proto/postflash.rs`](../flasher/src/proto/postflash.rs), new this
  session. `flash-cdc` now polls the bus after `RESET` and records `Confirmed` or
  `IdentityMismatch`, so `ktflash recover` can finally say *done*. New flags:
  `--expect VID:PID`, `--no-reprobe`, `--reprobe-timeout`.

  Two deliberately conservative rules, both unit-tested:
  - **An ISP-mode device anywhere on the bus is never a success.** Even if some other runtime
    dongle is present. Claiming success while a device sits in the bootloader tells the operator
    to walk away from something that still needs reflashing.
  - **A mismatch is only reported against an explicit `--expect`.** `IdentityMismatch` is a halt
    state; inferring the expected VID:PID and then halting on it would manufacture alarm from a
    guess.

### 2. `ktmac flow` — the macOS path with no OrbStack

`orbstack/ktflash-orbstack.sh flash` creates an Ubuntu guest, installs build deps, passes the
dongle through, builds ktflash in the guest, unlocks, re-attaches the device because it
re-enumerated, and flashes over libusb. Every step of that exists to work around one thing:
macOS refusing to let libusb claim the HID interface.

`ktmac flow --image fw.bin` does it natively:

```
preflight → dry run → confirm → unlock (IOHIDManager) → resolve /dev/cu.* → ktflash flash-cdc
  --transport serial --port … --execute --yes → ktflash's own post-reset reprobe
```

It **shells out to the Rust `ktflash`** for the write rather than reimplementing the protocol:
the framing, journal and safety gates all live there, and a second implementation of the thing
that erases firmware is the last thing this project needs. It prints the exact command it runs.

Also new: `ktmac doctor`, which checks everything the flow needs and reports the dry-run and
execute gates separately.

### 3. A design flaw I introduced and then fixed

The first version of `flow` required TCC consent and an attached dongle before it would do
anything — including the **dry run**, which only reads a file and prints a packet plan. That
blocked the exact inspect-before-you-commit workflow the dry run exists for. Preflight checks
now carry a `requiredFor: .always | .execute`, and a dry run needs only `ktflash` itself.

---

## Still to do

**No hardware needed:**

1. **[`APPLY.md`](APPLY.md) step 7** — packaging and release scripts into `packaging/` and
   `scripts/`, plus the `[package.metadata.deb]` / `[generate-rpm]` blocks. None of it has been
   built; `cargo-deb` and `cargo-generate-rpm` were never run.
2. Run the shell scripts through `shellcheck` (not available on the authoring machine) and
   actually execute `./scripts/release.sh --dry-run`.
3. `brew install zig && cargo install cargo-zigbuild`, then verify the one unproven link:
   that zigbuild can compile vendored libusb's C sources.

**Needs a Mac + dongle — in this order:**

4. `ktmac list`, then after an OrbStack unlock, `ktmac port`. **This single check validates the
   whole macOS-native premise**: if `8888:cdc0` does not publish a `/dev/cu.usbmodem*` node,
   Stage B needs rethinking before any more code is written. Ten minutes.
5. Grant Input Monitoring, then `ktmac unlock --send`. Then `--no-id-prefix`, then `--seize`,
   then the C E3 probe. Pass = the dongle re-enumerates as `8888:cdc0`.
6. `ktmac flow --image fw.bin` (dry run first, then `--execute`).
7. **Whatever the unlock result, fix the docs.** `PROTOCOL.md` and `dongle-investigation.md`
   contradict each other about macOS `SetReport`; E1 settles it and one of them is wrong.

**Needs the Linux VMs:**

8. `cd tools/vmatrix && go build && ./vmatrix init`, fill in SSH targets, `./vmatrix run`.
9. **Configure USB passthrough by bus/port, not VID:PID, before anything else.** `unlock`
   re-enumerates the dongle under a new ID, so an ID-keyed hypervisor rule drops it at exactly
   the moment it matters — `unlock` looks like it succeeded and the bootloader never appears.
10. P2 and P3 by hand. `vmatrix` refuses to automate them: they change device state and destroy
    firmware, and must not happen because someone typed a command that looked like it ran tests.

---

## Where the risk actually is

Ranked. Everything is untested against hardware; this is about which parts will bite.

1. **`serialtransport.rs` macOS port matching** falls back to "any `/dev/cu.usbmodem*`".
   **`ktmac`'s [`SerialPortFinder.swift`](../macos/native/ktmac/Sources/KTMacKit/SerialPortFinder.swift)
   is the correct implementation** (`IORegistryEntrySearchCFProperty` +
   `kIORegistryIterateParents`). Port it into Rust, or shell out to `ktmac port`. Do not ship the
   fallback — guessing wrong aims a firmware write at the wrong device.
2. **termios binary safety.** The protocol contains `0x11`/`0x13` (XON/XOFF) and `0x0d`/`0x0a`.
   `cfmakeraw` plus explicit `IXON`/`ICRNL`/`OPOST` clears are in there, but this is exactly the
   kind of thing that silently corrupts one byte of a firmware image.
3. **The `tio::` ioctl constants** are hard-coded (`TIOCMBIS` = `0x5416` Linux, `0x8004746c`
   macOS) rather than taken from `libc`. Check them against your headers.
4. **The reprobe's runtime-device pick** takes the *first* non-bootloader dongle `scan()`
   returns. With two dongles attached that could be the wrong one. `--expect` catches it; think
   about whether it should be required when more than one device is present.
5. **`cargo-zigbuild` + vendored libusb** — the unproven link in the release plan §3.2.
6. **`bootdiag_live()`** in `cmd_flash_cdc.rs` is `#[allow(dead_code)]` and **not** a drop-in
   replacement — it *sends* `KTM`, which advances the one-shot state machine.
7. **Every shell script is unexecuted**, and shellcheck was unavailable.

---

## Decisions you may want to reverse

| Decision | Why | How |
|---|---|---|
| Serial preferred over libusb by default | the only thing that works on macOS; on Linux it avoids `cdc_acm`, the interface claim and the ModemManager race | `Preference::default()` in `boottransport.rs` |
| `ktmac` shells out to `ktflash` for the write | one implementation of the protocol, not two | — |
| Reprobe leaves the journal at `reset-issued` on `NoDevice`/`StillInBootloader` rather than `Failed` | the write itself completed; the honest state is "reset issued, outcome unknown" | `cmd_flash_cdc.rs` |
| No containers anywhere | a static musl binary has no dependencies, and `unlock`'s re-enumeration makes `--device` passthrough actively wrong | — |
| No Swift test target | neither XCTest nor swift-testing ships with CLT-only; checks live in `ktmac selftest` so they actually run | add Xcode, port mechanically |
| Go shells out to `ssh` rather than `x/crypto/ssh` | inherits the operator's SSH config, agent and jump hosts; zero third-party crypto | — |

---

## Not done, and why

| Blocked on | Item |
|---|---|
| A dongle | every I/O path in every language |
| A Mac with the dongle | the N0 check above — highest information per minute in this document |
| Linux VMs | the whole test matrix |
| `zig`, `cargo-deb`, `cargo-generate-rpm`, `shellcheck` | cross-compiling, packaging, shell lint |
| Xcode.app | a real Swift test target |
| An Apple Developer account | notarization — deferred by design, blocks nothing else |
| **Your decision** | 10 commits in git history carry a personal email as the author address. Removing it needs `git filter-repo` + a force-push to `main`, which would break every existing clone including the hardware agent's. Not done unilaterally — see the note at the end of the session summary. |

Ground rules kept throughout: destructive gates untouched (`--execute` + `--yes`, dry run by
default, `vmatrix` refuses P2/P3), no firmware images, `proto/` stays free of `rusb` and OS I/O,
and exactly one new dependency across the whole thing (`libc`, already transitive).
