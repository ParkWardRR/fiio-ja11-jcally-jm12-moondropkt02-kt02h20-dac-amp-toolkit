# FIIO Control (Android) — RE findings

Static RE of the official **FIIO Control** Android app, both versions found in
`~/Downloads`:

- `com.fiio.control_3.45.0-85` (minAPI21, arm64-v8a/armeabi-v7a/x86/x86_64)
- `com.fiio.control_4.0.0-90` (minAPI24, same ABIs)

Goal: corroborate the Windows‑tool‑derived CDC bootloader protocol
([`CDC-PROTOCOL.md`](../docs/CDC-PROTOCOL.md)) from an independent vendor artifact, and
reverse the **runtime PEQ/state control protocol** — the ROADMAP's
"EQ / PEQ tuning over the `0xFF01` control channel" idea item.

## Method

Both APKs are hybrid **Flutter + native Android** apps (`libapp.so` = Dart AOT
snapshot, `libflutter.so` = engine — not decompiled here, would need `blutter`),
but the entire JA11 module and the shared device-control layer are plain
**Java/Kotlin, R8‑obfuscated but not encrypted** — fully readable via:

```bash
brew install apktool jadx
apktool d -s -f -o <out>/apktool "<apk>"      # manifest + resources, fast
jadx -d <out>/jadx --show-bad-code -j 4 "<apk>"  # full source decompile
```

Both versions decompiled cleanly (a handful of expected per-method errors on edge
cases, ~5900–8400 dex methods). The JA11-relevant code is **byte-identical
between v3.45.0 and v4.0.0** — only R8's obfuscated symbol names differ between
builds.

## 1. App structure relevant to JA11

- `com.fiio.controlmoduel.model.ja11.ui.Ja11Activity` (extends `Ja11OtaUpgradeActivity`,
  extends `com.fiio.controlmoduel.base.BaseUsbControlActivity`) — the JA11 screen.
- Internal product ID for JA11 is the constant **`109`**, used consistently across
  the OTA URL table, the PEQ model constructor, and the device dispatch switch.
- No BLE for JA11 (it has no radio) — despite the app requiring
  `android.hardware.bluetooth_le`, JA11 is driven entirely over **USB** from the
  phone (OTG), via `android.hardware.usb.UsbManager`/`UsbDeviceConnection`.
- No `device_filter.xml` / USB `<intent-filter>` for `USB_DEVICE_ATTACHED` exists in
  either manifest — JA11 (and other USB products) are selected by the app's own
  device picker UI, not matched by VID/PID at the OS level. **Negative result**:
  the APK does not contain a ready-made VID/PID compatibility list for Phase 6.

## 2. CDC bootloader protocol — independently confirmed (CONFIDENCE: HIGH)

`Ja11OtaUpgradeActivity` talks to the bootloader over a `com.hoho.android.usbserial`
(`UsbSerialProber`/`UsbSerialPort`) CDC‑ACM connection at **115200 8N1** — i.e. a
real serial port, exactly matching `CDC-PROTOCOL.md`'s "appears as `/dev/ttyACM0`"
transport, obtained from an **entirely different vendor artifact** than the
Windows `.exe` originally reversed.

The packet builder (`Ja11OtaUpgradeActivity.f0(int bank, int len, int addr, byte[] payload)`,
same in both app versions modulo obfuscated names) is a **byte-exact match** for
`docs/CDC-PROTOCOL.md`'s 6-byte header + CRC-32:

```java
byte[] bArr3 = {105, (byte) i11, (byte) ((i10 << 5) | (i11 >> 8)), (byte) i12, (byte) (i12 >> 8), (byte) (i12 >> 16)};
// H[0]=0x69('i'=105) constant marker
// H[1]=len&0xFF, H[2]=(bank<<5)|(len>>8), H[3..5]=addr LE24
```

CRC: a 256-entry `long[]` table (`B0`), confirmed byte-for-byte the canonical
**CRC-32 table** (`table[1] == 1996959894 == 0x77073096`, poly `0xEDB88320`),
computed over **header(6)+payload** with **init=0, no final XOR** — matching
`docs/CDC-PROTOCOL.md` exactly, including the "not payload-only" correction
already recorded there:

