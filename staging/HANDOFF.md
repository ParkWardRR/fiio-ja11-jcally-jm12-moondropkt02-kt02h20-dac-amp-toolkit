# HANDOFF — staging/

**To:** the agent picking this up · **From:** a no-hardware session, 2026-09-05
**Read order:** this file → [`README.md`](README.md) (what was verified) → [`APPLY.md`](APPLY.md) (integration steps)

---

## The one-paragraph version

Three plans were written ([`RELEASE-PLAN`](../docs/RELEASE-PLAN.md),
[`LINUX-TESTING`](../docs/LINUX-TESTING.md), [`MACOS-NATIVE`](../docs/MACOS-NATIVE.md)) and then
implemented as far as a machine with **no dongle** can take them. Rust, Swift, Go and C all
build and their checks pass. **Nothing has touched hardware.** Your job is to test it, fix what
is wrong, integrate what survives, and delete this directory.

---

## What you are inheriting

| Language | Where | What | Verified |
|---|---|---|---|
| **Rust** | `flasher-src/` | flash-cdc state machine lifted onto a trait, serial transport, transport selection, **operation journaling**, rewired command | `clippy -D warnings` clean, **93 tests pass** |
| **Swift** | `macos-native/ktmac/` | IOKit: USB enumeration, serial-port→USB resolution, HID unlock (E1/E2), self-tests | `swift build` clean, **13/13 selftest checks pass**, subcommands run |
| **Go** | `tools/vmatrix/` | SSH runner for the Linux VM matrix + report generator | `go vet`/`gofmt` clean, runs end-to-end against unreachable hosts |
| **C** | `macos-native/` | E3 `USBInterfaceOpenSeize` probe | compiles `-Wall -Wextra` clean, runs |
| **Shell** | `release/`, `linux-testing/` | release, install, host-facts, P0, P2 scripts | `bash -n` / `sh -n` only — **never executed** |

Language choice was driven by fit, not preference: Swift because IOKit from Swift is far less
painful than the C plug-in dance (the one exception, E3's `IOUSBLib`, stayed in C precisely
because it was already working); Go because a concurrent SSH fan-out with a markdown reporter
is miserable in bash; Rust because that is what the tool is.

---

## Two findings you should know before you start

### 1. macOS TCC blocks the HID unlock experiment before USB is even reached

Running the E1 probe with **no device attached** returns `0xe00002e2` = `kIOReturnNotPermitted`
from `IOHIDManagerOpen`. That is macOS TCC, not USB: since 10.15, opening an `IOHIDManager`
needs **Input Monitoring** consent for the calling app — for a CLI, the terminal.

```
System Settings > Privacy & Security > Input Monitoring > +   (add Terminal/iTerm)
```

Consequences: E1/E2 cannot be evaluated at all until this is granted (a "no devices found"
result beforehand means nothing), and if ktflash ships an `IOHIDManager`-based unlock, this
consent prompt becomes part of the macOS first-run experience. E3 goes via `IOUSBLib` and
probably avoids it, trading a consent prompt for a `sudo` requirement.

Both `ktmac` and the C probe detect this and print the fix rather than a hex code.

### 2. `flash-cdc` was writing flash with no recovery record at all

ROADMAP Appendix A says *"destructive writes ship **only** with recovery semantics."* That held
for `flash --apply`, which journals as it goes. It did **not** hold for `flash-cdc` — the path
that is proven on hardware and that the README tells users to run. An interrupted `flash-cdc`
left `ktflash recover` nothing to read.

`proto/ktcdc_journal.rs` fixes it, with the ordering discipline the journal API asks for:
`Erased` is recorded **before `KSTA` goes on the wire**, because KSTA triggers the erase and the
flash is gone whether or not we live to see the ACK. There is a test named after exactly that
(`ksta_being_sent_is_enough_to_mark_the_flash_erased`).

This is the change I would most want a second pair of eyes on.

---

## Where the risk actually is

Ranked. Everything below is untested against hardware; this is about which parts will bite.

1. **`serialtransport.rs` macOS port matching** — falls back to "any `/dev/cu.usbmodem*`" and
   refuses when there are several. **`ktmac`'s `SerialPortFinder.swift` is the correct
   implementation** (`IORegistryEntrySearchCFProperty` with `kIORegistryIterateParents`) and is
   meant to be ported into Rust, or shelled out to via `ktmac port`. Do not ship the fallback:
   guessing wrong aims a firmware write at the wrong device.
2. **termios binary safety** — the protocol contains `0x11`/`0x13` (XON/XOFF) and `0x0d`/`0x0a`.
   `cfmakeraw` plus explicit `IXON`/`ICRNL`/`OPOST` clears are in there, but this is exactly the
   kind of thing that silently corrupts one byte in a firmware image.
3. **The `tio::` ioctl constants** are hard-coded (`TIOCMBIS` = `0x5416` Linux, `0x8004746c`
   macOS) rather than taken from `libc`. Check them against your headers.
