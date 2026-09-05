# 🗺️ Roadmap

A **chronological** map of this toolkit — the phases in the order they happen, what each
delivers, and exactly where the frontier is today.

**Direction:** macOS + OrbStack first (✅ working) → Linux‑native next. **No Windows user path** —
Windows is only ever a reverse‑engineering source.

**Legend:** ✅ done · 🚧 in progress · ⏳ planned · ❌ not possible (see why) · ⭐ pivotal

> [!CAUTION]
> **There is no way to back up a KT02H20 dongle's firmware in software — and flashing can brick it.**
> `ktflash` cannot read firmware *off* the device (confirmed by full RE — Phase 5 below).
> If a cross‑flash goes wrong, the `KT_USB_BOOT` ROM survives an app‑flash so the dongle stays
> reflashable — but **you need a compatible image on hand to retry**. A botched cross‑flash with
> no working image leaves it bootloader‑only indefinitely. **Save a known‑good image first, and
> proceed at your own risk.**

> ## 📍 You are here
>
> **The native write is proven on hardware on *three* independent paths now**, all 2026‑09‑05:
> **macOS + OrbStack** (`v1.1.0`, raw libusb); **Linux‑native over the serial transport**
> (`v1.1.1`+, a Debian 13 VM reached over `usbipd-win` from a Windows host, no OrbStack); and
> **macOS‑native with zero OrbStack** — `ktmac unlock` (Swift, `IOHIDManager`) plus `ktflash
> flash-cdc --transport serial`, on the same Mac used for this session. That last one **overturned
> this project's own long-standing assumption** that macOS couldn't drive the unlock at all — see
> [`docs/MACOS-NATIVE.md`](docs/MACOS-NATIVE.md) and Appendix D. Phases 0–4 are complete. The
> frontier now is **merging `ktmac` into `ktflash`** (still two binaries today), **prebuilt
> binaries**, and **Phase 6 (more dongles)**. **Phase 5 (software backup) is closed as *not
> possible* on this silicon** — it needs hardware.

---

## The timeline

```text
Phase 0  Reverse-engineer to the bootloader ........................... ✅ done
Phase 1  Hardware-free protocol + safety core ........................ ✅ done
Phase 2  Reverse the CDC download protocol  ⭐ .................... ✅ done (was the blocker)
Phase 3  Native flash on hardware (writer)  ⭐ ................... ✅ done — PROVEN on hardware
Phase 4  Linux + macOS native release (binaries, notarization) ...... ✅ write proven, both OSes · binaries ⏳
Phase 5  Firmware backup / readback ................................. ❌ not possible in software
Phase 6  Fleet: JM12, compatibility matrix, dongle discovery ........ ⏳ needs evidence
```

Historical milestone tags (`M0`, `M1`, …) are kept in parentheses for continuity.

---

## Phase 0 — Reverse‑engineer to the bootloader ✅

1. ✅ Recovered the normal‑mode HID protocol (`KT_USB_BOOT`) from the vendor tool with rizin +
   Ghidra → [`docs/EXTRACTION.md`](docs/EXTRACTION.md), [`docs/PROTOCOL.md`](docs/PROTOCOL.md).
2. ✅ Confirmed a real cross‑flash: Moondrop KT02H20 (`31b2:0111`) → FiiO JA11 (`2972:0102`).
3. ✅ `ktflash` Rust tool (`rusb`, single binary): `probe`/`handshake`/`unlock`/`bootdiag`.
4. ✅ Live ratatui TUI + `demo` mode.
5. ✅ Device classification (JA11 `2972:0102`, stock `31b2:0111`, bootloader `8888:cdc0`).
6. ✅ macOS → OrbStack driver to reach the HID endpoints macOS blocks.
7. ✅ Identified the core as **Andes NDS32** (no DSP) — [`docs/NDS32-CORE-ID.md`](docs/NDS32-CORE-ID.md).

---

## Phase 1 — Hardware‑free protocol + safety core ✅

