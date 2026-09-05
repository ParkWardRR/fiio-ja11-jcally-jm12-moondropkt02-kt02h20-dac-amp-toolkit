<div align="center">

# ⚡ ktflash · KTMicro KT02H20 · FiiO JA11 toolkit

**Identify and cross‑flash the tiny USB‑C DAC/amp dongles built on KTMicro `KT02H20`
(FiiO JA11, JCALLY JM12, and clones) — natively from a Mac, with a pretty TUI.**

![ktflash TUI demo](assets/demo.gif)

<p>
  <img alt="License: Blue Oak 1.0.0" src="https://img.shields.io/badge/license-Blue%20Oak%201.0.0-1a73e8">
  <img alt="Rust" src="https://img.shields.io/badge/built%20with-Rust-dea584?logo=rust&logoColor=white">
  <img alt="macOS + OrbStack" src="https://img.shields.io/badge/target-macOS%20%2B%20OrbStack-000?logo=apple&logoColor=white">
  <img alt="Native write: proven" src="https://img.shields.io/badge/native%20write-proven%20on%20hardware-2ea44f">
  <img alt="No firmware backup" src="https://img.shields.io/badge/firmware%20backup-not%20possible-b60205">
</p>

</div>

> [!CAUTION]
> **Flashing can brick your dongle, and there is NO way to back up its firmware first.**
> `ktflash` cannot read firmware *off* a KT02H20 in software — confirmed by full
> reverse‑engineering ([details](docs/CDC-PROTOCOL.md)). If a cross‑flash goes wrong, the dongle
> will be bootloader‑only, but the KT_USB_BOOT ROM survives an app‑flash — **you can retry
> with any compatible firmware image**, not just the original. That said, a wrong or mismatched
> image will fail to boot, so **save a known‑good image before you start** and **proceed at your
> own risk**. `ktflash flash-cdc` refuses to write without `--yes` for exactly this reason.

> [!NOTE]
> **No Windows, no VM, no container required — on macOS *or* Linux.** The end‑to‑end native
> *write* is ✅ **proven on hardware** three independent ways (2026‑09‑05): from a **Mac +
> OrbStack** (`ktflash flash-cdc`, libusb, no Mac mini/Windows/vendor tool); fully
> **Linux‑native** (same command, serial transport, a Debian 13 VM, no OrbStack); and fully
> **macOS‑native** — `ktmac unlock` (a small Swift companion, `IOHIDManager`) plus `ktflash
> flash-cdc --transport serial` reflashed the stock `JA11_V2.2.bin` with **zero OrbStack
> involvement**. That last one overturned this project's own earlier assumption that macOS
> couldn't drive the unlock at all — see [`docs/MACOS-NATIVE.md`](docs/MACOS-NATIVE.md).
> `ktmac` isn't merged into `ktflash` yet, so OrbStack remains the simplest single‑binary path
> for now.

---

## 🎧 What & why

Those pocket USB‑C dongles are little computers that make a phone/laptop drive real headphones.
Tons of cheap ones use the **same KTMicro `KT02H20`** chip as the FiiO JA11 — so the JA11's
firmware (better DSP/PEQ) can be **cross‑flashed onto them**. This toolkit reverse‑engineers the
protocol and gives you a clean Rust tool + a live TUI to do it from a Mac.

| You want to… | Go |
| --- | --- |
| 🔎 **Identify** my dongle | run `ktflash` (the TUI up top) |
| ⚙️ **Flash** it (native) | [FLASHING.md](docs/FLASHING.md) · [will mine work?](docs/COMPATIBILITY.md) |
| ↩️ **Revert** to stock | [REVERT.md](docs/REVERT.md) — *bring your own stock image; no dump exists* |
| 🛠️ **See how it was reversed** | [CDC-PROTOCOL.md](docs/CDC-PROTOCOL.md) · [EXTRACTION.md](docs/EXTRACTION.md) |
| 🗺️ **See the plan** | [ROADMAP.md](ROADMAP.md) — done vs. next |

---

## 🚀 Quickstart

```bash
# build (needs libusb: `brew install libusb`)
cd flasher && cargo build --release

# identify — pretty, live, read-only, native macOS
./target/release/ktflash                 # TUI dashboard (shown above)
./target/release/ktflash probe           # one-shot report

# flash path: macOS → OrbStack Linux guest (reaches the USB endpoints macOS blocks)
./orbstack/ktflash-orbstack.sh setup     # create the guest + deps
./orbstack/ktflash-orbstack.sh attach    # pass the dongle through

# ⚠️ SAVE A KNOWN-GOOD IMAGE FIRST — there is no read-back. Then, inside the guest:
ktflash flash-cdc --image fw.bin         # DRY RUN — prints the packet plan, touches nothing
ktflash unlock                           # → fresh CDC bootloader (8888:cdc0)
ktflash flash-cdc --image fw.bin --execute --yes   # writes ( --yes = "I know the risks" )
```

Full flow + the honest per‑step status → [docs/FLASHING.md](docs/FLASHING.md).

---

## 🚦 Where it runs

| Host | Identify | Unlock | Native write |
| --- | --- | --- | --- |
| **macOS (native, `ktflash` only)** | ✅ | ❌ `rusb` can't claim the interface | ❌ |
| **macOS (native, + `ktmac`)** | ✅ | ✅ **proven** (`ktmac unlock`, `IOHIDManager`) | ✅ **proven** (`flash-cdc --transport serial`) |
| **macOS + OrbStack** | ✅ | ✅ | ✅ **proven** (`flash-cdc`, libusb) |
| **Linux (native)** | ✅ | ✅ | ✅ **proven** (`flash-cdc`, serial transport — Debian 13) |

