# Dongle compatibility — which KT02H20 devices take FiiO JA11 firmware

The FiiO/JadeAudio **JA11** firmware targets the **KTMicro `KT02H20`** DAC SoC. Note: FiiO has
never officially confirmed the exact die — the KT02H20 identification is widely held based on
register‑mapping and community RE, not a manufacturer statement. Many cheap USB‑C dongles use the
same chip, and several have been cross‑flashed to JA11 successfully — gaining FiiO's DSP/PEQ and
(usually) re‑enumerating as `2972:0102 "JadeAudio JA11"`.

> [!CAUTION]
> **Same chip is not enough.** Jack wiring, mic ADC, USB descriptors, oscillator/clock
> settings, calibration, and flash layout can differ. A wrong‑hardware flash can mute
> output, kill the mic, or brick until you reflash. Treat everything below "High"
> confidence as *experiment at your own risk*, and have a recovery path
> ([`REVERT.md`](REVERT.md)) ready.

## Roster

| Device | JA11 flash status | Confidence | Notes | Typical price |
|---|---|---|---|---|
| **FiiO / JadeAudio JA11** | Officially supported | **High** | The intended target for FiiO's web/local update tools and v2.2 firmware. | ~$10–18 |
| **JCALLY JM12** | Repeated successful community cross‑flashes | **High** (for a non‑FiiO device) | Widely reported to enumerate as a JA11 afterward and gain FiiO DSP/PEQ. Uses KT02H20. | ~$2–15 |
| **Audiocular A16x** | Repeated successful community cross‑flashes | **High** | Frequently cited alongside JM12 as a confirmed KT02H20 flash target; direct community reports of successful flash via FiiO tool. | ~$10–20 |
| **Audiocular C18** (USB‑C cable dongle) | Repeated successful community cross‑flashes | **High** | Same chip family as A16x; repeatedly named as a confirmed flash target in community sources. | ~$10–20 |
| **Moondrop KT02H20 dongle** | ✅ Confirmed in this repo | **High** | The unit flashed here (`31B2:0111 "KT02H20 HIFI Audio"`, dmesg "MOONDROP JU Jiu") → `2972:0102 "JadeAudio JA11"`. Descriptor strings are first‑party data — unverified by external sources. | ~$5–12 |
| **Fransun T2 Pro — KT02H20 edition** | Successful community cross‑flashes, step‑by‑step tutorials | **Medium‑High** | Full Russian‑language tutorial and a Vietnamese firmware‑mod post both confirm the flash via FiiO tool + JA11 v2.2, mirroring the JM12 process. **Confirm it's the KT02H20 version** before touching it. | ~$4–10 |
| **Venture Electronics VE ODO** | Reported compatible | **Medium** | Direct Reddit report of a successful ODO(+) → JA11 flash; VE ODO listed as a supported cross‑flash target in a community firmware post alongside JA11/JM12/Fransun. Hardware revisions may matter. | ~€5 / ~$6–10 |
| **KBear TC12** | Likely compatible, untested | **Low‑Medium** | Called out by a detailed flashing guide as a probable JM12 rebrand, but explicitly untested by that author. Treat as unverified. | ~$5–15 |
| **Unknown generic KT02H20 adapters** | Possible, not assumed | Low | Verify the chip *and* behaviour first. Descriptors/wiring/calibration may differ. | ~$3–10 |
| **Kiwi Ears AD1** | No verified JA11 flash found | Unknown / avoid | Has KT02H20 (manufacturer spec‑confirmed), but no known successful JA11 flash. Community reports show it flashed to a different vendor's ("Zoofio G11") firmware — JA11 specifically is uncharted. Don't assume compatible from the chip alone. | ~$9–15 |

## How to check a candidate before flashing

1. **Fingerprint it.** Plug it into the Mac and run:
   ```
   ./orbstack/ktflash-orbstack.sh attach
   ./orbstack/ktflash-orbstack.sh probe
   ```
   A KTMicro device shows **VID `0x31B2`** (or a rebrand VID) with a USB‑Audio interface +
   a vendor **HID** interface carrying report IDs `0x4B`/`0x54`. That HID + the KTMicro VID
   is the signature this firmware family expects.
2. **Confirm the chip family**, not just "KT02H20 on the box." The firmware header targets
   `KT02H20B`; a different KTMicro die (e.g. a 24/96‑only part like the KZ C04's) is a
   different animal.
3. **Have stock to go back to.** If you can't source a matching stock image, don't flash a
   device you care about. See [`REVERT.md`](REVERT.md).

## Contributing a data point

Flashed something (or bricked it)? Open an issue with: the **before** and **after** USB
`VID:PID` + product string, the exact firmware used, and whether audio/mic/charging still
work. Real before/after descriptors are worth more than a "yes it worked."
