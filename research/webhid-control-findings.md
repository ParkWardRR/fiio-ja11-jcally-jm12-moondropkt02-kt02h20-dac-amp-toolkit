# FIIO's own WebHID control site — RE findings

**Site**: [fiiocontrol.fiio.com](https://fiiocontrol.fiio.com) — a Vue.js single-page app using
the **WebHID** and (for firmware updates) **Web Serial** browser APIs. Per FIIO's own forum
posts, it requires "a browser that supports Web HID API, such as Chrome, Edge, Opera, etc.",
works on **Windows and macOS** ("Android and iOS systems are not supported"), and added JA11
PEQ tuning support in **firmware V1.9** (2024-07-29).

This is a **third, independent artifact** confirming the protocol already reversed from the
Windows vendor tool (`cdc-re-findings.md`) and the Android app
(`android-app-re-findings.md`) — and it's the most directly relevant one for `ktctl`, since it's
plain browser JavaScript already running natively on macOS, with none of the APK-decompilation
legal/technical overhead.

## Method

Fetched directly with `curl` (a public website, no auth, no app store/DRM involved):
`index.html` → its single JS bundle (`static/js/index-*.js`, ~2.2 MB, Vite/Vue production
build, minified but not obfuscated — variable names are short but all string/numeric literals
are intact). Searched the bundle for `navigator.hid`/`requestDevice` usage, the frame-builder
call sites, and FIIO's product-ID/name tables.

## Findings that confirm existing RE (independent triangulation)

| Fact | Android app said | WebHID site says | Verdict |
|---|---|---|---|
| Frame magic | `AA 0A` write / `BB 0B` read | Frame builder called as `ut(170,10,cmd,payload)` (write) / `ut(187,11,cmd)` (read) — `170==0xAA`, `10==0x0A`, `187==0xBB`, `11==0x0B` | ✅ exact match |
| JA11 internal product id | `109` | `109` (from the site's device-id table) | ✅ exact match |
| JA11 filter types | `Peak(0)/LowShelf(1)/HighShelf(2)`, 3 of 7 shared types | Same 3, same order, from the site's per-device filter-type table | ✅ exact match |
| CRC-8 table | Dallas/Maxim (`table[1]==94`) | Byte-identical table found in the bundle | ✅ exact match |
| PEQ opcode | `0x15` (21) | `21` | ✅ |
| Preset-select opcode | `0x16` (22) | `22` | ✅ |
| Master gain opcode | `0x17` (23) | `23` | ✅ |

## New findings (not visible from the Android app alone)

### 1. The mystery leading `0x02` byte is the USB HID Report ID

The Android app always prefixed JA11 frames with `0x02` but it wasn't clear why (§4 of
`android-app-re-findings.md` flagged this as unconfirmed). The WebHID site's report-ID lookup
resolves it directly: **the HID report ID is per-product**, looked up by product name — `JA11 →
2`, a different product (`KA17`) `→ 1`, USB-Audio-Class-only devices `→ 0`, and everything else
defaults to `7`. So `0x02` isn't part of the payload or a protocol quirk — **it's the standard
USB HID report ID prefix** `sendReport()`/`bulkTransfer()` needs, and it's `2` specifically
*because JA11 is JA11*, not a universal constant. `ktctl` needs this same per-device report ID
table if it ever supports more than the JA11.

### 2. A required ~100ms keepalive/heartbeat

The site runs a periodic "KeepAlive" task (literally named `"KeepAlive"` in the minified bundle)
at a **100ms interval** for a specific list of devices that **includes the JA11**, with a
heartbeat-failure counter that triggers a disconnect after repeated misses. **This has no
equivalent finding in the Android app RE** — worth hardware-confirming, since if real, `ktctl`
needs to keep sending *something* (likely a cheap read, e.g. the `0x0B` version query) every
~100ms or the device may drop the connection. This is a genuinely new requirement, not just
corroboration.

### 3. Confirmed dual-channel architecture matches `ktflash` + `ktctl`'s split exactly

The site's internal state models two `connectType`s: `HID` (labeled `"USB"` in the UI) and
`SERIAL` (labeled `"Serial Port"`) — i.e. it independently arrived at the same architecture
already reverse-engineered here: a **WebHID-based runtime control channel** (this doc, and
`android-app-re-findings.md` §4/§4b) plus a **separate Web-Serial-based channel** for firmware
upgrade (matching `ktflash`'s CDC-ACM bootloader work in `docs/CDC-PROTOCOL.md`). Two different
transports for two different jobs, on both platforms that have implemented this device — good
validation that `ktctl` and `ktflash` staying as separate tools (per `ktctl`'s README) mirrors
FIIO's own architecture, not an arbitrary split.

### 4. JA11's real factory EQ presets — corrects a common web claim

Some marketing/forum copy (and a web-research summary fed into this session) describes the
JA11's "three factory EQ options" as **Classic, Pop, and Jazz**. The WebHID site's own preset
table for the JA11 says otherwise — its `[cmd 0x16 value] → [label]` map for JA11 specifically
is:

| value | label |
|---|---|
| `0` | Vocal |
| `1` | Classic |
| `2` | Bass |
| `3` | USER1 (a user-customizable slot, not a factory curve) |
| `4` | EQ off |

"Classic/Pop/Jazz" is a **different FIIO product's** preset set (the BTR13, per the same table)
— easy to see how that got conflated in secondhand descriptions. This directly resolves and
supersedes the "0-3 = preset slot (inferred), 4 = off" guess in `android-app-re-findings.md`
§4 — now confirmed with real labels from FIIO's own shipping code.

### 5. VID list includes KTMicro's `0x31B2` — a Phase 6 lead for `ktflash`, not yet a conclusion

The site's WebHID device filter (`navigator.hid.requestDevice({filters: [...]})`) lists several
vendor IDs, including **`10610` = `0x2972`** (FIIO's own VID) and **`12722` = `0x31B2`** — the
same KTMicro VID `ktflash` already ties to the Moondrop/JCALLY JM12 clones
(`docs/COMPATIBILITY.md`). **This is not proof FIIO's control site works with those clones** —
a defensive/generic filter entry could exist for other reasons (e.g. catching a bootloader-mode
VID, or an unrelated KTMicro-based product FIIO also sells) — but it's a concrete lead worth
chasing for Phase 6's compatibility matrix: if the WebHID site *does* successfully connect to a
JM12, that's a free, no-flashing way to test PEQ compatibility across the shared silicon.

### 6. Other opcodes exist in the shared command set, most likely not JA11's

The frame-builder call sites cover a much larger opcode range (LED color/brightness/mode,
gesture config, and more) than JA11 exposes — this is FIIO's **one shared protocol across their
whole USB/BLE product line**, of which JA11 only uses a handful of opcodes. Not itemized here
(most are clearly for products with hardware JA11 lacks, e.g. RGB LEDs) — flagged so nobody
mistakes "found in the bundle" for "applies to JA11."

## Still open

- **Byte-level confirmation of the CRC-8 range and the two magic-byte report-ID relationship**
  — the WebHID site's `ut()` frame builder wasn't fully deobfuscated here; the call-site
  arguments (`ut(170,10,cmd,payload)`) confirm the opcode/magic-byte layer but not, e.g.,
  whether the CRC-8 covers the report-ID byte or not.
- **Whether the 100ms keepalive is JA11-hardware-required or just this web client being
  conservative** — worth testing without it before assuming it's load-bearing.
- Whether the KTMicro VID entry (§5) actually round-trips PEQ commands against a JM12/Moondrop
  unit — untested, no hardware here.
