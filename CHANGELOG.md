# Changelog

## [1.2.0] — 2026‑09‑05 — first release: native macOS + Linux, no OrbStack required

The headline: this project's own long‑standing claim that macOS couldn't drive the unlock at
all was wrong. This release is fully native on both macOS and Linux as a result — plus the
Linux‑native write path, the `ktmac` merge, the post‑reset reprobe, and the first prebuilt
binaries. This is the first tagged release.

### Added
- **Linux‑native write, proven on hardware**: a full `flash-cdc --execute --yes` write completed
  on a real Debian 13 VM over the serial transport — 67/67 packets ACKed, device re‑enumerated
  as a working JA11 with an identical descriptor SHA‑256 to before. First hardware run of the
  refactored driver/journal code.
- **`ktflash bootdiag --send`** — an explicit, advancing liveness check (sends `KTM`, waits for
  the `0x78` ACK) alongside the default non‑advancing `bootdiag`. Both now work over
  `--transport auto|serial|usb`.
- **`proto::ktcdc_driver` / `proto::ktcdc_journal`** — the flash-cdc state machine and its
  operation journal, lifted out of `main.rs` onto the `Transport` trait so they're unit-tested
  independent of `rusb`.
- **`serialtransport.rs` / `boottransport.rs`** — a CDC-ACM tty transport (serial, preferred by
  default) alongside the existing libusb path, so Linux and macOS can both drive the bootloader
  without claiming a USB interface.
- **macOS‑native write, proven on hardware, same day**: `IOHIDManagerOpen`/`IOHIDDeviceSetReport`
  via the `ktmac` Swift companion reaches the device natively for `unlock` (the only prerequisite
  is Input Monitoring consent, macOS TCC) — confirmed with a complete `flash-cdc --transport
  serial` write, 67/67 packets, no OrbStack involved.
