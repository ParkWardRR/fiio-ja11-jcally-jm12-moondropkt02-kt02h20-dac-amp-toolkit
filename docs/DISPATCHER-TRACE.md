# NDS32 dispatcher trace + follow-up RE status, Sept 2026

Sub-thread of the RE work tracked in the top-level `HANDOFF.md` (§6, task 3: "chip/firmware
arch profile"). Read `docs/NDS32-CORE-ID.md` first (the foundational writeup: image layout,
how the CPU core was identified as Andes NDS32). This doc covers what happened *after* that —
a batch of follow-up investigations was started, one made real progress before being
interrupted, three did not get past setup. Everything below is either confirmed-with-evidence
or explicitly marked as unconfirmed/guess. Written so a fresh agent/session can pick up this
specific sub-thread cold.

## Where things stand, in one paragraph

The core ID (Andes AndesTar v3 / NDS32) is solid. A deeper trace of the command dispatcher at
`0x83010` just turned up a structural fact that reframes the rest of the work: **this
firmware image only contains the "app" layer.** A bunch of functions the app calls
(`func_0x00024b4c`, `func_0x00016c14`, `func_0x00016c00`, `func_0x000154f8`, and others) live
at addresses below `0x80000` — i.e. *outside* this file's mapped memory (`ram` block is
`0x80000`–`0x906ef`, exactly this file's 67,312 bytes). Those are almost certainly calls into
a **resident vendor SDK / boot ROM** that ships separately (in mask ROM or an earlier flash
region) and is simply not part of this firmware-update `.bin`. That single fact answers the
"where's the interrupt vector table" question (not in this file — see below) and explains why
static peripheral-register mapping from this file alone will hit a ceiling.

## What actually got done

### 1. Dispatcher trace — real progress, not just theory

Working script: `research/DispatchAnalysis.java` (Ghidra headless `GhidraScript`, same
pattern as `research/ScanArch.java`/`ListFuncs.java` — run via
`analyzeHeadless <proj_dir> <proj_name> -process fw.bin -readOnly -noanalysis -scriptPath <dir> -postScript DispatchAnalysis.java`
against a **private copy** of `research/ghidra_proj/`, per the concurrency warning in
`docs/NDS32-CORE-ID.md`).

**Memory layout it revealed:** two blocks — `ram` (`0x80000`–`0x906ef`, the whole loaded
file) and `csr` (`csreg:0000`–`csreg:1fff.3`, uninitialized, non-executable, read-only) — the
NDS32 Ghidra module models Andes' control-status-register space as a *separate address
space*, not `ram`. That's why `setgie()`/`dsb()` decompile as clean intrinsic calls rather
than raw memory writes — they're implemented as pcode-ops against `csr`, exactly matching
real NDS32 hardware (CSRs are a separate instruction-addressed space, not memory-mapped).

**Callers of the dispatcher** (`docs/NDS32-CORE-ID.md` names it `entry_guess`/`FUN_ram_00083010`):

- **`entry_guess` (`0x83000`) has ZERO references to it anywhere in the image.** That name
  was an artifact of the *original* seeding script (`ScanArch.java` called
  `createFunction(0x83000, "entry_guess")` to force analysis to start there) — it is **not**
  evidence of a real entry point. Don't treat `0x83000` as special going forward; the actual
  code entered from outside is the sibling function next door.
- **`FUN_ram_00083010` (`0x83010`) has two real, in-image callers:**
  - **`FUN_ram_00084410(int param_1, uint param_2)`** — a loop over `param_2`-byte input in
    fixed **10-byte records** (`uVar7 = uVar7 + 10 & 0xff`), reading a flag byte, three payload
    bytes, and a 2-byte field per record, doing a small remap of a nibble when a computed
    value falls in range `0x27..0x2f` (`case 1→0x30000, 2→0x40000, 4→0x10000, 5→0x20000`
    written into bits `16-23`), then calling `FUN_ram_00083010(cmd_byte, addr_like, data)` once
    per record. **The 10-byte record size exactly matches the known `0x4b`-report reply size
    and the known `0x54` unlock-frame size from `docs/PROTOCOL.md`.** This is strong (not yet
    100%-certain) evidence that `FUN_ram_00084410` IS the on-chip parser for inbound HID
    vendor reports, and `FUN_ram_00083010` is the actual per-record command dispatcher.
  - **`FUN_ram_00084370`** — an audio-buffer-refill-looking function (decrements a byte
    position through a `0xfa5`-byte region in `0x1bd`-byte steps — `0xfa5 / 0x1bd = 9`
    exactly, so 9 chunks of 445 bytes; logs the debug string `"D user next ilde addr %d"` at
    load address `0x8c7c8`, which matches the debug-string-table offset `0xc7c8` documented in
    `docs/NDS32-CORE-ID.md` **exactly** — nice independent confirmation the file-offset↔
    load-address mapping is right). This function calls `FUN_ram_00083010(0x53, 0, 0)`
    directly — i.e. the dispatcher's `'S'` (start) case is also used as an **internal control
    API**, not only reachable via USB reports.
