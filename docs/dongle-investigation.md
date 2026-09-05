# USB Audio Dongle Investigation — 3 devices

_Probed live from a MacBook Air (Apple Silicon) over USB, macOS, Sept 2026. Tooling: `ioreg`, `libusb` (ctypes), and custom `IOHIDManager` probes in C. No device was flashed or modified; control-channel access was read-only except a bounded, recoverable write-sweep on the Keysion (approved)._

## TL;DR

**All three are USB-C-to-3.5mm headphone DAC/amp dongles** — not a car amp among them (an early misread of the "CB1200AU HiFi DSP" name, corrected).

**Key fix vs. the earlier draft:** CB1200AU is **TTGK's own in-house DAC/DSP SoC** — it appears across TTGK's own product catalog (DAT9122HM-TT, DM5121HM-TT, DA8121HM-TT modules) — not a "JieLi-family" third-party part. The JieLi theory in the original draft is retracted; treat the EP0-descriptor leak as an artifact of TTGK's own USB stack, not evidence of a JieLi core.

| # | Device | Vendor / OEM | VID:PID | Category | Platform (confirmed) | Firmware-flashable? |
|---|--------|--------------|---------|----------|---------------------|---------------------|
| 1 | **KEYSION CB1200AU HiFi DSP** | Keysion / **TTGK (Shenzhen TTGK Technology, brand "Kangtang")** | `0x3302:0x128C` | USB-C→3.5mm DAC/amp, hi-res (32-bit/384 kHz, DSD64/128 claimed), Walk Play EQ, ~65 mW @32 Ω | TTGK in-house CB1200AU DAC/DSP SoC, UAC2 | No official/3rd-party image; no runtime DFU. Brick risk. |
| 2 | **TTGK CM01** | **TTGK Technology** | `0x3302:0x33A0` | USB audio adapter (stereo out + mic) | Same TTGK CB1200AU-family platform as #1 | Same as #1. |
| 3 | **KZ Acoustics C04** | KZ / **KTMicro** | `0x31B2:0x0313` | USB-C→3.5mm DAC, 24-bit/96 kHz negotiated | KTMicro USB-audio DAC (likely KT02H20 family) | Config via KTMicro/vendor tool possible; mask-ROM/OTP core with onboard FLASH for effects. |

**Big finding:** #1 and #2 are the **same OEM** (TTGK Technology, "Keysion" is a TTGK-affiliated brand). Two *different* chip vendors (TTGK and KTMicro) both expose a **similar HID control convention** — vendor usage page `0xFF01`, Report IDs `0x4B`/`0x54` — consistent with the shared **Walk Play** EQ/tuning ecosystem that TTGK operates and licenses out.

**Also worth noting:** USB VID `0x3302` is registered in third-party USB-ID databases to **both Moondrop and TTGK Audio** — it's a shared/unofficial vendor ID, not one TTGK exclusively owns, which is common practice among small Chinese audio OEMs using unregistered or borrowed VIDs.

---

## 1. KEYSION CB1200AU HiFi DSP (USB-C→3.5mm hi-res DAC / headphone amp)
_Currently unplugged; characterized in an earlier pass._