- **`flasher/src/macos_ktmac.rs`** — `ktflash unlock` (and the TUI's unlock action) now
  auto-detects a `ktmac` binary on macOS (`KTMAC_PATH` env, next to the running executable, or
  `$PATH`) and shells out to it, instead of requiring a separate manual `ktmac unlock --send`
  step. Falls back to the previous `rusb` attempt (and its OrbStack pointer) if `ktmac` isn't
  built. Confirmed on real hardware: `ktflash unlock` with no flags triggered the bootloader via
  `ktmac`, and `ktflash flash-cdc` completed the write on the same run.
- **Post-reset reprobe** (`proto::postflash`, ROADMAP Phase 3.4): after `RESET`, `flash-cdc` now
  polls the bus and records `Confirmed`/`IdentityMismatch`, so `ktflash recover` can finally say
  *done* — previously a journal topped out at `ResetIssued` and a flash that reset into a
  non-booting image looked identical to one that worked. New flags: `--expect VID:PID`,
  `--no-reprobe`, `--reprobe-timeout`. Confirmed on hardware: `[reprobe] ✅ 2972:0102 — Flash
  confirmed`, `ktflash recover` reporting `Confirmed`/`Done`.
- **Prebuilt binaries, built and signed**: macOS universal binary (`aarch64`+`x86_64` via
  `lipo`, ad-hoc signed after lipo), `x86_64`/`aarch64` `unknown-linux-musl` static binaries via
  `cargo-zigbuild --features vendored` (confirmed static with `file`), `.deb` (`cargo-deb`) and
  `.rpm` (`cargo-generate-rpm`), `SHA256SUMS` + a real minisign signature (`packaging/ktflash.pub`
  committed; secret key held in a password manager, never in this repo).

### Fixed
- **`packaging/99-ktflash.rules`**: `uaccess`-only default is a no-op on headless Linux (no
  logind seat, confirmed on a real headless VM) — `GROUP="plugdev"` is now uncommented by
  default alongside it.
- **`serialtransport.rs`**: `libc::ioctl`'s request-parameter type differs between glibc
  (`c_ulong`) and musl (`c_int`) — a real bug that only surfaced by actually cross-compiling to
  `x86_64-unknown-linux-musl`, since every prior Linux build/test ran natively against glibc.
  Fixed with `as _` at the call site so the cast targets whichever type the platform declares.
- **`flasher/Cargo.toml`**: `cargo-generate-rpm` v0.21.0's actual schema for
  `post_install_script` is a plain string (optionally a path to a script file), not the
  `{ program, script }` table that had been drafted by analogy with other formats. Also added
  `auto-req = "disabled"` — `requires = {}` alone doesn't stop the tool from shelling out to
  `ldd` for dependency auto-detection, which fails outright on a host with no `ldd`.
  Both corrected `docs/PROTOCOL.md`, `docs/FLASHING.md`, `usbtransport.rs`, and
  `orbstack/README.md` (OrbStack demoted to a documented fallback, not the only path).
- **`flasher/Cargo.toml`**: the package `description` still said "from macOS + OrbStack" as the
  only path, surfaced by actually reading the built `.deb`'s control file.
- **`flasher/src/tui.rs`** and **`main.rs`**'s HELP text/module docs: several places still
  described `unlock`/`flash-cdc` as OrbStack-only, contradicted by the native proof above.
- Fixed a pre-existing mislabeled cross-reference in `ROADMAP.md` (a link read "Appendix C" but
  pointed at Appendix B's anchor).

### Removed
- **AlmaLinux/RHEL hardware validation is no longer a project goal.** RHEL's kernel packaging
  deliberately excludes the `vhci-hcd` USB/IP client driver, so a RHEL/Alma guest can never be
  reached over this project's `usbipd-win`-based remote test rig. The `.rpm` package and static
  musl binary still ship for Alma/RHEL — only the "validate it over this remote rig" goal is
  dropped. Documented in the `.rpm` control metadata not applying here — see README and
  `ROADMAP.md` Appendix D for the full finding.

### Documented
- A hard AlmaLinux/RHEL limitation: `kernel-devel` ships no `vhci-hcd` driver source, so
  RHEL-family guests can't be USB/IP clients at all (blocks this test rig, not `ktflash`).
- `docs/LINUX.md` rewritten: package install first, `ModemManager` note, current `flash-cdc`
  usage (it previously described an older, unused `flash --apply` path as "once the codec
  lands," long after the codec had in fact landed).
- README Quickstart rewritten to lead with downloading a release, and a new "macOS Gatekeeper:
  fixing 'cannot be opened'" section explaining exactly when that warning appears (browser
  downloads only, never `curl`) and how to clear it without disabling Gatekeeper system-wide.

## [1.1.1] — 2026‑09‑05

### Changed
- **README + ROADMAP overhauled** to match reality: native write is done and hardware‑proven,
  and firmware backup is documented as **not possible in software** (hardware‑only).
- **Prominent "no backup / can brick / at your own risk" warnings** across the README, ROADMAP,
  `docs/FLASHING.md`, the TUI (startup banner + device panel + the `f` flash guide), and the
  `flash-cdc` write gate.
- **`orbstack/ktflash-orbstack.sh`**: added a guarded `flash <fw.bin>` command (dry‑run →
  confirm → unlock → write) and refreshed its help/notes.

### Fixed (docs/cleanup)
- Purged stale "download protocol not reversed yet / WIP native write" language from
  `docs/FLASHING.md`, `docs/LINUX.md`, `docs/PROTOCOL.md`, `docs/REVERT.md`, `proto/cdc.rs`,
  `proto/mod.rs`, and the `bootdiag`/`flash --apply` messages — all now point to the reversed
  framing in `proto::ktcdc` / the `flash-cdc` command.
- `ktflash dump` is relabelled a raw `0x08` diagnostic that does **not** read flash (and is
  **not** a backup), matching the hardware/RE finding; `docs/REVERT.md` now states plainly that
  a stock image can only come from the vendor package or a hardware dump.

## [1.1.0] — 2026‑09‑05

### Added — native CDC write, proven on hardware (2026‑09‑05)
- **`proto::ktcdc`** — the byte‑exact CDC bootloader download framing (header bit‑packing,
  KT CRC‑32 `init=0`/`xorout=0` over header+payload, packet planner), decompiled from the
  vendor tool and unit‑tested (6 tests; 70 total, CI green).
- **`ktflash flash-cdc --image <fw.bin> [--flag 0|1] [--base 0xADDR] [--execute]`** — the
  native CDC bootloader writer. Dry‑run by default; `--execute` walks the full state machine
  (`KTM → CHP → erase → PWO → KSTA → data×N → STP → RESET`), auto‑deriving `flag` from
  image byte `0x0F`. **Confirmed by a full reflash of the stock `JA11_V2.2.bin`** over the
  bootloader from a Mac + OrbStack — device re‑enumerated to `2972:0102` as a working JA11.
- Findings: `RESET`=`ZRST` (`5a 52 53 54`), `CHP` returns a 13‑byte `KT02H20B` info blob
  live; the normal‑mode `0x08` word‑read does **not** reach flash (Task B backup disproven).
- Docs: corrected `docs/CDC-PROTOCOL.md` (CRC scope + header layout + hardware‑validation table).

### Added — the dongle‑free foundation
Everything *around* the (now‑reversed) CDC write protocol, built and tested with **no
hardware** (unit tests, local CI). See [`ROADMAP.md`](ROADMAP.md).
- **Transport‑independent protocol core** (`flasher/src/proto/`, no `rusb`/OS deps):
  - `image` — `KT_Helios` firmware parser/validator → `ktflash image <fw.bin>`.
  - `frame` — the known normal‑mode `0x4B`/`0x54` HID frames.
  - `transcript` — M0 normalized capture format + a `tshark` decoder → `ktflash bootdiag
    --replay <t.json>` (validates captures with no hardware); fixtures in `flasher/fixtures/`.
  - `cdc` — typed messages, a `Transport` trait, a `FrameCodec` seam (`PendingCodec` = the
    still‑unreversed real framing; `ReferenceCodec` = a test stand‑in), a `Session` state
    machine, and a `FakeBootloader` emulator.
- **Safety layer (M2):** `plan` (image gate + SHA‑256 + reject‑by‑default `Decision`),
  `fingerprint` + `manifest` (fail‑closed device‑family allow‑list; VID/PID alone never
  suffices), and `journal` (durable operation journal + recovery‑state model).
  - `ktflash flash --plan <fw> [--device VID:PID] [--manifest m.json]` and
    `flash --apply <plan.json> [--execute] [--force-unsupported-device VID:PID]`.
  - `ktflash recover <journal.json>` — the safe next action for an interrupted flash.
  - `ktflash fingerprint` — structured device identity (JSON).
- **Native transport (M3):** `usbtransport::RusbBootloaderTransport` over the bootloader bulk
  endpoints; `packaging/99-ktflash.rules` (udev) + `docs/LINUX.md`.
- **Readback spike (M4a):** `cdc::ReadFlash`/`Data` + `Session::read_region`; `ktflash dump
  --addr --len` via the known `0x08` word‑read (feasibility probe, **not** a proven backup).
- **Compatibility matrix (M6):** `compat` records with a confidence ladder + evidence checks →
  `ktflash compat --template | --validate <matrix.json>`.
- **Local CI:** `flasher/ci.sh` (clippy `-D warnings`, tests, replay smoke, + `cargo
  audit`/`cargo deny` when installed) run via `scripts/hooks/pre-push`; `flasher/deny.toml`;
  `docs/RELEASING.md`. **No GitHub Actions.**

### Notes
- The one hardware‑gated gap is the CDC download wire framing; capture it (roadmap Phase 2) and
  implement `KtCdcCodec` behind the existing `FrameCodec` seam to light up native flash/backup.

## [1.0.0] — 2026-09-04

First production release. **macOS‑first (via OrbStack), no Windows path.**

### Added
- **`ktflash`** (renamed from `kt02h20-flasher`) — a Rust `rusb` tool with a **beautiful live
  TUI** (`ratatui`) as the default, plus `probe` / `handshake` / `unlock` / `bootdiag` and a
  `demo` mode (the simulated flash journey shown in the README GIF).
- **Device classification** — now recognises the flashed **FiiO JA11 `2972:0102`**, stock
  **KTMicro `31B2:0111`**, other `31B2:*` dongles, and the **`8888:CDC0`** bootloader.
- **`assets/demo.gif`** + its VHS `demo.tape` — reproducible README demo.

### Changed
- **Removed the Windows path.** Windows was only ever the reverse‑engineering *source*
  (the vendor tool we disassembled); it is not a user‑facing flow. The project is
  **macOS + OrbStack**, with native Linux next. (FiiO ships their own Windows updater.)
- Docs reframed around macOS + OrbStack; `FLASHING.md` rewritten accordingly.
- `dongle-investigation.md` de‑duplicated and cleaned (TTGK‑in‑house correction kept, JieLi
  theory retracted, stray citation markers removed).

### Status
- ✅ Protocol reversed; cross‑flash confirmed (Moondrop KT02H20 → JA11).
- 🚧 End‑to‑end native write pending the CDC bootloader download protocol.

## [0.2.0] — 2026-09-04
- Rust `rusb` CLI (probe/handshake/unlock/bootdiag); OrbStack driver; docs
  (EXTRACTION/FLASHING/COMPATIBILITY/REVERT); reversed `KT_USB_BOOT` protocol; confirmed the
  KT02H20 → JA11 flash. _(This release also shipped a since‑removed Windows automation used
  during reverse engineering.)_

## [0.1.0]
- Docs‑first: initial `PROTOCOL.md` + dongle investigation.