- The dispatcher's `'W'`/`0x3a` case builds a **5-iteration** table via
  `FUN_ram_000842ec`→`FUN_ram_00083888`/`FUN_ram_000832d0`/`FUN_ram_000835ac`, writing into
  `DAT_ram_000429d8` and `DAT_ram_00042960` (6-word stride each, 5 entries) — almost certainly
  **5-band EQ coefficient computation**, which lines up with the five near-identical
  byte-blobs noticed during initial string-scanning (`docs/NDS32-CORE-ID.md`'s raw hex pass
  saw a repeated 6-byte-ish fragment 5 times ~0x1000 apart). Not fully traced — worth
  finishing if someone wants the actual biquad math.

**The missing-ROM callees**, confirmed unresolved (`checkSymbol()` in the script returned "not
found", and `fm.getFunctionContaining()` returned null for all of these — i.e. Ghidra has no
function *and no memory* at these addresses in the loaded image):
- `func_0x00024b4c` — the "send a 10-byte `0x4b`-framed reply" helper. Called from every
  dispatcher branch. **Reversing the actual reply framing requires the missing ROM code** —
  can't be done from this file.
- `func_0x00016c14` (read) / `func_0x00016c00` (write) — the register accessor pair used for
  the `0xcc0`-offset block in the `'W'`/index-`0x3a` case. Same problem: these are ROM calls,
  not in-image.
- `func_0x000154f8` — called with a single byte arg from two places; unresolved.

**Practical conclusion for anyone continuing the dispatcher work:** the protocol-level
semantics of `'W'`/`'R'`/`'S'`/`'C'`/`9` are now reasonably well understood at the *app*
level (see table below), but the exact wire-level reply bytes and the `0xcc0` register's
actual hardware meaning are gated on getting hold of the ROM code below `0x80000` — see
"Recommended next step" at the bottom.

| Byte | Meaning (app-level, confirmed from decompile) | Confidence |
|---|---|---|
| `'W'` (0x57) | Write: `table[index] = value` at `0x42560 + index*4`; index `0x61..0xe0` also re-triggers a 48kHz-rate call (`func_0x0000d138`); index `0x18-0x23`/`0x24-0x2f` also call `FUN_ram_00084290(1)`/`(2)`; index `0x3a` does a 3-step read-modify-write on the ROM-owned `0xcc0` register (bit 3, bits 0-2, bit 31 in sequence — looks like a commit/latch pattern: set two config fields then a "go" bit); index `0x3b` calls `func_0x000154f8(value)` | High for the table-write path; low for what `0xcc0`/`0x3b` physically control (ROM-gated) |
| `'R'` (0x52) | Read: reply = `table[index]` (4 bytes), framed via `func_0x00024b4c(0x4b, buf, 10)` | High |
| `'S'` (0x53) | Start: `setgie(0)`→`dsb()`→some DMA/buffer init (`func_0x00015d00`+`func_0x00015fd4` against a fixed buffer at `0x82000`, 0x400 bytes)→refcount decrement→`setgie(1)`/`dsb()` once refcount hits 0. Also invoked internally (not just from USB) by the buffer-refill task `FUN_ram_00084370`. | Medium-high |
| `'C'` (0x43) | Close: same critical-section shape as `'S'` minus the `func_0x00015fd4` re-init call — looks like "stop without re-arming" | Medium-high |
| `9` | → `FUN_ram_00085e54(9, param_2, param_3)`, which reads a config word via `func_0x000180e4()`, compares against a cached value, sets/clears bit 24 of a status word accordingly, reads two 0x34-byte config blocks via `func_0x00021864(0/1, ...)`, packs a few fields (a 3-bit field << 28, a sign-ish flag << 31, low 24 bits) into two status words, then replies via the same 10-byte `0x4b` framer. Looks like a **hot-plug/config-change poll** (maybe jack-detect or sample-rate-change ack) — least understood of the five. | Low-medium |

Every case ends by replying with `func_0x00024b4c(0x4b, <10 bytes>, 10)` — so **every**
runtime command on this dispatcher replies on report `0x4b`, matching `docs/PROTOCOL.md`'s
report-ID picture even though the actual reply *payload* framing can't be pinned down without
the ROM.

### 2. Reset/interrupt vector table — effectively answered (not by direct search, but by the ROM-boundary finding above)