- **Product reality:** an affordable USB-C-to-3.5mm DAC + mini headphone amp. Marketed **up to 32-bit/384 kHz**, **DSD64/128**, **~65 mW @ 32 Ω**, plug-and-play UAC 2.0; pairs with TTGK's companion app **Walk Play** for multi-band EQ. _(Not a car amp — corrected from the first draft.)_
- **Identity:** VID `0x3302` (shared/vanity, used by both Moondrop and TTGK Audio per USB-ID databases), PID `0x128C`, serial `3302128C251127`, bcdDevice `0x0001`, USB 2.0 HS, composite `EF/02/01`.
- **Topology:** UAC2 — IF0 audio-control, IF1 out (2ch = **stereo headphone out**), IF2 in (1ch = **headset/inline-mic pin**); the `48 kHz` seen was just the currently-negotiated rate, not the ceiling. + IF3 vendor HID (2 interrupt endpoints).
- **Control channel:** HID report descriptor = Consumer collection (Report ID `0x03`: Vol±/Play/Voice media keys) + vendor page `0xFF01` with **two 63-byte bidirectional pipes, Report IDs `0x4B` and `0x54`**. This is the **Walk Play EQ/tuning path**, which TTGK runs from its own `walkplay.szttgk.com` domain.
- **Chip (corrected identification):** **CB1200AU is a TTGK in-house DAC/DSP SoC**, appearing across multiple TTGK reference PCBA modules (DAT9122HM-TT, DM5121HM-TT, DA8121HM-TT) with claimed 384 kHz/32-bit playback and 8-band EQ support tied to Walk Play. The earlier "JieLi-family USB-stack fingerprint" and "AC69 Bluetooth SoC / class-D power stage" theories are both retracted — there's no evidence tying CB1200AU to JieLi silicon; it's TTGK's own part. Confirming the exact die marking would still require opening the case.
- **Probe result:** Full bidirectional HID transport works natively from the Mac (open shared / GET_REPORT / SetReport all return 0). Device replies only to **correctly-framed** commands over interrupt-IN; a bounded blind opcode sweep got zero responses → the protocol has real framing (header/length/checksum).
- **Firmware verdict:** No official Keysion image published; no 3rd-party/open firmware for this model; **no DFU/bootloader interface** in runtime mode. Nothing safe to flash. Since CB1200AU is TTGK's own chip rather than a JieLi part, the JieLi SDK/tooling referenced in the earlier draft (`Jieli-Tech/fw-AC63_BT_SDK`, etc.) is **not applicable** here — drop that lead. Only a full dump from an identical CB1200AU unit (via Walk Play's own update mechanism, if any) would be safe to restore from.

---

## 2. TTGK "CM01" (USB audio adapter)

- **Identity:** VID `0x3302` **(same as the Keysion)**, PID `0x33A0`, **no serial**, bcdDevice `0x0001`, bcdUSB `0x0201`, USB 2.0 HS, composite `EF/02/01`, bus-powered **100 mA**, config wTotalLength 422.
- **Strings:** iManufacturer = `TTGK Technology Co.,Ltd`, iProduct = `CM01`. → **Confirms TTGK is the OEM behind "Keysion."**
- **Topology (UAC2):**
  - IF0 — Audio Control.
  - IF1 — Playback OUT, 3 alt settings, iso EP `0x01` at **192 / 288 / 384 B** per microframe (real multi-rate playback bandwidth).
  - IF2 — Capture IN (mic), iso EP `0x81` at 24 / 36 B.
  - IF3 — **HID, single interrupt-IN endpoint `0x83` (64 B), report-descriptor len 63.** OUT reports go via control SetReport.
- **Control channel:** HID = 61-byte bidirectional pipe (Report ID `0x01`, declared on Consumer page usage 0) + a media-key collection. Different framing from the Keysion's `0xFF01`/`4B`/`54`, but same idea.
- **Probe result (read-only):** `GET_REPORT(Input, 0x01)` returns a **structured 62-byte status block**: `01 00 7b ff 00 80 00 …` — i.e. Report ID 1 is a **readable config/status register** (more forthcoming than the Keysion, which only echoed its descriptor buffer). No unsolicited reports while idle.
- **Chip / firmware:** Same TTGK OEM and CB1200AU-family USB-audio platform as the Keysion → same firmware situation (no public images, no DFU). `0x33A0` PID + mic + richer playback bandwidth suggests a fuller USB audio-adapter variant on the shared TTGK SoC platform.

---

## 3. KZ Acoustics "C04" (USB-C headphone DAC)

- **Identity:** VID `0x31B2` **(KTMicro)**, PID `0x0313`, **no serial**, bcdDevice `0x0003`, USB 2.0 / **Full-Speed**, class `0` (per-interface), bus-powered **98 mA**, config wTotalLength 203.
- **Strings:** iManufacturer = `KTMicro`, iProduct = `KZ Acoustics C04`.
- **Topology (UAC1, output-only — no mic):**
  - IF0 — Audio Control.
  - IF1 — Playback OUT, iso EP `0x04`. Two formats:
    - alt1 — **16-bit**, 44.1 / 48 / 88.2 / 96 kHz
    - alt2 — **24-bit**, 44.1 / 48 / 88.2 / 96 kHz
    - → **Real ceiling: 24-bit / 96 kHz, stereo.** No 192/384 kHz, no 32-bit, no DSD.
  - IF2 — **HID, two interrupt endpoints (`0x83` IN + `0x03` OUT), 16 B, report-descriptor len 70.**