4. **`cargo-zigbuild` + vendored libusb** — the one unproven link in the release plan
   (§3.2). If it can't compile libusb's C, build natively on the VMs instead; that fallback is
   already wired into `release.sh` via `KT_SKIP_LINUX=1`.
5. **`bootdiag_live()`** in `cmd_flash_cdc.rs` is `#[allow(dead_code)]` and **not** a drop-in
   replacement — it *sends* `KTM`, which advances the one-shot state machine. Read APPLY.md
   step 5 before wiring it.
6. **Every shell script is unexecuted.** `shellcheck` was not available on the authoring machine.

---

## Suggested order of work

**No hardware needed (do these first):**

1. APPLY.md steps 1–4 — the Rust integration. Rehearsed end-to-end; expect clippy clean and
   93 tests. **Diff `ktcdc_driver.rs` against the original `main.rs:907-995` before trusting
   it** — it is meant to be behaviourally identical, and any difference is my bug.
2. `swift build && .build/debug/ktmac selftest` in `macos-native/ktmac/`.
3. `go build && ./vmatrix init` in `tools/vmatrix/`.
4. Run the shell scripts through `shellcheck` and actually execute `release.sh --dry-run`.

**Needs a Mac + dongle:**

5. `ktmac list`, `ktmac ports` — confirm the dongle enumerates and, after an OrbStack unlock,
   that `ktmac port` resolves `8888:cdc0` to a `/dev/cu.*` path. **This single check validates
   the whole macOS-native premise** (`MACOS-NATIVE.md` §5, N0). If no `cu.usbmodem*` appears for
   the bootloader, stop and rethink Stage B before writing more code.
6. `ktmac unlock --send` (after granting Input Monitoring). Then `--no-id-prefix`, then
   `--seize`, then the C E3 probe. Pass = the dongle re-enumerates as `8888:cdc0`.
7. Whatever the answer, **fix the docs**: `PROTOCOL.md` and `dongle-investigation.md`
   currently contradict each other about macOS `SetReport`, and E1 settles it.

**Needs the Linux VMs:**

8. `vmatrix init`, fill in the SSH targets, `vmatrix run -dist ...`.
9. **Before anything else, configure USB passthrough by bus/port, not VID:PID.** `unlock`
   re-enumerates the dongle under a new ID, so an ID-keyed hypervisor rule drops it at exactly
   the moment it matters — `unlock` looks like it succeeded and the bootloader never appears.
10. P2 and P3 by hand. `vmatrix` deliberately refuses to automate them.

---

## Decisions I made that you may want to reverse

| Decision | Why | How to reverse |
|---|---|---|
| Serial preferred over libusb by default | only thing that works on macOS; avoids `cdc_acm`, the interface claim and the ModemManager race on Linux | one line: `Preference::default()` in `boottransport.rs` |
| No containers anywhere | a static musl binary has no dependencies, and `unlock`'s re-enumeration makes `--device` passthrough actively wrong | — |
| `libc` direct instead of the `serialport` crate | already transitive, MIT/Apache, no new `cargo deny` question | — |
| No Swift test target | neither XCTest nor swift-testing exists with CLT-only; checks live in `ktmac selftest` so they actually run | add Xcode, port mechanically |
| Go shells out to `ssh` rather than using `x/crypto/ssh` | inherits the operator's SSH config/agent/jump hosts; zero third-party crypto | — |
| Journal writes every 16 packets | a file write per KB would slow the flash and add failure modes to the recovery mechanism | `JOURNAL_EVERY_PACKETS` |
| `Confirmed` is never recorded by the driver | it means "post-reset reprobe saw the expected identity", which the driver cannot observe | — |

---

## What I could not do, and what it would take

| Blocked on | Item |
|---|---|
| A dongle | every I/O path in every language |
| A Mac with the dongle | N0 (does `8888:cdc0` publish a `/dev/cu.*`?) — ten minutes, highest information per minute in this whole handoff |
| Linux VMs | the entire test matrix |
| `zig` + `cargo-zigbuild` | cross-compiling; not installed here |
| `cargo-deb` / `cargo-generate-rpm` | the packaging metadata is written but unvalidated |
| `shellcheck` | shell lint |
| Xcode.app | a real Swift test target |
| An Apple Developer account ($99/yr) | notarization — deferred by design, blocks nothing else |

---

## Ground rules kept throughout

- Destructive gates untouched: `--execute` + `--yes` still required, dry run still the default,
  no default container `CMD`, `vmatrix` refuses to automate P2/P3.
- No firmware images anywhere.
- `proto/` stays free of `rusb` and OS I/O — `ktcdc_driver.rs` and `ktcdc_journal.rs` respect it.
- New dependencies: exactly one (`libc`, already transitive). Swift and Go packages are
  dependency-free.
- Honest labels: nothing claims to work on hardware, because nothing has been tried on hardware.

Delete `staging/` once its contents live in `flasher/src/`, `scripts/`, `packaging/` and
`tools/`. It is a scratchpad, not a second source of truth.
