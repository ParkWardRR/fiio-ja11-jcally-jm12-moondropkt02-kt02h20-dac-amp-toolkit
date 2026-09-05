# JadeAudio JA11_V2.2.bin — architecture ID + initial RE pass

Sept 2026. Firmware: `JadeAudio JA11_V2.2.bin`, 67,312 B, header magic `KT_Helios_v1b`, chip
`KT02H20B`. Companion to the CDC bootloader protocol RE in the `ktflash` toolkit
(`docs/CDC-PROTOCOL.md`), which covers how to get *bytes* onto the chip. This covers what's
*inside* those bytes.

## Header / image layout (confirmed)

| Offset | Field | Value |
|---|---|---|
| 0x00 | magic | `KT_Helios_v1b\0\0\0` |
| 0x10 | chip id | `KT02H20B` |
| 0x18 | `Size` (u32 LE) | `0x000106f0` = 67312 — matches actual file size exactly |
| 0x20 | build hash | `75503ea` |
| 0x30 | build date/ver string | `2025-06-30 V:1.0` |
| 0x40 | `ENTY` table tag | `ENTY` |
| 0x44 | load address (u32 LE) | `0x00083000` |
| 0x48 | entry size (u32 LE) | `0x0001d188` (target-space size incl. erase-block padding; larger than the file — not a file-offset length) |
| 0x50–0x2FFF | zero padding | — |
| 0x1000–0x1140 | build-info strings | `2.2.0`, `Jul 8 2025`, `19:24:29`, `PerfCfg:`, `JadeAudio JA11`, `2020-02-20-0000-0000-0000` |
| 0x3000–0xc7b0 | **code + rodata** | mapped to `0x00083000`–`0x0008c7b0` (see below) |
| 0xc7b0–0x106f0 | **debug log string table** | printf-style strings, see below |

**Key mapping: file offset `0x3000` = load address `0x00083000`.** I.e. base address for the
raw image = `0x00080000`, header occupies the low 0x3000 (unmapped/not part of the running
image — it's the flasher's own metadata, stripped before load).

## CPU core: confirmed **Andes AndesTar v3 (NDS32)** — not ARM, not RISC-V

KTMicro doesn't publish the KT02H20's core anywhere public. I brute-forced it empirically:
imported the raw image into Ghidra at the base above and ran full auto-analysis under every
ISA Ghidra supports (ARM Cortex-M/thumb, RISC-V32/AndeStar_v5, MIPS/16e, TriCore, Xtensa,
8051, NDS32, ...). Byte-coverage of the ~38 KB code region and decompiler sanity:

| Candidate | Bytes covered | Functions found | Verdict |
|---|---|---|---|
| ARM:LE:32:Cortex | ~11% | 5, riddled with bogus `coprocessor_moveto2`/`halt_baddata` | wrong |
| RISCV:LE:32:default | ~6% | 17, mostly "unable to resolve constructor" errors | wrong |
| RISCV:LE:32:AndeStar_v5 | ~5% | 15, same failure mode | wrong |
| MIPS / MIPS16e / TriCore / Xtensa | <5% | 1–2, garbage | wrong |
| **NDS32:LE:32:default** | **~94%** | **95, clean decompiles** | **correct** |

The NDS32 decompile is unambiguous — not just "fewer bad instructions" but semantically
sane C with real Andes toolchain intrinsics showing up by name: `setgie(0)` / `setgie(1)`
(Andes' `__nds32__setgie_en`/`dis` — Set/clear Global Interrupt Enable) and `dsb()` (data
sync barrier), bracketing what are obviously critical sections. No other ISA candidate
produced anything semantically coherent.

**Practical upshot:** KT02H20 = an Andes AndesTar v3 32/16-bit mixed-length RISC core. This
is a *real, documented, licensable* ISA (not a closed in-house core like the JieLi/Actions
chips used in some sibling USB-audio dongles, which have no public disassembler at all).
Ghidra ships full support for it out of the box. And there's a genuinely usable open toolchain path: `nds32le-elf-gcc` was upstreamed into
mainline GCC (since 4.9) and binutils (2.25) by Andes itself, and Andes still publishes
prebuilt toolchains + build scripts at `andestech/nds-gnu-toolchain` on GitHub (also packaged
in Arch Linux as `nds32le-elf-gcc`). This is the realistic path to eventually **building**
custom firmware, not just reading it.

## What's already legible