`ktflash` itself still can't claim the interface for `unlock` on macOS (`IOHIDFamily` owns it) —
that part of the old assumption was correct. What changed: a **separate, tiny path**
(`IOHIDManager` instead of libusb, in the `ktmac` companion tool) reaches the device anyway, no
interface claim needed, and the bootloader it triggers is a plain serial device `ktflash` already
drives natively. OrbStack remains the simplest **single‑binary** path until `ktmac` is merged in
— see [`docs/MACOS-NATIVE.md`](docs/MACOS-NATIVE.md) for the full story and open item.
**Linux needs no detour at all**: install the [udev rules](packaging/99-ktflash.rules) and
`ktflash` drives everything over the CDC‑ACM tty directly. Prebuilt binaries (both OSes) are ⏳
next — for now, `cargo build --release` from source. Full validation notes (including a real
RHEL/AlmaLinux limitation with USB/IP test rigs) → [ROADMAP Phase 4](ROADMAP.md).

---

## 🌱 How it works, gently

1. `ktflash` **unlocks** the dongle with a magic HID string — report `0x54` `"T12345678"`.
2. That **reboots it into a bootloader** that appears as a USB serial device (`0x8888:0xCDC0`).
3. `flash-cdc` streams the firmware to that bootloader in CRC‑checked 1 KB packets
   (`KTM → CHP → erase → PWO → KSTA → data×N → STP → RESET`), and it reboots into the new firmware.

Every byte was recovered by disassembling the vendor's own tool — no USB capture needed. The
full spec is in [docs/CDC-PROTOCOL.md](docs/CDC-PROTOCOL.md) and the RE writeup in
[research/cdc-re-findings.md](research/cdc-re-findings.md).

> **Why no backup?** The bootloader has no read command, and the runtime firmware's control
> channel only reads EQ coefficients — never flash. Reading the original firmware would need
> hardware (JTAG/SWD, chip‑off, or a factory ISP). See [ROADMAP Phase 5](ROADMAP.md#phase-5--firmware-backup--readback--not-possible-in-software).

---

## 🎚️ Device roster

Full table + how to vet a candidate → [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md).
**Reverting any of these needs a stock image you supply — there is no way to dump the original.**

| Dongle | USB ID | JA11 flash |
| --- | --- | --- |
| **FiiO / JadeAudio JA11** | `2972:0102` | 🟢 Official target · native write proven |
| **JCALLY JM12** | KTMicro rebrand | 🟢 Community‑confirmed |
| **Moondrop KT02H20 dongle** | `31B2:0111` | 🟢 **Confirmed** → `2972:0102` |
| **Fransun T2 Pro (KT02H20 ed.)** · **VE ODO** | rebrand | 🟡 Reported |
| **Kiwi Ears AD1** | rebrand | 🔴 No verified flash — avoid |

---

## 📚 What's inside

| Path | What |
| --- | --- |
| [`flasher/`](flasher/) | Rust `ktflash` — TUI + `probe`/`unlock`/`bootdiag`/`flash-cdc`/`image`/`demo` |
| `flasher/src/proto/ktcdc.rs` | Byte‑exact CDC download framing (header/CRC/packet planner), unit‑tested |
| [`orbstack/`](orbstack/) | macOS → Linux‑guest driver (`ktflash-orbstack.sh`) |
| [`docs/`](docs/) | [CDC-PROTOCOL](docs/CDC-PROTOCOL.md) · [FLASHING](docs/FLASHING.md) · [COMPATIBILITY](docs/COMPATIBILITY.md) · [REVERT](docs/REVERT.md) · [NDS32-CORE-ID](docs/NDS32-CORE-ID.md) |
| [`research/`](research/) | `cdc-re-findings.md` + the headless Ghidra scripts used to reverse it |

---

## 🧭 Status & help wanted

- ✅ **Protocol fully reversed + native write PROVEN on hardware, three ways** — `ktflash
  flash-cdc` did a complete reflash of the stock `JA11_V2.2.bin` from a Mac + OrbStack (`v1.1.0`),
  fully Linux‑native over the serial transport on a Debian 13 VM, and fully **macOS‑native** (no
  OrbStack at all, via the `ktmac` companion for `unlock`) — all 2026‑09‑05. The macOS‑native
  result overturned this project's own earlier "macOS can't drive the unlock" assumption; see
  [`docs/MACOS-NATIVE.md`](docs/MACOS-NATIVE.md) and [ROADMAP Phase 4](ROADMAP.md).
- ❌ **Firmware backup is not possible in software** on the KT02H20 (no read command; the
  normal‑mode reader is inert on this silicon) — keep your original image. [Why.](docs/CDC-PROTOCOL.md)
- 🚧 **Next:** merge `ktmac`'s native unlock into `ktflash` itself (currently two binaries);
  prebuilt binaries (Linux + macOS); more KT02H20 dongles; AlmaLinux/RHEL hardware validation on
  a native (non‑USB/IP) machine.
- 🟡 **Wanted:** before/after descriptors from any dongle you flash; **PCB photos / JTAG‑SWD pad
  locations** (the only path to a real backup); a JCALLY JM12 + its stock image.

Full plan (done vs. next) → [ROADMAP.md](ROADMAP.md). Tooling is **Rust + shell** (no Python).

---

## ⚖️ License & disclaimer

[Blue Oak Model License 1.0.0](LICENSE.md)**.** Independent interoperability research; not
affiliated with KTMicro, FiiO, JadeAudio, JCALLY, Moondrop, etc. **No firmware images are
distributed here.** Flashing hardware you own is entirely at your own risk: it **can brick the
device**, and because there is **no firmware backup path**, recovery depends on you having a
compatible image on hand. See the **No Liability** clause.