- **Control channel:** HID = media keys (Report ID `0x01`) + **vendor page `0xFF01` with Report IDs `0x4B`/`0x54`, 10-byte reports** — the *same convention as the Keysion*, smaller payloads. This is KTMicro's config path (digital filter / gain / hi-res mode; KTMicro's own USB-audio SoCs integrate onboard FLASH specifically for this kind of second-stage effects configuration).
- **Probe result (read-only):** `GET_REPORT(Input, 0x4B/0x54)` **stalls** (`0xe0005000`) — these pipes are write-then-async-reply only; read-only can't coax them without sending an OUT command.
- **Chip identification (updated):** KTMicro's **KT02H20** is confirmed by its own datasheet to support up to **384 kHz/32-bit DAC** with **DSD64/128**, integrated headphone Class-G amp, and onboard FLASH for effects configuration, and it's the DAC used in numerous well-known dongles (JCALLY JM12, FiiO JA11-adjacent designs, Audiocular Note). Since this unit only negotiates **24-bit/96 kHz** — well below the KT02H20's rated ceiling — it is most likely either **a lesser/lower-cost KTMicro SKU** or a **KT02H20 unit firmware-limited to 24/96** via the onboard FLASH config (vendors occasionally ship value SKUs this way to hit a lower price point). **Definitive chip ID still requires reading the die marking (open case / photo).**

---

## Cross-device conclusions

0. **All three are the same product category** — USB-C-to-3.5mm headphone DAC/amp dongles (hi-res-capable, EQ via a companion app). Not a car amp among them.
1. **TTGK = the OEM for #1 and #2.** Devices #1 (Keysion CB1200AU) and #2 (CM01) share VID `0x3302` and the `TTGK Technology Co.,Ltd` manufacturer string; "Keysion" ships TTGK's own CB1200AU chip. This VID is also used by Moondrop, so don't over-index on VID alone for future identifications.
2. **Shared HID tuning convention, not shared silicon.** TTGK's CB1200AU platform (#1, #2) and KTMicro's platform (#3) both expose vendor usage page `0xFF01` with Report IDs `0x4B`/`0x54` — this now looks like convergent design around the Walk Play companion-app convention rather than a shared chip vendor (the earlier "JieLi" link is retracted).
3. **Mac-side transport is fully proven.** Open (shared), GET_REPORT, and SetReport all work natively via `IOHIDManager` — **no Windows needed for the transport**, only to capture the real app's framing.
4. **No slam-dunk firmware target.** None of the three exposes a runtime DFU/bootloader; none has published official or 3rd-party firmware. The realistic wins are **tuning** (EQ / digital-filter / gain via each vendor's protocol) and **backup/recovery**, not "better firmware." Note: at least one TTGK-designed dongle (TRN Black Pearl) has received an official Walk Play firmware update addressing DRE distortion on Cirrus Logic CS431xx chips — so TTGK does push firmware updates for some SKUs via Walk Play; whether CB1200AU units get the same treatment is unconfirmed.

## Recommended next steps
- **Capture the companion app once** (Walk Play, at `walkplay.szttgk.com`, or the KTMicro/KZ tool — a USB capture, or an Android USB/HCI snoop) → decode the `0x4B`/`0x54` (or CM01 Report-1) frame format → replay from the Mac. **KZ C04's 10-byte protocol is the easiest to reverse.**
- **Photo the PCBs** to confirm exact silicon (verify CB1200AU markings on the TTGK dongles; confirm KT02H20 vs. a lesser KTMicro SKU on the C04).
- **Check for a Walk Play firmware/OTA path** for the CB1200AU units, given TTGK has shipped at least one firmware fix for a related dongle (TRN Black Pearl) through that app.
- **Backup before any write:** read each unit's own image with its vendor tool first.
