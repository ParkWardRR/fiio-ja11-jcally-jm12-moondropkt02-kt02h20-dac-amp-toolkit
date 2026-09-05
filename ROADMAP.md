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
> **The native write is done and proven on hardware** (`v1.1.0`, 2026‑09‑05): `ktflash flash-cdc`
> reflashed the stock `JA11_V2.2.bin` end‑to‑end over the CDC bootloader, entirely from a
> **Mac + OrbStack** — no Mac mini, no Windows, no vendor tool. Phases 0–3 are complete. The
> frontier now is **Phase 4 (Linux‑native release / binaries)** and **Phase 6 (more dongles)**.
> **Phase 5 (software backup) is closed as *not possible* on this silicon** — it needs hardware.

---

## The timeline

```text
Phase 0  Reverse-engineer to the bootloader ........................... ✅ done
Phase 1  Hardware-free protocol + safety core ........................ ✅ done
Phase 2  Reverse the CDC download protocol  ⭐ .................... ✅ done (was the blocker)
Phase 3  Native flash on hardware (writer)  ⭐ ................... ✅ done — PROVEN on hardware
Phase 4  Linux-native release (binaries, notarization) .............. 🚧 code in / release ⏳
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
4. ⏳ **Still to harden**: journal each stage before its destructive command and confirm success
   by a post‑reset reprobe (the `Journal` model exists; wire it into `flash-cdc`).

**Recovery reality:** the `KT_USB_BOOT` ROM survives an app‑flash, so a bad write can be redone
by re‑unlocking and reflashing a compatible image — but **only if you have that image**. A botched
cross‑flash with no working image = a dongle stuck in bootloader indefinitely. See Phase 5 for
why backup is impossible in software.

---

## Phase 4 — Linux‑native release 🚧 *(code in; release pending)*

Drop the OrbStack detour on Linux and ship binaries.

1. 🚧 **Native transport + udev**: `RusbBootloaderTransport` (done) +
   [`packaging/99-ktflash.rules`](packaging/99-ktflash.rules) + [`docs/LINUX.md`](docs/LINUX.md).
   ⏳ *Test `flash-cdc` on real Debian/Arch over `hidraw`/libusb (no OrbStack).*
2. ⏳ **Prebuilt binaries** (x86_64 + aarch64) via `cross-rs`/`cargo-zigbuild` + SHA‑256 checksums.
3. ⏳ **macOS notarization** (Apple Developer signing) — prerequisite for a Homebrew tap.
4. ✅ **Reproducibility**: `Cargo.lock` committed; toolchain/target recorded.

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
2. ⏳ **Compatibility matrix from real data** — see [Appendix C](#appendix-c--compatibility-data-model).
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

## Windows box VELOCE (recovery + RE source)

SSH: `alfa@172.16.6.157` (key auth). `C:\kt\` has: `JA11_Upgrade_Tool.exe`, `fw\JA11_V2.2.bin`,
and PowerShell probes (`chk.ps1`, `validate_boot.ps1`, `enter_probe_race.ps1`,
`serial_probe.ps1`, `hid_read.ps1`). GUI automation runs via `schtasks /run /it` (interactive
console session ID 1). Vendor UIA flash flow: `pushButton_scan` → `pushButton_File` → SendKeys
path + `{ENTER}` → `pushButton_UpGrade`; success shows in the STATUS BAR
(`"UPGRADE FIRMWARE SUCESS"`, sic). **The vendor tool reliably recovers a bricked app‑flash.**
Intermittent SSH `"system cannot find the path specified"` — just retry; prefer
`powershell -File script.ps1` over long inline `-Command`.
