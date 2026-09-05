# ktflash (Rust)

A small, pretty `rusb`/libusb tool to **identify, handshake, and unlock** KTMicro `KT02H20`
USB‑C DAC dongles (the FiiO JA11 family). Read‑only by default; ships a live TUI.

```
cd flasher && cargo build --release      # needs libusb: brew install libusb
./target/release/ktflash                 # → TUI dashboard
```

| Command | What it does | Where |
|---|---|---|
| _(none)_ | live **TUI dashboard** — detects the dongle, shows its mode + interfaces, watches transitions | any OS |
| `probe` | one‑shot device report | any OS |
| `demo` | the simulated flash journey (stock → bootloader → JA11) — what the README GIF shows | any OS |
| `handshake` | normal mode: send the `0x4B`/`0x33` HID frame, read the reply | Linux/OrbStack¹ |
| `unlock` | send `T12345678` → reboot into the CDC bootloader (recoverable) | Linux/OrbStack¹ |
| `bootdiag` | probe the bootloader's CDC bulk pipe (after `unlock`) | Linux/OrbStack¹ |
| `fingerprint` | structured device identity (JSON) — VID/PID, strings, personality hash | any OS |
| `image <fw.bin>` | parse + validate a `KT_Helios` firmware image | any OS (no hardware) |
| `flash --plan <fw> [--device V:P] [--manifest m.json]` | build a flash plan + run the safety gate | any OS (no hardware) |
| `flash --apply <plan.json> [--execute] [--force-unsupported-device V:P]` | re‑verify + (with `--execute`) journaled native flash | Linux/OrbStack¹ for `--execute` |
| `recover <journal.json>` | safe next action for an interrupted flash | any OS (no hardware) |
| `dump --addr 0xA --len N [--out f]` | M4a readback spike via `0x08` word‑read | Linux/OrbStack¹ |
| `compat --template \| --validate <matrix.json>` | emit/validate a compatibility record | any OS (no hardware) |
| `bootdiag --replay <t.json>` | decode + validate a capture transcript | any OS (no hardware) |

¹ These **claim the USB interface**, which macOS refuses (`IOHIDFamily` owns it →
`LIBUSB_ERROR_ACCESS`). Run them inside the OrbStack guest ([`../orbstack/`](../orbstack/)) or on
Linux ([`../docs/LINUX.md`](../docs/LINUX.md)). Everything marked "no hardware" works natively on macOS.

## Protocol + safety core (`src/proto/`, no hardware)

The transport‑independent logic lives in [`src/proto/`](src/proto/) and is fully unit‑tested
without a dongle (`cargo test`): `image` (KT_Helios), `frame` (normal‑mode HID), `transcript`
(M0 capture format + tshark decoder), `cdc` (bootloader state machine + `FakeBootloader`),
`plan` (image gate + SHA‑256), `fingerprint` + `manifest` (fail‑closed device‑family allow‑list),
`journal` (operation journal + recovery model), and `compat` (compatibility matrix). Hardware I/O
is isolated in [`src/usbtransport.rs`](src/usbtransport.rs). The real CDC wire framing is the one
remaining gap: `cdc::PendingCodec` → `KtCdcCodec`. Run the full gate with [`./ci.sh`](ci.sh).

## Devices recognised

- `2972:0102` — **FiiO / JadeAudio JA11** (already‑flashed)
- `31B2:0111` — **KTMicro KT02H20** (stock)
- `31B2:*` — other KTMicro dongles
- `8888:CDC0` — **KT_USB_BOOT** bootloader (CDC serial, after unlock)

## Not here (yet)

The end‑to‑end firmware **write** — the CDC bootloader download protocol isn't fully reversed
(see [`../docs/EXTRACTION.md`](../docs/EXTRACTION.md)). `ktflash` takes the dongle up to and into
the bootloader, and the *entire flow around the write* — image gate, plan/apply, manifest,
operation journal, recovery, native transport — is built and tested. Capturing the wire framing
and implementing `KtCdcCodec` is the one open task; see [`../ROADMAP.md`](../ROADMAP.md) Phase 2.