Everything testable without a dongle, in [`flasher/src/proto/`](flasher/src/proto/) (no `rusb`/OS):
capture/replay format, the `cdc` `Session`/`FrameCodec`/`FakeBootloader` scaffolding, the
`KT_Helios` image parser, the plan/apply safety gate, device fingerprint + firmware manifest,
the operation journal, and local CI ([`flasher/ci.sh`](flasher/ci.sh), clippy `-D warnings` +
tests + replay smoke, run by [`scripts/hooks/pre-push`](scripts/hooks/pre-push)).

---

## Phase 2 — Reverse the CDC download protocol ⭐ ✅ *(was the blocker)*

The `8888:cdc0` bootloader download protocol is **fully reverse‑engineered, byte‑exact**, from
the vendor tool (Ghidra) — no USB capture needed in the end. Full writeup:
[`research/cdc-re-findings.md`](research/cdc-re-findings.md), spec in
[`docs/CDC-PROTOCOL.md`](docs/CDC-PROTOCOL.md).

1. ✅ **6‑byte data‑packet header** bit‑packing (`FUN_0054d7d0` / `FUN_005733a0`).
2. ✅ **KT CRC‑32** variant — poly `0xEDB88320`, **init=0 / xorout=0**, over **header+payload**
   (corrected a prior "payload‑only" note).
3. ✅ **State machine + tokens**: `KTM → CHP → erase → PWO → KSTA → data×N → STP → RESET`;
   `RESET`=`ZRST` (`5a 52 53 54`); `flag` (write base / handshake path) derived from image byte `0x0F`.
4. ✅ **`proto::ktcdc`** implements it, unit‑tested (packet math, CRC, planner) — 70 tests, CI green.

---

## Phase 3 — Native flash on hardware ⭐ ✅ *PROVEN 2026‑09‑05*

1. ✅ **`ktflash flash-cdc --image <fw.bin> [--flag 0|1] [--base 0xADDR] [--execute --yes]`** —
   dry‑run by default; `--execute` walks the full state machine over the CDC bulk pipe.
2. ✅ **Hardware‑confirmed**: a full reflash of the stock `JA11_V2.2.bin` (67 packets, all ACKed)
   from a Mac + OrbStack; the device re‑enumerated to `2972:0102` as a working JA11.
3. ✅ **Safety**: refuses to write without `--yes`, prints a loud back‑up‑first warning, and
   auto‑derives `flag` from the image so you can't pick the wrong write base by hand.
4. ✅ **Hardened, hardware‑confirmed 2026‑09‑05:**
   - **Journal before the destructive command.** [`proto/ktcdc_journal.rs`](flasher/src/proto/ktcdc_journal.rs)
     drives a `Journal` from the writer's events, recording `Erased` **before `KSTA` goes on the
     wire** — once KSTA is sent the flash is gone whether or not we live to see the ACK, so
     journalling on the success edge would tell a crashed operator that nothing destructive
     happened.
   - **Post‑reset reprobe.** [`proto/postflash.rs`](flasher/src/proto/postflash.rs) polls the bus
     after `RESET` and records `Confirmed` / `IdentityMismatch`, so `ktflash recover` can finally
     say *done*. Conservative by design: an ISP‑mode device anywhere on the bus is never a
     success, and a mismatch is only ever reported against an explicit `--expect VID:PID`
     (`IdentityMismatch` is a halt state — inferring the expectation would manufacture alarm
     from a guess). **Confirmed on real hardware**: `flash-cdc --expect 2972:0102 --execute --yes`
     printed `[reprobe] ✅ 2972:0102 — Flash confirmed`, and `ktflash recover` reported
     `last recorded stage: Confirmed` / `safe next action: Done` — closing the loop this phase's
     goal named.

**Recovery reality:** the `KT_USB_BOOT` ROM survives an app‑flash, so a bad write can be redone
by re‑unlocking and reflashing a compatible image — but **only if you have that image**. A botched
cross‑flash with no working image = a dongle stuck in bootloader indefinitely. See Phase 5 for
why backup is impossible in software.

---

## Phase 4 — Linux + macOS native release ✅ *write PROVEN on both, 2026‑09‑05* · binaries ⏳

