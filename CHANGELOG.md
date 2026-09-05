# Changelog

## [Unreleased] — Linux‑native flash proven on hardware — 2026‑09‑05

### Added
- **`ktflash bootdiag --send`** — an explicit, advancing liveness check (sends `KTM`, waits for
  the `0x78` ACK) alongside the default non‑advancing `bootdiag`. Both now work over `--transport
  auto|serial|usb`.
- **`proto::ktcdc_driver` / `proto::ktcdc_journal`** — the flash-cdc state machine and its
  operation journal, lifted out of `main.rs` onto the `Transport` trait so they're unit-tested
  independent of `rusb` (93 tests total).
- **`serialtransport.rs` / `boottransport.rs`** — a CDC-ACM tty transport (serial, preferred by
  default) alongside the existing libusb path, so Linux (and eventually macOS-native) can drive
  the bootloader without claiming a USB interface.

### Proven on hardware
- **Linux-native, no OrbStack**: a full `flash-cdc --execute --yes` write completed on a real
  Debian 13 VM over the serial transport — 67/67 packets ACKed, device re-enumerated as a working
  JA11 with an identical descriptor SHA-256 to before. First hardware run of the refactored
  driver/journal code. Full narrative in `ROADMAP.md` Appendix D.
- Confirmed `packaging/99-ktflash.rules`'s `uaccess`-only default is a no-op on headless Linux
  (no logind seat) — `GROUP="plugdev"` is now uncommented by default.
- Documented a hard AlmaLinux/RHEL limitation: `kernel-devel` ships no `vhci-hcd` driver source,
  so RHEL-family guests can't be USB/IP clients at all (blocks this test rig, not `ktflash`).

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