- Entry point at `0x83000` is a dispatcher (`entry_guess` in the project) that switches on a
  single command byte and fans out to `FUN_ram_00083010` (the same dispatch logic, called with
  explicit params — looks like an ISR-context wrapper calling a plain-C-callable twin).
  Command bytes seen: `'W'` (0x57 — write, drives per-address gain/register table updates,
  guards a `0x18`/`0x24`-relative special-case band), `'R'` (0x52 — read a table entry),
  `'S'` (0x53 — start, calls into a DMA/DSP init pair guarded by `setgie`/`dsb`), `'C'` (0x43 —
  close/stop, mirrors `'S'`), and `9` (dispatches to `FUN_ram_00085e54`). This is very likely
  the **runtime HID vendor-protocol handler** (the `0x4b`/`0x54`-framed EQ/tuning path
  described in `docs/PROTOCOL.md`) — worth confirming against the report-ID framing next.
- RTOS with named tasks (`top_task`, `usb_task`, `adc_task`, `dac_task`, `eq_task`,
  `keys timer`, `itrim timer`) and leveled debug logging (`D`/`I`/`E` prefixes,
  `%d`/`%x`/`%s`-style printf) compiled in — strings live at `0xc7b0`–end of file.
  `bll_au*` naming (`bll_auusb`, `bll_auin`, `bll_auout`, `bll_autop`) is almost certainly a
  KTMicro-internal SDK module-namespace convention ("BLL" = some internal abstraction layer,
  "au" = audio) — not found published anywhere, so treat as a KTMicro-only symbol, not a
  reusable open component.
- Button/earphone-jack state machine strings (`AUTOP_ActiveEnterStdbyFunc`, `VOL_UP`/`VOL_DN`/
  `MUTE`, `SINGLE`/`DOUBLE`/`TRIPLE`/`LONG` click detection, `eartp cfg is 3Pole`/`Ctia`/`Omtp`)
  — a full inline-remote/earphone-type autodetect + gesture layer.

## Ghidra project

`research/ghidra_proj/JA11_V2.2.gpr` / `.rep` (plus the source `JA11_V2.2.bin` alongside it) —
open with `ghidraRun`. **Gitignored on purpose** (this repo's rule is to never commit vendor
firmware or derived binary blobs — same reason `*.bin`/`*.exe` are excluded); the project is
local-only, regenerate it from `JA11_V2.2.bin` if you're on a fresh checkout (Import: base
`0x00080000`, language `NDS32:LE:32:default`). Coverage of the `0x83000`–`0x8c7b0` code region
is ~94% (95 functions); the remaining gaps (`research/ja11-v2.2-funcs-report.txt`, `GAP` lines,
~2.4 KB total, mostly 20–110 B runs) are very likely literal data tables (jump tables / EQ
coefficient arrays) sitting between functions, not missed code — worth eyeballing in the GUI
but low priority.

`research/ScanArch.java` and `research/ListFuncs.java` are the Ghidra headless scripts used to
do the architecture sweep and produce the function/gap report (same pattern as
`research/DecompHID.java`) — reusable for re-running analysis after further manual work
(rename functions, retype structs, etc. — those persist in the `.rep` project, these scripts
don't need to be re-run unless you want a fresh sweep).

## Suggested next steps (not yet done)

1. Open the project in the Ghidra GUI and manually walk the `0x83000` dispatcher against the
   already-known `0x4b`/`0x54` HID report framing from `docs/PROTOCOL.md` to nail
   down the full runtime command set (only `W`/`R`/`S`/`C`/`9` seen so far in a shallow pass).
   That's the piece that actually matters for "write your own firmware that speaks the same
   protocol the stock app expects" without redoing USB enumeration/descriptors from scratch.
2. Find the real reset vector / interrupt vector table — `0x83000` decompiled as a sane
   function, not obviously a hardware IVT, so the true entry point may be elsewhere in the
   image (check low addresses right after the header, or look for a table of `j`/branch
   instructions Ghidra's analyzer flagged as data).
3. Get `nds32le-elf-gcc` installed locally (Arch package is easiest as a reference for exact
   flags/triple; `andestech/nds-gnu-toolchain` for building it directly on macOS/Linux) and
   confirm it round-trips: compile a trivial NDS32 stub, diff its codegen idioms against this
   firmware's, to validate the compiler-ABI assumptions before attempting anything real.
4. Map the peripheral register addresses referenced in the decompiled functions (e.g.
   `_DAT_ram_c00121c0`, the `0xcc0`-offset block accessed via `func_0x00016c14`/`func_0x00016c00`
   read/write helpers) against *something* — there's no public KT02H20 register manual, so this
   will likely have to be inferred from behavior (probe with the existing `ktflash` read/write
   primitives) rather than documented.

Full "write your own firmware" is a real multi-session project from here — the ISA/toolchain
question (the part with no public answer) is now resolved; what's left is normal, if tedious,
peripheral-map reverse engineering.