No agent got far enough to run a dedicated NDS32-IVT-convention search. But the dispatcher
trace above already answers the practical question: **the vector table is not in this file.**
Evidence: (a) the `ENTY` header declares exactly one load segment (`0x83000`, per
`docs/NDS32-CORE-ID.md`) — consistent with this being an app-only OTA payload, not a full
flash dump; (b) core runtime services (the HID reply framer, register accessors) are called
at fixed low addresses that don't exist in this image at all, meaning there's a whole
resident lower layer (very plausibly starting near `0x0` and including the real reset vector,
NDS32 IVB-configured exception table, and the vendor RTOS/HAL) that ships in the chip
separately and this update only overlays the `0x83000`-based app region on top of it.
**Nothing more to search for in `JA11_V2.2.bin` on this question** — don't spend more time
re-deriving this from the app image; the only way forward is getting the missing ROM (see
below).

### 3. `nds32le-elf-gcc` toolchain — not started

The agent assigned to this only got as far as making a private Ghidra project copy
(`/tmp/ja11-re-toolchain`, if it still exists — may have been cleaned up) and never began the
actual toolchain install. **Whoever picks this up should start from scratch** using the
original task brief: check Homebrew (unlikely), check `andestech/nds-gnu-toolchain` /
`andestech/Andes-Development-Kit` GitHub releases for a usable prebuilt, and if nothing works
natively on macOS/arm64, use an OrbStack Ubuntu VM (check `orb list` first — a `ktflash` VM
may already exist from the earlier CDC-protocol RE work, reuse it if so) to build or install
`nds32le-elf-gcc`, then compile a test program and compare codegen idioms (register/`$gp`
usage, calling convention) against the real firmware's decompile. Full original brief is
still valid — nothing here changes it.

### 4. Peripheral register map — not started, and now known to be partially blocked

Same situation as #3: the agent only made a private project copy, no script was written.
**Update the original brief** with the ROM-boundary finding: most peripheral access in this
app image goes through the *unresolved* ROM helpers `func_0x00016c14`/`func_0x00016c00`
(generic register read/write) rather than touching MMIO addresses directly, so a purely
static pass over `JA11_V2.2.bin` will only find a **partial** register map. What IS directly
visible and confirmed in-image (worth building on):
- `_DAT_ram_c00121c0` — read-modify-write, `|= 0x80000000` pattern (single high bit set,
  classic "commit"/"busy"/"go" bit), touched from the dispatcher's generic reply path
  (`entry_guess`/`FUN_ram_00083010`'s tail, after most command types).
- `_DAT_ram_c0021034` — read-modify-write, `& 0xc00fffff | computed_field | 0x80000000` from
  `FUN_ram_000845f8` (a small lookup-table-driven scaling function) — bits 20-29 look like a
  computed field, bit 31 the same "go" pattern as above.
- Both addresses are in the `0xc00xxxxx` range — that's very likely this chip's
  memory-mapped peripheral window (separate from the `0xcc0`-relative block, which is
  ROM-proxied and might be a *different* numbering scheme — e.g. `0xcc0` could be a
  ROM-internal register-table index rather than a raw address).
A future pass should (a) grep the WHOLE decompiled function set (not just the dispatcher
subtree already covered) for every `_DAT_ram_c0*` reference, not just these two, and (b) not
expect completeness — flag ROM-mediated access clearly as "needs the ROM" rather than
guessing blind.

## Recommended single next step, if you only do one thing

**Get the missing ROM/boot code.** Three unresolved threads above (the exact `0x4b` reply
framing, the real IVT, and full peripheral mapping) are all blocked on the same missing
piece: whatever lives below load address `0x80000` on the real chip. The existing `flasher/`
CLI (`docs/PROTOCOL.md`'s `0x08` read-word command) can read arbitrary 32-bit words from the
device over USB — in principle a full low-address dump is possible by walking `0x08` reads
from `0x0` upward against real hardware (the JA11 dongle from earlier sessions). This needs
actual hardware in hand and is out of scope for a docs-only agent, but it's the one action
that unblocks everything else on this list. If you have the hardware, that's the highest-value
next move; if not, the toolchain (#3) and finishing the EQ-coefficient trace (the 5-iteration
loop in §1) are the next best uses of time since they don't need the ROM.

## Reproducing the analysis environment

Same as `docs/NDS32-CORE-ID.md`: Ghidra `/opt/homebrew/Cellar/ghidra/12.1.3/libexec`, base
address `0x00080000`, language `NDS32:LE:32:default`, source `research/ghidra_proj/JA11_V2.2.bin`
(gitignored, regenerate the Ghidra project by importing it fresh if you're on a clean clone —
`research/ghidra_proj/` itself won't be present). Always work on a **private `/tmp` copy**
if you're going to run headless analysis — don't touch a shared project if other work might
be happening concurrently, learned the hard way when four agents were launched in parallel
against the same investigation.