```java
long j5 = 0;
for (i in 0 until header.len+payload.len) {
    j5 = ((j5 >> 8) & 0xFFFFFF) ^ table[(int) (((frame[i] & 0xFF) ^ (j5 & 0xFF)) & 0xFF)];
}
// appended little-endian, no final XOR
```

**Net result**: the JA11 CDC bootloader protocol in `docs/CDC-PROTOCOL.md` is now
confirmed from *two independent vendor artifacts* (Windows Qt tool, Android app) —
about as solid as static RE gets without a USB capture.

## 3. FIIO's OTA CDN — a public firmware source (CONFIDENCE: HIGH)

`wf/f.java` (`com.fiio.controlmoduel`'s OTA helper) contains a hardcoded per-product
URL table for **~30 FIIO product lines**, keyed by the same internal product ID
used everywhere else in the app. For JA11 (`id == 109`):

| Purpose | URL |
|---|---|
| Firmware image | `http://fiio-bluetooth.fiio.net/JA11/JA11.bin` |
| Version check | `http://fiio-bluetooth.fiio.net/JA11/version_new.txt` |
| OTA changelog | `http://fiio-bluetooth.fiio.net/JA11/JA11_ota_log_{en,zh}.txt` |

Verified live (2026‑09‑05): `version_new.txt` returns `2.2`; `JA11.bin` is
**67,312 bytes** (`Content-Length`), the exact size of the `JadeAudio JA11_V2.2.bin`
already reversed in `research/cdc-re-findings.md` §1 — i.e. **this is the same
firmware**, served directly by FIIO with no Windows installer needed. This is a
convenient, always-available source for the "bring your own image" precondition in
[`REVERT.md`](../docs/REVERT.md) — no need to find/keep the Windows package around.
(Not yet byte-compared/hashed against the local copy; do that before relying on it
as a substitute.)

## 4. Runtime PEQ / state control protocol (CONFIDENCE: HIGH, static only)

This is new — not previously documented. Found via `Ja11Activity` → its `sa.a`
ViewModel → `qa.b` (a per-product command-frame builder shared by many FIIO
device modules through a common R8-merged dispatcher, `qa.a`).

### Transport
**Raw USB bulk transfer against the HID-class interface**, not the CDC-ACM serial
port used for OTA. `ng/b.java` resolves the interface/endpoints **dynamically**
(no hardcoded interface or endpoint numbers) with a simple, portable heuristic —
confirming and completing `docs/CDC-PROTOCOL.md`'s note that runtime EQ commands
go over the "`0xFF01` HID collection":

```java
for (int i10 = 0; i10 < usbDevice.getInterfaceCount(); i10++) {
    UsbInterface iface = usbDevice.getInterface(i10);
    if (iface.getInterfaceClass() == 3 && iface.getEndpointCount() == 2) {   // 3 = USB_CLASS_HID
        for (int i11 = 0; i11 < iface.getEndpointCount(); i11++) {
            UsbEndpoint ep = iface.getEndpoint(i11);
            if (ep.getDirection() == 0) outEndpoint = ep;         // UsbConstants.USB_DIR_OUT
            else if (ep.getDirection() == 128) inEndpoint = ep;   // UsbConstants.USB_DIR_IN
        }
    }
}
usbDeviceConnection.claimInterface(iface, true);  // force-claim, detaches the kernel HID driver
```

i.e.: **the first interface with class `HID` (3) and exactly 2 endpoints is the
control interface** — one OUT, one IN, picked by direction bit. This resolves the
"exact endpoint numbers" open item below: there's nothing to hardcode, the
interface/endpoints are descriptor-discovered, so `ktctl` can use the identical
heuristic against `rusb`/`nusb` interface descriptors on macOS/Linux.

### Frame format (`qa.b.f()` = read/query frames, `qa.b.g()` = write frames)

```
read  (USB mode): 02 BB 0B <seq_hi> <seq_lo> <cmd> <len> <payload...> <crc8> EE
write (USB mode): 02 AA 0A <seq_hi> <seq_lo> <cmd> <len> <payload...> <crc8> EE
```

- Leading `0x02` is present because the JA11 manager is constructed with a
  `isUsb`-style flag hardcoded `true` (`qa.b(pa.a) { this.f17339e = true; }`); the
  `false` branch (no leading `0x02`, magic swapped to `-69/-86` without the prefix)
  is presumably the BLE variant used by FIIO's Bluetooth products sharing this
  same builder class.
- `BB 0B` = read-frame magic, `AA 0A` = write-frame magic (constants, always these
  two bytes in that order).
- `seq` is a 16-bit big-endian free-running counter (`0..32767`, wraps to `0`),
  incremented per call — not currently checked against docs' `0x4B` vendor-channel
  framing, this is a **separate, newer channel** (`AA/BB…EE`) than the `0x4B`
  11-byte reports the Windows tool used for its own (older/different-product) EQ
  channel. Treat as a distinct protocol version until proven otherwise on a JA11.
- `cmd` — single opcode byte (values seen for JA11: see table below).
- `len` — payload length in bytes.
- `crc8` — **CRC‑8/MAXIM** (aka Dallas/Maxim 1-Wire CRC-8; poly `0x31` reflected,
  init `0`), table-driven (`qg.a.f17478d`, confirmed by `table[1] == 0x5E == 94`,
  the canonical CRC-8/MAXIM signature). Computed over the frame **from the
  post-`0x02` magic byte through the last payload byte** (i.e. excludes the leading
  `0x02` and excludes itself/the trailing `0xEE`).
- `0xEE` — fixed frame terminator, last byte.

### Known opcodes (JA11, product id 109)

| cmd | direction | payload | meaning |
|---|---|---|---|
| `0x15` (21) | read: 1B band index → reply 8B<br>write: 8B | `[index, Q_hi, Q_lo, gain_hi, gain_lo, freq_hi, freq_lo, type]` | per-band PEQ get/set. 5 bands queried on connect (`i in 0..4`) |
| `0x16` (22) | read: 0B → reply 1B<br>write: 1B | `[value]` | PEQ enable / active-preset-index (0-3 = a preset slot, 4 = "off", inferred from `sa.a.f()`) |
| `0x17` (23) | read: 0B → reply 2B<br>write: 2B | 16-bit signed, ×10 | global/makeup gain, in 0.1 dB steps |

Per-band payload encoding (`qa.b.h(zg.a band)` on write; mirrored in `qa.b.b(String)`
on read-reply parsing):

- `index` — 1 byte, band number (0-based).
- `Q` — 16-bit **signed** big-endian, fixed-point ×100 (e.g. `0.7` → `70` → `0x0046`).
- `gain` — 16-bit signed big-endian, fixed-point ×10 (dB, e.g. `-3.5` → `-35`).
- `freq` — 16-bit **unsigned** big-endian, plain Hz (no scaling).
- `type` — 1 byte, filter type enum. FIIO's shared PEQ picker (`R$string`, package
  `com.fiio.fiioeq`) defines **7** named filter types in this fixed order: `Peak`,
  `LowShelf`, `HighShelf`, `BandPass`, `LowPass`, `HighPass`, `AllPass` (indices
  `0`-`6`). **The JA11 specifically only exposes 3** — its own band-edit fragment
  (`oa/d.java`, the `oa` package being the JA11 model's fragments) builds its
  filter-type picker from only the first three: `{filter_peak, filter_low_shelf,
  filter_high_shelf}` — i.e. on a JA11, `type` is `0`=Peak, `1`=LowShelf,
  `2`=HighShelf only. (Other FIIO products' band-edit fragments — e.g. `zb/f.java`,
  `bd/h.java` — use the full 7-type list; the `3↔1`/`4↔2` remap noted above is
  `qa/b.h()` translating between a 7-type BLE product's values and the JA11's
  3-type USB range, not something the JA11 itself needs to interpret.)

Scaling helper (`tg.b.i(float v, int scale) → hex string`, `tg.b.g(String) → 2 bytes`):
`hex(int(v*scale))`, keeping only the low 16 bits (so negative values fall out as
correct two's-complement `int16`) — i.e. exactly a **signed big-endian `int16`,
value = round(v*scale)**. The decode side (`db.a(scale, hex)`, name is an R8
class-merging artifact — nothing to do with Google ML Kit / barcode scanning,
just an arbitrarily-named merged utility class) is presumed to be the inverse
(`parseInt16(hex)/scale`) by symmetry, not yet byte-traced.

### What this enables

This is exactly the ROADMAP "Ideas" item — **"EQ / PEQ tuning over the `0xFF01`
control channel (the app protocol `W/R/S/C`) from the TUI"** — now has a concrete,
byte-level spec to implement against: claim the vendor USB interface, bulk-write
`AA 0A`-framed commands (cmd `0x15`/`0x16`/`0x17`), CRC-8/MAXIM checksum, `0xEE`
terminator. **Not yet hardware-validated** — everything above is static RE only;
treat the `0x02` lead byte's necessity and the exact CRC-8 byte range as unconfirmed
until tried against a real JA11 (ideally with a USB capture, since unlike the CDC
bootloader work there's no Windows-side decompile to cross-check this specific
channel against). The interface-discovery heuristic and the filter-type enum are
now pinned down from static RE alone (§4 above) — the remaining gap is purely
"does it work when you actually send it," not "what should be sent."

## 4b. Additional opcodes — the "device state" channel

The JA11's **state tab** (`fragment_ja11_state.xml`, `oa/i.java` fragment) uses the *same*
`AA/BB…EE` frame format but a **separate frame-builder instance** (`qa.c`, driven by the
`sa.b` ViewModel — distinct from `qa.b`/`sa.a` used for PEQ). Cross-referencing
`qa/c.java`'s reply parser against the state tab's layout XML fully identifies six more
opcodes:

| cmd | meaning | payload / values |
|---|---|---|
| `0x02` (2) | current output volume | 1 byte, plain integer (label: `device_volume`) |
| `0x09` (9) | current sample rate / format | 1 byte, index into `{32k, 44.1k, 48k, 88.2k, 96k, 176.4k, 192k, 352.8k, 384k, 705.6k, 768k, DSD64, DSD128, DSD256, DSD512}` |
| `0x0B` (11) | firmware version | 2 bytes → `"{byte0}.{byte1}"` (e.g. `2.2`) — a **runtime** version readback, redundant with (and a good cross-check against) the network `version_new.txt` endpoint in §3 |
| `0x12` (18) | in-line mic detect | 1 byte boolean; UI label `@string/ka1_mic_detect` = *"In-line microphone :"* — whether a mic-equipped headset is plugged into the combo jack |
| `0x16` (22) | *(seen in the frame parser; not bound to any visible control in the state tab — meaning not yet identified)* | 1 byte |
| `0x20` (32) | USB Audio Class mode | 1 byte, `0` = UAC 1.0, non-zero = UAC 2.0; read **and** write (`sa.b.q(int)` sends this on write) — surfaced as a `RadioGroup` (`rg_uac`, `rb_uac_a`/`rb_uac_b`) in the UI |

Note `0x16` here is a **different meaning** than `0x16` on the PEQ channel (§4, "PEQ enable /
preset slot") — same numeric opcode, different frame-builder instance (`qa.c` vs `qa.b`), so
either the cmd namespace is scoped per logical channel rather than being device-global, or
these two `0x16` uses coincidentally don't collide on the wire for reasons not yet understood.
**Needs hardware confirmation** to know whether `qa.b` and `qa.c` really are talking to the
same physical endpoint pair or different ones.

### What this adds

Beyond PEQ, `ktctl` can plausibly also expose: **live volume**, **current sample rate/format**
(handy for confirming bit-perfect passthrough), **firmware version** (no OTA/network call
needed), **mic-detect status**, and **UAC 1.0/2.0 mode switching** (useful for OS/driver
compatibility troubleshooting) — all over the same USB interface and frame format as PEQ,
just a different frame-builder "channel." All static-RE-only, same caveats as §4.

## 4c. Real-world UI confirmation (no wire capture, but strong semantic corroboration)

FIIO's own support article,
[*"How to control the JA11 via the FiiO Control APP in Android mobile phone?"*](https://fiiosupport.freshdesk.com/support/solutions/articles/69000869868-how-to-control-the-ja11-via-the-fiio-control-app-in-android-mobile-phone-),
plus screenshots of the live app connected to a real JA11, confirm the **semantics** (not the
wire bytes) of everything in §4/§4b:

- **EQ screen**: a 5-band curve editor (bands seen at `29 / 81 / 600 / 7460 / 15660 Hz`, gains
  in a `±12 dB` range) plus a separate **master gain** slider, an EQ on/off toggle, and
  `Custom`/`Advanced Settings`/`save` actions — matches `0x15` (per-band), `0x17` (master
  gain), `0x16` (on/off) exactly.
- **Status screen**: shows **volume `60`**, **sample rate `384k`**, **in-line mic detect ON**,
  and **UAC version `UAC 2.0`** (selectable against `UAC 1.0`) — a live, real-device readout
  matching `0x02`/`0x09`/`0x12`/`0x20` (§4b) value-for-value.
- **iOS, confirmed unsupported by FIIO itself** (not just Apple-platform speculation): the
  article states plainly, *"The JA11 could not be controlled via the iOS version FiiO Control
  APP."* Consistent with §4's transport finding — this is raw USB host access
  (`UsbDeviceConnection`), which iOS doesn't expose to third-party apps outside MFi.

**One open discrepancy**: the Status screen's displayed version is **`1.4`**, not `V2.2` (the
firmware image analyzed in `cdc-re-findings.md` and confirmed live from FIIO's OTA CDN in §3).
Plausible explanations, unconfirmed: `1.4` could be a **protocol/hardware revision** string
(read via runtime opcode `0x0B`) distinct from the **flashable firmware build number** (`V2.2`,
read via the CDC bootloader's own `VER`/`INF`) — i.e. two different version counters on the
same device, not a contradiction. Needs a real device to resolve; don't assume either number
is wrong.

## 4d. Cross-validation against independent open-source implementations

Two other open-source projects have independently reverse-engineered (and, unlike
everything in §4/§4b, actually shipped against real hardware) the same
`AA 0A .../BB 0B ... EE` runtime control channel:

- **[fiiocontrol-oss](https://github.com/adithyasource/fiiocontrol-oss)**
  (adithyasource) — a WebHID driver reverse-engineered by sniffing the *official*
  fiiocontrol.fiio.com web app's own traffic. Lists JA11 under "currently
  supported" (not "testing"), i.e. it works against real units in the wild.
  Device driver: [`src/libs/devices/fiioJa11.js`](https://github.com/adithyasource/fiiocontrol-oss/blob/main/src/libs/devices/fiioJa11.js).
- **[glacier-eq](https://github.com/Bukutsu/glacier-eq)** (Bukutsu) — a
  Tauri+Rust+React cross-platform PEQ editor with a device-registry
  architecture; lists FiiO JA11 as `Testing` (family match, not yet fully
  confirmed). JA11 protocol impl: `glacier-core/src/device/fiio.rs`
  (`JA11_PROTOCOL` vs. a separate `FIIO_PROTOCOL` for other FIIO products
  sharing the same frame builder).

**Agrees with §4, byte-for-byte** (report ID `0x02`, `AA 0A`/`BB 0B`…`EE` framing,
cmd `0x15` per-band layout — `index, gain×10 BE i16, freq BE u16, Q×100 BE u16,
type`, and the 3-value JA11 filter-type enum `0=Peak/1=LowShelf/2=HighShelf`):
strong independent confirmation this part of the static RE is correct.

**Disagrees with §4 on the master/global gain encoding (cmd `0x17`).** §4 claims
"16-bit signed, ×10, [implicitly big-endian like the other fields]" from the
Android app's static RE alone. Both external, hardware-facing implementations
instead use **×2560 scale, little-endian**, clamped to ±12 dB
(`fiioJa11.js`: `value = round(clamp(-12,12,v)*2560)`, sent
`value & 0xff, (value>>8)&0xff`; `fiio.rs`: `JA11_PROTOCOL { gain_scale: 2560.0,
endian: Little, .. }`) — and they agree with each other exactly. Given both are
field-tested (or field-adjacent) against real JA11 units and the Android
decompile for this one field was never hardware-checked, **treat ×2560/little-
endian as the more likely-correct value for master gain**, and treat §4's
"×10, presumably big-endian" for cmd `0x17` as probably describing a *different*
FIIO product sharing the `qa.b` frame builder, not the JA11 specifically. This
needs a real JA11 to settle definitively — it's an easy one-command hardware
check (`ktctl` doesn't exist yet, but a raw HID write of `AA 0A 00 00 17 02 00
00 00 EE` — gain `0.0`, or a nonzero test value — against report ID `2`, then
read back with `BB 0B 00 00 17 00 00 EE`, would confirm which encoding round-
trips correctly).

**A command §4 never found: save/commit-to-flash — and the two external
projects disagree with each other.** Neither the Windows tool nor this app
decompile surfaced a distinct "persist to NVM" opcode; §4 only covers live
`W/R` of RAM state. The two external repos both have one, but *not the same
one*: `fiiocontrol-oss` (tested, JA11-supported) sends
`AA 0A 00 00 19 01 03 00 EE` (cmd `0x19`/25, payload `[3]`) after writing all
bands; `glacier-eq`'s `JA11_PROTOCOL` overrides a generic `FIIO_PROTOCOL`
default of cmd `0x19` down to **cmd `0x18`/24, payload `[1]`**, specifically for
JA11 — but JA11 is only `Testing` status there, so that override may itself be
unconfirmed/aspirational rather than hardware-derived. **Unresolved**: which of
`0x18`/`payload=1` or `0x19`/`payload=3` (or both, for different purposes) is
the real JA11 save command. Needs hardware to settle; `fiiocontrol-oss`'s claim
carries more weight since its JA11 support is marked working, not "testing."

**Net effect on Open items below**: the ROADMAP "Ideas" item — EQ/PEQ tuning
from the TUI — is now much closer to a known-good spec than "concrete from one
static RE pass" (§4's original framing): three independent sources (this app,
a shipped WebHID driver, a cross-platform Rust/Tauri app) converge on the same
per-band framing, and the two open, hardware-facing ones converge on master-gain
scaling. What's left before implementing in `ktflash`/`ktctl` is real-hardware
confirmation of (a) master gain ×2560/little-endian over ×10/big-endian, and
(b) the correct save/commit opcode+payload — both single-session checks against
a real JA11, not further static RE.

## 5. Native libraries — not pursued further

- `libapp.so` / `libflutter.so` — Flutter/Dart AOT snapshot. Everything relevant to
  JA11 turned out to live in the native Java/Kotlin layer instead, so this wasn't
  needed. Would require `blutter` (Dart snapshot → pseudocode) if the Flutter side
  ever needs reversing.
- `libjl_ota_auth.so` (Jieli OTA-auth) — used by FIIO's many Jieli-chipset BLE
  products (earbuds, BT amps); **not applicable to JA11**, which has no BT MCU.
- `libRSSupport.so`/`librsjni*.so` — Android RenderScript support libs, unrelated.
- v4.0.0 adds ML Kit barcode-scanning (`libbarhopper_v3.so`) for some unrelated
  QR/pairing feature — unrelated to JA11.

## Open items / next steps

1. Hash-compare the downloaded `JA11.bin` (67,312 B, v2.2) against the local
   `JadeAudio JA11_V2.2.bin` used in `cdc-re-findings.md` to confirm byte-identity
   before treating FIIO's CDN as an equivalent backup source.
2. ~~Pin the exact USB interface/endpoint numbers~~ **resolved** — see §4 Transport:
   descriptor-discovered (`bInterfaceClass==3 && endpointCount==2`), not hardcoded.
3. ~~Confirm the PEQ filter-type enum values~~ **resolved** — see §4: JA11 uses
   `0`=Peak, `1`=LowShelf, `2`=HighShelf (3 of the 7 types FIIO's shared PEQ UI
   supports across its product line).
4. A live USB capture (Wireshark + usbmon, or a rooted-phone / OTG-analyzer setup)
   would upgrade all of §4 from "static-only" to hardware-confirmed, the same way
   §2 was upgraded in `docs/CDC-PROTOCOL.md`.
5. **(new, from §4d)** On real JA11 hardware: confirm master gain (cmd `0x17`)
   is `×2560`/little-endian (per `fiiocontrol-oss`/`glacier-eq`) and not `×10`/
   big-endian (per this doc's original Android-only read); and determine the
   correct save/commit-to-flash opcode — `0x18` payload `[1]` (`glacier-eq`) vs.
   `0x19` payload `[3]` (`fiiocontrol-oss`, JA11-tested) vs. possibly both for
   different purposes.