Drop the OrbStack detour on Linux — **done for the write path**; binaries/notarization remain.

1. ✅ **Native transport + udev, hardware‑confirmed**: the serial transport
   ([`flasher/src/serialtransport.rs`](flasher/src/serialtransport.rs), previously "untested
   against hardware") drove a **complete `flash-cdc --execute` write** — 67/67 packets ACKed,
   erase, `STP`, `RESET` all clean — over `/dev/ttyACM0` on a **Debian 13** VM, no `sudo`, no
   OrbStack, no libusb claim. [`packaging/99-ktflash.rules`](packaging/99-ktflash.rules) needed a
   real fix first: see finding below. Full session narrative → Appendix D.
2. ⏳ **Prebuilt binaries** (x86_64 + aarch64) via `cross-rs`/`cargo-zigbuild` + SHA‑256 checksums.
3. ⏳ **macOS notarization** (Apple Developer signing) — prerequisite for a Homebrew tap.
4. ✅ **Reproducibility**: `Cargo.lock` committed; toolchain/target recorded.

**Finding — `uaccess` is a no‑op on headless Linux, `plugdev` is not optional:** the shipped
`99-ktflash.rules` relied on systemd‑logind `uaccess` (seat‑local access) with the `GROUP=plugdev`
fallback commented out. On a real headless VM (SSH‑only session, no logind seat —
`loginctl show-session … -p Seat` prints empty), `uaccess` grants nothing: `ktflash probe` still
needed `sudo` to read manufacturer/product/serial strings. Adding `GROUP="plugdev", MODE="0660"`
(now uncommented by default) fixed it immediately, with no downside on desktop systems where
`uaccess` still applies on top. **Ship both, always** — this was an open question in
[`LINUX-TESTING.md`](docs/LINUX-TESTING.md) §3 P1; it's now answered from real hardware, not
just predicted.

**Finding — AlmaLinux/RHEL cannot be a USB/IP client, by Red Hat's own choice:** attempting the
same validation on AlmaLinux 10.2 hit a hard wall *before* `ktflash` was even involved: RHEL's
`kernel-devel` source tree ships `drivers/usb/usbip/{Kconfig,Makefile}` with the actual `.c`
driver source **removed**, and no `vhci-hcd` module anywhere in `kernel-modules-extra`. This is a
limitation of *this specific test rig* (dongle reached over `usbipd-win`, which needs `vhci-hcd`
on the client) — a real RHEL box with the dongle plugged in directly would be unaffected, since
`ktflash` itself has no USB/IP dependency. Recorded here so nobody re-discovers this the hard way;
Debian's result stands as the Linux‑native proof for this round.

---

## Phase 5 — Firmware backup / readback ❌ *not possible in software*

**Closed as infeasible on this silicon** (full RE — [`research/cdc-re-findings.md`](research/cdc-re-findings.md) §5,
[`docs/CDC-PROTOCOL.md`](docs/CDC-PROTOCOL.md)):

- The **CDC bootloader has no read/dump command** — its state machine sends only the 10 known
  tokens. `INF` returns only a whole‑image **(size, CRC‑32)** fingerprint (flag=1, state‑gated),
  never flash contents.
- The **normal‑mode `0x08` word‑read** exists in the vendor exe (`FUN_0054d170`) but is **inert
  on this chip**: the runtime `0xFF01` HID collection is an audio/EQ dispatcher (`W/R/S/C`;
  `R` reads *EQ coefficients*, not flash), with no `0x08` case — so `0x33` times out and reads
  return zeros. There is no ISP‑enter that keeps the device in normal mode with reads on.
- The only reliable live reads are `CHP` (chip‑ID) and the `INF` (size, CRC‑32) fingerprint.

**⇒ A true backup requires hardware access** to the resident `KT_USB_BOOT` ROM / lower‑flash
region (code below load `0x80000`, absent from the app image): **JTAG/SWD, chip‑off, glitching,
or a KTMicro factory ISP tool.** None is reachable through any USB command.

**What this means for users:** keep a working firmware image **before** you flash — it cannot be
recovered off the device afterward. Flash at your own risk.

*Remaining (hardware‑only, ⏳):* document the JTAG/SWD pads if found on a PCB; a `verify` helper
that reads the installed `(size, CRC‑32)` fingerprint to confirm what's on a device.

---

## Phase 6 — Fleet: JM12, matrix, discovery ⏳

Grow beyond the JA11, evidence‑first.

1. ⏳ **JCALLY JM12 first‑class**: fingerprint (`probe`/`fingerprint`), a documented flash + a
   one‑command revert (bring your own stock image — no dump path exists).
2. ⏳ **Compatibility matrix from real data** — see [Appendix B](#appendix-b--compatibility-data-model).
3. ⏳ **Dongle discovery**: source & fingerprint more KT02H20‑based dongles (Fransun T2 Pro,
   Audiocular A16x, VE ODO, KZ/CCA…); a `probe`‑driven "is this a KT02H20 dongle?" flow.

---

## 💡 Ideas / nice‑to‑have

- **EQ / PEQ tuning** over the `0xFF01` control channel (the *app* protocol `W/R/S/C`) from the TUI.
- **In‑TUI flash** with a hard confirmation (currently the TUI hands you the CLI command via `f`).
- **Auto‑recovery**: detect a stuck bootloader and offer to reflash the last known image.

---

## 🙌 How to help

- **Test `flash-cdc` on Linux‑native** (no OrbStack) and report back — the last thing gating Phase 4.
- **Before/after USB descriptors + structured evidence** from any dongle you flash (Phase 6).
- **PCB photos / JTAG‑SWD pad locations** — the only route to a real firmware backup (Phase 5).
- **A JCALLY JM12** (or other KT02H20 dongle) with its original firmware image.

Build/test with [`flasher/ci.sh`](flasher/ci.sh). Open an issue or PR.

---
---

# Appendix A — the safety model

Destructive writes ship **only** with recovery semantics, and `flash-cdc` refuses to write
without `--yes`. `KT_Helios` magic is necessary but *not sufficient* — a structurally valid image
can still be wrong for a board, and **there is no on‑device backup to fall back on**.

### The image gate (before any erase)

Implemented in [`proto/plan.rs`](flasher/src/proto/plan.rs); verdict is
`Refuse` / `NeedsConfirmation` (reject‑by‑default) / `Proceed`.

| Gate | Requirement |
|---|---|
| Header/layout parse | Full `KT_Helios` structure, not just the magic |
| Size constraints | Declared size matches the file; reject truncation/padding |
| Cryptographic identity | SHA‑256 computed + logged |
| Firmware manifest | Image hash → allowed device family + risk (fails closed) |
| Device‑family match | Structured fingerprint, **not VID/PID alone** |
| Backup precondition | **User confirms they hold a working image (`--yes`)** — no auto‑backup exists |
| Post‑flash check | Post‑reset reprobe of the expected identity (⏳ wire into `flash-cdc`) |

### The operation journal + recovery‑state model

Every flash can write an append‑safe journal ([`proto/journal.rs`](flasher/src/proto/journal.rs));
`ktflash recover <journal.json>` maps the last recorded state to the safe next action. Because the
`KT_USB_BOOT` ROM survives an app‑flash, an interrupted/failed write is recoverable **by reflashing
a compatible image** — which you must already have.

---

# Appendix B — compatibility data model

`M6` is machine‑readable from day one, and keeps "technically flashes" separate from "safe to
recommend." Confidence ladder: `candidate` → `observed` → `verified`, plus `unsupported` /
`unsafe`. A device is **tested** only after structured evidence is submitted, and **recommended**
only after functional checks (audio, mic, gain, buttons, balance, suspend/resume, control channel)
pass. Note: `restore-verified` is **not attainable** without a firmware backup path (Phase 5), so
reverts rely on a user‑supplied stock image.

```yaml
device:
  marketing_name: JCALLY JM12
  usb_normal: { vid: "0x31b2", pid: "0x0111", bcd_device: "0x…",
                descriptor_sha256: "…", audio_descriptor_sha256: "…" }
  bootloader: { vid: "0x8888", pid: "0xcdc0" }
firmware: { filename: "…", sha256: "…", source_url: "…", redistribution: "…" }
result:
  flash_status: success; postflash_audio: pass; microphone: untested
  app_control_channel: pass; suspend_resume: untested; restore_tested: false
  risk_level: experimental
evidence:
  normal_before_descriptor: "…"; normal_after_descriptor: "…"
  bootloader_capture: "…"; operation_log: "…"; contributor_attestation: "…"
```

---

# Appendix C — tooling & references

| Tool / project | Role |
|---|---|
| Ghidra headless (`analyzeHeadless`) | Reproducible vendor‑binary analysis (how the protocol was reversed) |
| rizin / rizin projects | Fast disassembly + string/xref inspection |
| `rusb` / libusb | The USB transport in `ktflash` |
| OrbStack | Hands the dongle to a Linux guest so libusb can claim the interface on macOS |
| `cross-rs` / `cargo-zigbuild` | Linux x86_64/aarch64 builds (Phase 4) |
| local CI ([`flasher/ci.sh`](flasher/ci.sh)) | clippy + tests + replay, via pre‑push hook (no GitHub Actions) |

---

# Appendix D — Operational notes & hardware evidence

Preserved from the development handoff for continuity. This is not user‑facing guidance — it's
the raw record of what was confirmed, and the operational context for anyone picking this up.

## Proven hardware claims (do not re‑litigate)

| Claim | Evidence |
|---|---|
| Unlock (`0x54 "T12345678"`) → re‑enumerates as `8888:cdc0` CDC serial | Windows: `USB\VID_8888&PID_CDC0\KT_VIRTUAL_COM_PORT` → COM3 |
| `KTM` handshake token = `1e 4b 54 4d` | Fresh bootloader: `KTM` → reply `0x78` |
| `0x78` (`'x'`) is the accept/ACK byte | Handshake reply is exactly `0x78`; `KEY` → `78 00 78` |
| `KEY` token = `f0 4b 45 59` | ACKs `0x78` |
| Bootloader is a **one‑shot sequential** state machine | After `KTM` is consumed, repeat `KTM` → no reply until re‑unlock |
| Block checksum = standard CRC‑32, poly `0xEDB88320` | Table @ `0x0120e020` — first 8 words match canonical CRC‑32 (byte‑exact) |
| `flash-cdc` full reflash of `JA11_V2.2.bin` | 67 packets, all ACKed; device re‑enumerated to `2972:0102` — Mac + OrbStack, 2026‑09‑05 |
| **Serial transport (CDC‑ACM tty) drives the full protocol**, not just libusb bulk | `bootdiag --send` (KTM→`0x78`) *and* a complete `flash-cdc --execute` (67/67 packets, erase, `STP`, `RESET`) over `/dev/ttyACM0` — Debian 13 VM, 2026‑09‑05 (§ below) |
| A repeated `KTM` after the handshake is already consumed gets **zero bytes back**, not garbage | `flash-cdc` on an already‑progressed bootloader → `KTM: expected [78], got [] after 800ms` — clean, safe, non‑destructive failure; journal recorded `Staged`→`Failed`, `safe_next_action: CancelOrBegin` |
| **Native macOS `unlock` works** via `IOHIDManager` — the project's own long‑standing "macOS can't drive this" claim was wrong | `IOHIDDeviceSetReport` returns success and the device re‑enumerates as `8888:cdc0`, confirmed by `ioreg` and `/dev/cu.usbmodem101` appearing — `ktmac unlock --send`, 2026‑09‑05, no OrbStack |
| A complete `flash-cdc` write works fully macOS‑native (serial transport, no OrbStack, no libusb) | 67/67 packets ACKed, erase, `STP`, `RESET` clean; device re‑enumerated with the same descriptor SHA‑256 as before — 2026‑09‑05, same session as the Linux‑native proof |
| `ktmac flow` (the unified preflight→dry‑run→unlock→flash→reprobe orchestration, calling `ktflash` as a subprocess) works end‑to‑end on real hardware | Full run completed with `--execute --yes`; device came back as a working `2972:0102` JA11 — first hardware run of `Flow.swift`, previously "STATUS: UNTESTED against hardware" |
| The post‑reset reprobe (`proto::postflash`, ROADMAP Phase 3.4) correctly detects success | `flash-cdc --expect 2972:0102 --execute --yes` printed `[reprobe] ✅ 2972:0102 — Flash confirmed`; `ktflash recover` then reported `Confirmed` / `Done` instead of the old ceiling of `ResetIssued` / `WaitAndReprobe` |

## Operational gotchas

- **HID `0xFF01` collection is unreachable on macOS and Windows** — always use OrbStack/Linux
  `hidraw` for unlock, dump, and flash operations.
- **Bootloader does not idle‑timeout back to normal mode.** If it gets stuck mid‑sequence (the
  state machine is one‑shot and once‑dirtied it won't re‑accept tokens), recovery options are:
  physical unplug/replug, a full vendor‑tool reflash (resets to normal on success), or the
  `RESET`/`ZRST` command once wired into the CLI.
- **`orb usb attach` must be re‑run after every unlock** — the device changes USB ID from its
  normal VID/PID to `8888:cdc0` on reboot, and OrbStack won't follow it automatically.
- **Git guard:** commits and pushes require `GIT_ALLOW_REAL_NAME=1` on both operations, with
  the machine's configured `user.name`/`user.email` (`git config --global user.name/user.email`).

## 2026‑09‑05 — Linux‑native validation, session narrative (VELOCE Hyper‑V lab)

Recorded in full because it's the kind of session worth writing up later: what was tried, what
broke, and what the breakage taught us. The dongle spent this entire session plugged into a
**Windows 10 Pro** box (`VELOCE`) with two Hyper‑V Linux guests (Debian 13, AlmaLinux 10.2) on an
internal NAT switch — no OrbStack anywhere in this loop.

**Getting the dongle off Windows and into a guest.** Hyper‑V has no built‑in USB passthrough, so
the plan was Option B from [`LINUX-TESTING.md`](docs/LINUX-TESTING.md) §2.2: export it from
Windows. `usbipd-win` wasn't installed; `winget install dorssel.usbipd-win` put it on in under a
minute, and it auto‑created its own inbound firewall rule and started listening on `3240` with no
extra steps. Binding the dongle (`usbipd bind --busid 1-4`) needed `--force` — a leftover
`USBPcap` filter driver from the original protocol‑reversing session on this same machine
conflicts with usbipd's own filter. On the Debian 13 guest, `apt install usbip` + `modprobe
vhci-hcd` was all it took; `usbip attach -r <host-LabNAT-ip> -b 1-4` and the dongle showed up in
`lsusb` like any other USB device.

**The re‑enumeration trap, hit exactly as documented.** The first `ktflash unlock` rebooted the
dongle to `8888:cdc0` — and Windows immediately dropped the usbipd share, rebinding the device to
its native "USB Serial Device (COM3)" driver, exactly the failure mode §2.1 warns about. The fix
was exactly what the doc prescribes: `usbipd bind --busid 1-4 --force` again (same busid, new
VID:PID, needs re‑sharing every time), then `usbip attach` again on the guest. This needed to
happen after *every* unlock for the rest of the session — scripting it is the obvious next step
for anyone doing this repeatedly.

**The permissions question that was previously just a prediction.** `ktflash probe` ran fine
without `sudo` (device enumeration doesn't need it) but came back with **empty** manufacturer/
product/serial strings until `sudo` was added — reading string descriptors needs an opened device
handle, which needs udev to grant access. Installing the shipped `99-ktflash.rules` (uaccess‑only
at the time) changed nothing: `loginctl show-session … -p Seat` printed empty for the SSH session,
confirming there's no logind seat to grant uaccess *to*. Adding the commented‑out
`GROUP="plugdev"` line and reloading udev fixed it on the same running device, no replug needed.
This is now the shipped default (Phase 4 above) — a real, previously‑open question closed by
hitting it on real hardware rather than reasoning about it.

**First hardware proof the serial transport actually works.** Everything about
`serialtransport.rs` had been "compiles, never run" since the APPLY.md handoff. `ktflash bootdiag`
opened `/dev/ttyACM0` cleanly (auto‑selected over libusb, as designed); `bootdiag --send` got the
`KTM`→`0x78` handshake back over the tty. Small thing, but it was the first byte of protocol ever
exchanged over a CDC‑ACM tty in this project instead of raw USB bulk endpoints.

**A safe failure that proved the safety model, not just the happy path.** `bootdiag --send`
advances the one‑shot bootloader state machine — by design, so this was known going in. Running
`flash-cdc --execute --yes` right after, without a fresh `unlock`, produced exactly the failure
the docs predict: `KTM: expected [78], got [] after 800ms`, zero bytes back, nothing erased. The
journal recorded `Staged` → `Failed` and reported `safe_next_action: CancelOrBegin` — correctly
telling the operator nothing destructive happened and a plain retry is fine. This was the first
time that logic ran against real hardware in a genuinely wrong state, not a `FakeKt` unit test.

**Recovering needed a real power cycle, and the easy ways didn't work.** `Disable-PnpDevice`
failed with the same `HRESULT 0x80041001` WMI error seen elsewhere in this lab's history;
`pnputil /disable-device` / `/enable-device` both returned "This command is not supported on this
OS product" (a client‑SKU restriction). With no way to soft‑reset the port remotely, the operator
physically unplugged and replugged the dongle. It came back as `2972:0102`, still bound to
usbipd's share (bus/port didn't change) — one thing that *did* just work.

**The full, real write.** `unlock` → rebind → reattach → `flash-cdc --image JA11_V2.2.bin
--execute --yes`: `KTM`, `CHP` (13‑byte chip‑info blob), `ERASE`, `PWO`, `KSTA` (erase), all 67
data packets ACKed (`78 a5` each), `STP`, `RESET` — all clean, first attempt after the fresh
unlock. The device re‑enumerated as `2972:0102`, and `ktflash fingerprint` reported the **same
descriptor SHA‑256** as before the flash. This is the first hardware run of the *refactored*
driver (`proto::ktcdc_driver` + `proto::ktcdc_journal`, previously validated only against a fake
bootloader) and the first flash of any kind over serial + USB/IP rather than raw libusb + OrbStack.

**Where AlmaLinux stopped it cold.** Installing build deps was normal EL packaging (`dnf`, EPEL
for nothing usbip‑related — EPEL doesn't have it either). The dead end was structural: RHEL's own
`kernel-devel` source tree ships `drivers/usb/usbip/` with only `Kconfig` and `Makefile` — **the
driver's `.c` source is not there**, and `vhci-hcd` isn't in `kernel-modules-extra` either. This
is Red Hat choosing not to ship USB/IP client support, not a packaging gap to route around with
`dnf search` harder. It blocks *this rig* (dongle reached via `usbipd-win`), not `ktflash` itself
— a real RHEL machine with the dongle plugged in directly would never hit this. Decision: leave
Alma's hardware validation for whenever the dongle can be plugged into a native Linux box.

## 2026‑09‑05 (same day) — macOS‑native validation, session narrative

The dongle moved from the Windows lab straight to the Mac used for this whole project. What
follows overturns a claim this project had carried since its very first commit.

**The premise everyone had been working from.** `docs/PROTOCOL.md` stated flatly that
`IOHIDDeviceSetReport` "goes down the control pipe the firmware ignores" on macOS, and
`docs/MACOS-NATIVE.md` itself — the plan document for exactly this investigation — noted that a
*different* file, `docs/dongle-investigation.md`, disagreed: it had found `SetReport` working
natively via `IOHIDManager` on a different dongle (a KZ C04). The two docs contradicted each
other, and the project had built its entire macOS story (OrbStack, for everything) on the
pessimistic one without ever running the fifteen‑minute experiment that would settle it.

**The blocker turned out to be a permission dialog, not a protocol limitation.** Building
`staging/macos-native/ktmac` (Swift, `swift build`, clean) and running `selftest` passed all 13
hardware‑free checks immediately — IOKit USB enumeration needs no special permission at all.
`ktmac list` correctly identified the dongle natively. But `ktmac unlock` (dry run) failed with
`IOHIDManagerOpen: kIOReturnNotPermitted` — macOS TCC (Input Monitoring), not a USB error, and
`ktmac` prints exactly that distinction instead of a bare hex code. Nimbalyst (the app hosting
this session's terminal) needed Input Monitoring granted in System Settings, then a full
quit‑and‑relaunch — a background reload isn't enough, TCC checks the running process's
entitlement at launch.

**Once granted, the dry run worked immediately** — `IOHIDManagerOpen` succeeded, found the
target device, and printed the exact report it would send (`report id=0x54 bytes=[54 31 32 33
34 35 36 37 38 00]`). Then the real experiment: `ktmac unlock --send`. `IOHIDDeviceSetReport`
returned success, and five seconds later: `PASS (bootloader appeared)`. Independently confirmed
via `ioreg` (idVendor `34952`/`0x8888`, idProduct `52672`/`0xcdc0`) and by macOS publishing
`/dev/cu.usbmodem101` for it — not just an API return code, an actual re‑enumerated device.

**The second half fell into place without any new code.** `ktflash bootdiag` (the existing Rust
binary, built earlier this session for the Linux‑native work) auto‑selected the serial transport
and opened `/dev/cu.usbmodem101` cleanly — no flags, no OrbStack, no libusb, no sudo. A dry‑run
`flash-cdc` against the same `JA11_V2.2.bin` used throughout this session printed an identical
plan to every other platform. Then, with explicit go‑ahead: `ktflash flash-cdc --transport
serial --image JA11_V2.2.bin --execute --yes` — `KTM`, `CHP` (13‑byte info blob), `ERASE`, `PWO`,
`KSTA` (erase), all 67 data packets ACKed, `STP`, `RESET`, all clean, first attempt, no power‑cycle
needed this time (the bootloader was still fresh — nothing had touched it since `unlock`). The
device re‑enumerated as `2972:0102`, and `ktflash fingerprint` reported the **same descriptor
SHA‑256** as every prior check this session.

**What this actually means, precisely stated.** `ktflash unlock` (the Rust binary, via `rusb`)
still cannot claim the interface on macOS — that half of the original claim was correct, and
remains true. What was wrong was the *conclusion drawn from it*: that macOS categorically
couldn't do the unlock. A different API (`IOHIDManager`, not libusb) reaches the same device
without claiming anything, because HID and interrupt‑pipe access don't require the interface
claim that blocks `rusb`. The full pipeline is proven and OrbStack‑free — but it is **two
binaries** today (`ktmac` for unlock, `ktflash` for everything else), not one. Merging
`ktmac`'s IOKit calls into `ktflash` itself is open work, tracked in
[`docs/MACOS-NATIVE.md`](docs/MACOS-NATIVE.md) §8.

## Windows box VELOCE (recovery + RE source)

SSH: `alfa@172.16.6.157` (key auth). `C:\kt\` has: `JA11_Upgrade_Tool.exe`, `fw\JA11_V2.2.bin`,
and PowerShell probes (`chk.ps1`, `validate_boot.ps1`, `enter_probe_race.ps1`,
`serial_probe.ps1`, `hid_read.ps1`). GUI automation runs via `schtasks /run /it` (interactive
console session ID 1). Vendor UIA flash flow: `pushButton_scan` → `pushButton_File` → SendKeys
path + `{ENTER}` → `pushButton_UpGrade`; success shows in the STATUS BAR
(`"UPGRADE FIRMWARE SUCESS"`, sic). **The vendor tool reliably recovers a bricked app‑flash.**
Intermittent SSH `"system cannot find the path specified"` — just retry; prefer
`powershell -File script.ps1` over long inline `-Command`.
