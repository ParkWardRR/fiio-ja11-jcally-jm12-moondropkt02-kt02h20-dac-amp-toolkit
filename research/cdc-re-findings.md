# JA11 CDC bootloader — byte-exact RE findings

Static RE only (no hardware). Decompiled with Ghidra 12.1.3 headless against
`/tmp/ghidra_proj/ja11b` (the imported vendor `JadeAudio JA11 Upgrade Tool.exe`).
Full decompiles saved at `/tmp/re_decomp.txt` and `/tmp/re_helpers.txt`.

Target functions (all 32-bit x86, __thiscall/__fastcall):
- `FUN_0054d7d0` — normal data-packet builder
- `FUN_005733a0` — FINAL packet builder
- `FUN_00b8c380` — response-driven state machine (`SerialRecive`)
- `FUN_00b8f200` — idle/timer state machine
- `FUN_00d2aa60` — ACK checker (`QByteArray::indexOf(char)`)
- helpers: `FUN_006cc3d0` = `QByteArray::insert(pos,ptr,len)`, `FUN_006cbaf0` =
  `QByteArray::append(ptr,len)`, `FUN_00d295d0` = `QByteArray::mid(src,pos,len)`,
  `FUN_006cf5b0` = `QByteArray(ptr,len)` ctor, `FUN_00b8bfd0` = serial send wrapper.

---

## 1. The 6-byte data-packet header  (CONFIDENCE: HIGH)

### Packet layout
```
+----------------+-------------------+----------------------+
| 6-byte header  |  payload (L bytes)|  CRC-32 (4 bytes LE) |
+----------------+-------------------+----------------------+
```
Built in `FUN_0054d7d0`: the payload QByteArray is filled first (via `mid`), the
6-byte header is **prepended** with `insert(0,&hdr,6)`, then the CRC-32 is
**appended**.

### The header is a 6-byte stack blob: `local_22` (4 bytes) + `local_1e` (2 bytes)
Byte indexing below is little-endian within those vars, i.e. `H[0]` = `(byte)local_22`
… `H[3]` = `local_22>>24`, `H[4..5]` = `local_1e`.

Evidence (FUN_0054d7d0):
```c
local_22 = 0x69;                       // H[0] = 0x69 marker; H[1..3]=0
local_1e = 0;                          // H[4..5]=0
...
// address low byte -> H[3]; address bits 8..23 -> H[4],H[5]
param_4 = param_3 * 0x400 + param_4;                 // (non-first block)
local_22 = CONCAT13((char)param_4,(undefined3)local_22);   // H[3] = addr & 0xff
local_1e = (undefined2)((uint)param_4 >> 8);               // H[4]=addr>>8, H[5]=addr>>16
...
if (iVar3 < 0x401) {                    // iVar3 = payload length L (QByteArray size)
   local_22._0_2_ = CONCAT11((char)iVar3,(byte)local_22);          // H[1] = L & 0xff
   local_22._0_3_ = CONCAT12((byte)((uint)iVar3>>8)&0x1f | H2&0xe0, ...); // H[2] low5 = (L>>8)&0x1f
}
LAB_0054d891:
if (param_3 % (byte)(&DAT_013e90c4)[param_2] == 0)
   local_22._2_1_ = H2 & 0x1f | (byte)(param_2 << 5);   // H[2] top3 = param_2 (bank id)
else
   local_22._2_1_ = H2 | 0xe0;                          // H[2] top3 = 0b111 (continuation)
FUN_006cc3d0(0,&local_22,6);            // prepend 6-byte header
```

### Exact 6 header bytes
| Byte | Value |
|---|---|
| H[0] | `0x69` (constant marker) |
| H[1] | `L & 0xFF`  (payload length, low 8 bits) |
| H[2] | `((L >> 8) & 0x1F)  \|  (top3 << 5)` |
| H[3] | `addr & 0xFF` |
| H[4] | `(addr >> 8) & 0xFF` |
| H[5] | `(addr >> 16) & 0xFF` |

- **Length L** is 13 bits: 8 bits in H[1] + 5 bits (0..4) in H[2]. Max seen 0x400.
- **addr** is a 24-bit little-endian field spread over H[3],H[4],H[5].
- **top3** (H[2] bits 5..7) is: the `param_2` bank index on region-boundary packets
  (`block % blockcount == 0`), otherwise `0b111` (=> H[2] high nibble `0xE0`), which is
  the normal "continuation" / final marker. `DAT_013e90c4 = {0x04, 0x20, 0x40, ...}`
  is a per-bank block-count table; at the call sites `param_2 == 1` so `blockcount = 0x20`
  (a 32 KB = 32×1KB region), and the boundary marker value is `1<<5 = 0x20`.

### Address / length derivation (from the two call sites in FUN_00b8c380)
Data packets are built as
`FUN_0054d7d0(out, /*param_2*/1, /*param_3=block*/blk, /*param_4=base*/ (flag<<15), src)`
where `flag = *(byte*)(this+0x38)` is 0 or 1, so `base = 0 or 0x8000`.

- **First block (`block==0`)**: payload copied from `src[0x10..0x10+0x3f0]` → **L = 0x3F0 (1008)**,
  `addr = base + 0x10`.  (`FUN_00d295d0(param_5,0x10,0x3f0)`)
- **Other blocks**: payload = `src[block*0x400 ..]`, clamped to `L = min(remaining, 0x400)`,
  `addr = base + block*0x400`.
- The `L > 0x400` path resizes to 0x400 and forces H[2] low5 = 4 (`& 0xffe0ffff | 0x40000`),
  i.e. `L = 0x400`.

So the firmware's first 16 bytes are **skipped** by the data loop and written **last**
by the final packet (see below).

### FINAL packet — FUN_005733a0  (CONFIDENCE: HIGH)
Called as `FUN_005733a0(out, /*param_2=addr*/(flag<<15), /*param_3*/src)`.
```c
local_22 = 0x69; local_1e = 0;
FUN_00d295d0(param_3,0,0x10);           // payload = src[0..0x10]  (the 16-byte image header)
local_22 = CONCAT13((char)param_2, CONCAT21(0xe010,(undefined1)local_22));
                                        // H[0]=0x69, H[1]=0x10, H[2]=0xe0, H[3]=addr&0xff
local_1e = (undefined2)((uint)param_2 >> 8);   // H[4]=addr>>8, H[5]=addr>>16
FUN_006cc3d0(0,&local_22,6);
```
=> FINAL header = `69 10 E0 <addrLo> <addrMid> <addrHi>`, payload = the image's first 16
bytes, addr = `flag<<15`. It is the same layout with **L fixed = 0x10** and **top3 = 0b111
(0xE0)**. This is the block that makes the image bootable, written last.

### CRC-32 — scope & endianness  (CONFIDENCE: HIGH — CORRECTS PRIOR DOC)
The CRC loop runs **after** the header is prepended and iterates over `puVar4[1]`
(= current QByteArray size = **header(6) + payload**), then appends 4 bytes:
```c
FUN_006cc3d0(0,&local_22,6);            // header now in the buffer
puVar4 = *param_1; uVar5 = puVar4[1];   // uVar5 = 6 + L
local_28 = 0;                           // <-- init 0
do { bVar1=*p++;
     local_28 = local_28>>8 ^ *(uint*)(&DAT_0120e020 + (byte)((byte)local_28 ^ bVar1)*4);
} while (p != end);                     // over all uVar5 bytes; NO final XOR
FUN_006cbaf0(&local_28,4);              // append CRC, little-endian (x86 memory order)
```
- **Scope: CRC-32 is computed over `header(6) + payload`, NOT payload-only.** (The prior
  `CDC-PROTOCOL.md` claim "computed over the payload" is **wrong** — the insert of the
  6-byte header happens before the CRC loop and the loop length includes it.)
- **Algorithm**: reflected CRC-32, table `DAT_0120e020` (poly 0xEDB88320), **init = 0x00000000**,
  **no final complement (xorout = 0)** — note this differs from the standard zip/PNG CRC-32
  which uses init 0xFFFFFFFF and xorout 0xFFFFFFFF.
- **Endianness**: appended little-endian (low byte first), 4 bytes.

### Reference Rust encoder
```rust
/// Reflected CRC-32, poly 0xEDB88320, init=0, xorout=0 (matches DAT_0120e020 usage).
fn crc32_kt(data: &[u8]) -> u32 {
    let mut crc: u32 = 0;
    for &b in data {
        crc = (crc >> 8) ^ CRC32_TABLE[((crc as u8) ^ b) as usize];
    }
    crc
}

/// Build a normal data packet.
///   payload: <= 1024 bytes
///   addr:    24-bit flash address for this payload
///   top3:    bank id (0..6) on a region-boundary packet, else 0b111 (continuation)
fn build_data_packet(payload: &[u8], addr: u32, top3: u8) -> Vec<u8> {
    assert!(payload.len() <= 0x400);
    let l = payload.len() as u32;
    let mut pkt = Vec::with_capacity(6 + payload.len() + 4);
    pkt.push(0x69);                                   // H[0]
    pkt.push((l & 0xFF) as u8);                       // H[1]
    pkt.push((((l >> 8) & 0x1F) as u8) | (top3 << 5));// H[2]
    pkt.push((addr & 0xFF) as u8);                    // H[3]
    pkt.push(((addr >> 8) & 0xFF) as u8);             // H[4]
    pkt.push(((addr >> 16) & 0xFF) as u8);            // H[5]
    pkt.extend_from_slice(payload);
    let crc = crc32_kt(&pkt);                         // over header + payload
    pkt.extend_from_slice(&crc.to_le_bytes());        // little-endian
    pkt
}

/// FINAL packet: writes the 16-byte image header last. top3 is always 0b111 (0xE0).
fn build_final_packet(image_header16: &[u8; 16], base_addr: u32) -> Vec<u8> {
    build_data_packet(image_header16, base_addr, 0b111) // L = 0x10, H[2] = 0xE0
}
```
Loop driver (from state 7/8): `block` starts at 0 and increments; `addr = base + 0x10`
for block 0 (payload 1008 B from image offset 0x10), else `addr = base + block*0x400`
(payload 1024 B from image offset `block*0x400`); `top3 = 1` when `block % 0x20 == 0`
else `0b111`. `base = flag<<15` (0 or 0x8000).

---

## 2. RESET / reboot, and STP / INF roles  (CONFIDENCE: HIGH)

### There IS a single reset-to-firmware command: `RESET = 5A 52 53 54`  ("ZRST")
In `FUN_00b8c380` state 10 the command bytes come from the ctor
`FUN_006cf5b0("ZRST%{appname}", 4)` — only the first **4** bytes are taken:
`Z R S T = 5A 52 53 54`. (String at file offset 0xE562C0 is `"ZRST%{appname}"`;
`%{appname}` is adjacent filler, not sent.) It is sent, the device replies `0x78`
(state 0xc waits for it), then state 0xd logs `"UPGRADE FIRMWARE SUCESS"`.

Two paths reach RESET:
- **flag == 0**: state 10 sends `RESET` directly → state 0xc.
- **flag != 0**: state 10 sends `INF` first (→ state 0xb), state 0xb does a read-back
  verification, then sends `RESET` → state 0xc.

### STP = `96 53 54 50`   (state 9)  — "stop programming"
`FUN_006cf5b0(&DAT_01255ae0,4)` = `96 53 54 50`. Sent in state 9 **after** the last
data packet + FINAL packet have both been ACKed with `0xA5`. Marks end of the program
stream. Device replies `0x78` (state 10 waits for it).

### INF = `F0 49 4E 46`   (state 10, only when flag != 0)  — "info / verify"
`FUN_006cf5b0(&DAT_01271720,4)` = `F0 49 4E 46`. Sent only on the flag-set path. It
transitions to state 0xb, which then **reads back** device memory
(`FUN_00d295d0`/`FUN_00d2a2d0`/`FUN_00d2a760` at offsets 0 and 0x10), byte-swaps to
big-endian and compares against expected values (a post-write verify). On match it sends
`RESET`. So INF requests device info used to verify the write before reset.

### State/command map (as decompiled)
| State | Waits for | Sends | Bytes |
|---|---|---|---|
| idle 1 | — | KTM | `1e 4b 54 4d` |
| resp 1 | 0x78 | CHP (flag=0) / VER (flag=1) | `d2 43 48 50` / `f0 56 45 52` |
| resp 2 | 0x78 | KEY | `f0 4b 45 59` |
| resp 3 | 0x78 | CHP | `d2 43 48 50` |
| resp 4 | (>=13 B) | erase/setup 10 B | `2d 29 00 10 0e 15 00 60 00 bc` |
| resp 5 | 0x78 | PWO | `3c 50 57 4f` |
| resp 6 | 0x78 | KSTA | `4b 53 54 41` |
| resp 7 | 0x78 | first data packet (`FUN_0054d7d0`) | — |
| resp 8 | 0xA5 (else 0x03=done) | next data pkt / FINAL pkt (`FUN_005733a0`) | — |
| resp 9 | 0xA5 | STP | `96 53 54 50` |
| resp 10 | 0x78 | INF (flag=1) then / or RESET | `f0 49 4e 46` / `5a 52 53 54` |
| resp 0xb | 0x78 | (verify readback) then RESET | `5a 52 53 54` |
| resp 0xc | 0x78 | → state 0xd | — |
| idle 0xd | — | log "UPGRADE FIRMWARE SUCESS" | — |

ACK checker `FUN_00d2aa60(char,0)` = `QByteArray::indexOf`; expected bytes are
`0x78` (accept), `0xA5` (block ACK), `0x03` (done/status).

---

## 3. Token confirmations  (CONFIDENCE: HIGH — read straight from .rdata)
All verified by dumping the bytes at the exact `DAT_` addresses referenced by the
send calls:

| Token | Bytes | Verified |
|---|---|---|
| KTM  | `1e 4b 54 4d` | ✅ @0x013297e0 |
| VER  | `f0 56 45 52` | ✅ @0x01346504 |
| CHP  | `d2 43 48 50` | ✅ @0x0125fe20 |
| KEY  | `f0 4b 45 59` | ✅ @0x01346500 |
| PWO  | `3c 50 57 4f` | ✅ @0x01336380 |
| KSTA | `4b 53 54 41` | ✅ @0x013297e4 |
| STP  | `96 53 54 50` | ✅ @0x01255ae0 |
| INF  | `f0 49 4e 46` | ✅ @0x01271720 |
| erase/setup (10 B) | `2d 29 00 10 0e 15 00 60 00 bc` | ✅ @0x01271724 |
| RESET | `5a 52 53 54` ("ZRST") | ✅ str @file 0xE562C0, 4 bytes sent |

---

## Confidence & open items
- Header bit layout, CRC scope+endianness, final-packet layout, RESET/STP/INF, all
  tokens: **HIGH** (direct decompile + byte dumps).
- **Correction to prior docs**: CRC-32 covers **header+payload** (not payload-only) and
  uses **init=0 / xorout=0** (not the standard 0xFFFFFFFF/complement variant). Re-check any
  existing `KtCdcCodec` fixtures against this.
- The `top3` bank field: at the observed call sites `param_2` is the literal `1`, and the
  boundary test uses `blockcount = DAT_013e90c4[1] = 0x20`. Whether firmware larger than the
  JA11 image ever passes `param_2 != 1` is unconfirmed; the table has entries `{4,0x20,0x40}`.
- `flag` (`this+0x38`, 0/1) selects VER/KEY vs CHP-only path and the INF-verify path, and
  sets the write base `flag<<15`. See section 4 for exactly how it is set and its concrete
  value for the JA11 V2.2 image.
- Not hardware-validated here; validate with FakeBootloader/golden fixtures before any write.

---

## 4. The `flag` (this+0x38) semantic — follow-up  (CONFIDENCE: HIGH)

The state-machine object is a QObject; its moc dispatcher is `FUN_00b8dbc0`
(slot 1=`FUN_00b8c380` resp-states, 2=`FUN_00b8f200` idle, 3=open-file, 4=`FUN_00b8ed20`
UpGrade, 7=byte setter). Relevant fields: `+0x20` = loaded image QByteArray,
`+0x28` = total block count, `+0x2c` = current block index, `+0x38` = **flag**,
`+0x3c` = state.

### 4.1 WHERE/HOW flag is set — derived from the IMAGE HEADER, not UI/constant
`flag` is written **only** in the file-open/prepare routine `FUN_00b8f7c0` (called from the
open-file slot and re-run at the top of the UpGrade handler). The UpGrade handler
`FUN_00b8ed20` resets `+0x3c/0x2c/0x40` but never touches `+0x38`, so the value set at
file-load time is what the flasher uses. (The slot-7 byte setter `FUN_00b8ed10` writes a
*different* byte, `+0x51`, and is unrelated to the flash flag.)

Evidence (`FUN_00b8f7c0`):
```c
puVar5 = *(uint **)(param_1 + 0x20);          // the loaded firmware image bytes
*(undefined1 *)(param_1 + 0x38) = 0;          // flag defaults to 0
... uVar4 = data offset (0x10 SSO / puVar5[3]) ...
if (*(char *)((int)puVar5 + uVar4 + 0xf) == '1') {   // <-- image byte at offset 0x0F
    *(undefined1 *)(param_1 + 0x38) = 1;             // flag = 1
    ...FUN_00c26c10("background-color: green ",...)   // UI indicator turns GREEN
} else {
    ...FUN_00c26c10("background-color: white ",...)   // UI indicator stays WHITE
}
```
So **flag = 1 iff image byte `0x0F` == ASCII `'1'` (0x31); otherwise flag = 0.** It is a
property of the loaded image header, decided at load time — not a UI choice, not a build
constant. (`'1'` almost certainly marks a secure/keyed image variant: flag=1 is the only path
that sends VER+KEY auth and does the INF read-back verify.)

### 4.2 Concrete value for `JadeAudio JA11_V2.2.bin`  → flag = 0
`xxd` of the real image:
```
00000000: 4b54 5f48 656c 696f 735f 7631 625f 5f5f  KT_Helios_v1b___
0000000f: 5f                                        _   <- byte 0x0F = 0x5F ('_'), != '1'
```
=> **flag = 0** for this image. Therefore:
- **Write base = `flag<<15` = `0x00000000`.** Data-packet addresses are `0x10` (block 0,
  1008 B) then `block*0x400`; the FINAL packet writes the 16-byte image header at address `0`.
- **Command path = CHP-only (flag=0):** state 1 sends CHP and jumps straight to state 4
  (erase); it does **not** send VER or KEY (those are the flag=1 branch).
- **Reset path:** state 10 sends `RESET` (`5A 52 53 54`) **directly** — no `INF`, no
  read-back verify (INF + the state-0xb verify are flag=1 only).

### 4.3 Packet/bank count for the JA11 image
Block count `this+0x28 = ceil(size / 0x400)`, where `size` is the image size after an
**optional** 0xFF pad (`FUN_00b8f7c0` lines 160-173):
```c
if (FUN_00e09900() != 0)                       // pad enabled?
    FUN_006cc920(size, (0x40000 - size) + flag*(-0x8000), 0xff);  // pad tail with 0xFF
size = imageSize;
blocks = size>>10; if (size & 0x3ff) blocks++; // ceil(size/1024)
*(param_1+0x28) = blocks;
```
`FUN_00e09900` = `(*(byte*)(mainwindow+0x120) >> 1) & 1` — **bit 1 of a runtime UI-state byte**
(an operator checkbox, e.g. "full/complete download"); not derivable from the image. Two
outcomes for flag=0:
- **Pad OFF:** size = 67312 (0x106F0) → `blocks = ceil(67312/1024) = 66` data packets + 1 FINAL
  = **67 packets total.**
- **Pad ON:** image 0xFF-padded to `0x40000` (256 KB) → `blocks = 256` data packets + 1 FINAL
  = **257 packets total** (the extra packets are pure 0xFF fill at addresses ≥ 0x10600; 0xFF is
  the erased-flash value).

**Bank id (`param_2`) is ALWAYS 1** for this tool: both `FUN_0054d7d0` call sites pass the
literal `1` (`FUN_0054d7d0(out,1,block,flag<<15,src)`) and `DAT_013e90c4[1] = 0x20`. So the
H[2] top-3-bits are `0x20` (=`1<<5`) on every 32 KB-aligned block (block index % 32 == 0:
blocks 0,32,64 unpadded; 0,32,…,224 if padded) and `0xE0` (`0b111`, continuation) on all other
data packets and on the FINAL packet. The field is effectively a "32 KB-aligned block" marker,
never a true multi-bank index, for this firmware/tool.

### 4.4 Bottom line for a same-image reflash of JA11 V2.2
flag=0 → base 0, CHP-only handshake (no VER/KEY), RESET directly (no INF verify), header
boundary marker 0x20 / continuation 0xE0. Packet count is deterministic: **67** with the pad
checkbox off, **257** with it on (extra packets are pure 0xFF fill). Replaying with the pad
**off** reproduces the minimal 67-packet sequence; byte-diff against a `FakeBootloader` before
touching hardware.

---

## 5. Firmware READ / backup feasibility  (added 2026-09-05)

Static RE of the vendor exe (rizin `/tmp/ja11.rzdb` + Ghidra `/tmp/ghidra_proj/ja11b`,
`analyzeHeadless … -postScript DecompRead.java`) plus the prior hardware results in
`docs/CDC-PROTOCOL.md`. Per-item confidence inline.

### 5.1 Q1 — Does any general flash-read command exist in the vendor exe?

**A general "read N 32-bit words @ addr" primitive DOES exist in the exe, but it lives on the
normal-mode HID/`0x4B` channel (the KT_USB library), NOT on the CDC bootloader, and it does
not reach flash on this silicon.**  Candidates, all confirmed by decompile:

| Addr | Role | Frame it builds | Receives+stores? | Confidence |
|---|---|---|---|---|
| `FUN_0054d170` | **multi-word READ / dump** — loops `count` times, `addr += 4` each iter, accumulates words into a buffer, returns a QByteArray (`FUN_00c26c10`) | `4B <addr:4 LE> 08 00 00 00 00 00` (11 B) — **cmd byte = `0x08`** at offset 5 | **YES** — this is the arbitrary-addr/len reader | HIGH |
| `FUN_0054d0e0` | `0x33` handshake/status | `4B 00 00 00 00 33 00 …` (11 B) | reads 11 B, checks `resp[7]==3` | HIGH |
| `FUN_0054d340` | single word read (cmd `0x08`, with a `& 0x20` nibble-tweak on a byte) | `4B <addr> 08 …` | reads 11 B, stores word | HIGH |
| `FUN_0054d400` | single-shot `0x4B` cmd+readback | `4B …` | reads 11 B, stores `resp` word | MED-HIGH |
| `FUN_005583a0` | `0x88` **write** word | `4B <addr:4> 88 00 <data:4> 00` (11 B) | **no reply parsed** (write-only) | HIGH |

- The read word is extracted from **`resp[1..5)` LE** in `FUN_0054d170` (not `resp[7..11]`
  as the `0x33` status uses). Frame address is a full **32-bit LE** field (bytes 1-4), so the
  reader can address anything, any length — it is a true general read *in code*.
- **No direct callers** resolve for `FUN_0054d170`/`0054d0e0`/`0054d340`/`0054d400` in either
  tool (Qt slot/functor indirection or dead library code). The exe ships the whole KT_USB HID
  ISP command set (`0x33/0x32/0x08/0x21/0x88`) but the JA11 GUI only ever *writes* over CDC.
- **No read/dump/upload/backup token exists on the CDC bootloader.** The serial state machine
  `FUN_00b8c380` has exactly cases 1..0xc + default and sends exactly the 10 known tokens; the
  only inbound-data handling is ACK-byte scans (`FUN_00d2aa60`) and the INF reply parse. A byte
  scan of the exe for `<lead><3-ASCII>` read-verb tokens (RD/DMP/UPL/BAK/GET/MEM/READ/LOAD)
  found only Qt-library substrings (`_MEM_`, `_LOAD_`, `PREADER`), no 11th command. (HIGH)

### 5.2 Q2 — INF exact wire format

- **Request is FIXED, carries no address/length:** `FUN_006cf5b0(&DAT_01271720,4)` sends
  exactly the 4 bytes **`F0 49 4E 46`**. There is no addr/len operand anywhere. (HIGH)
- **Response = whole-image verify only.** State 0xb (`FUN_00b8c380` case 0xb) does:
  `mid(reply,1,4)` → `toHex` (`FUN_00d2a2d0`, `"0123456789abcdef"`) → `toUInt(base16)`
  (`FUN_00d2a760`→`FUN_0072b9a0`) = **word1**; `mid(reply,5,4)` → same → **word2**. It then
  byte-swaps each (big-endian on the wire) and compares:
  - `byteswap(word1) == image size` (`*(uint*)(image+4)`), and
  - `byteswap(word2) == CRC32(entire image)` where the CRC is `FUN_0055f3b0` over the full
    image data — **the same reflected CRC-32 (table `DAT_0120e020`, poly `0xEDB88320`, init 0,
    xorout 0)** used to build data packets.
- So **INF is a fixed size+whole-image-CRC readback, not a data readback.** The device returns
  an ASCII-hex string encoding *(flash size, CRC32-of-flash)*; there is no way to select an
  address or pull actual bytes. It cannot reconstruct firmware. (HIGH)
- Useful corollary: INF can be sent on a *fresh* CDC bootloader (before any write) to
  **fingerprint the currently-installed firmware** (its size + CRC-32) — a verify/identity
  check, still not a dump.

### 5.3 Q3 — Can normal-mode `0x08` reads be made to return real flash?

**No — not on this silicon, by any known sequence.** Reasoning:

- The `0x33/0x08` HID frames are the **KT_USB_BOOT ISP** command set. They only answer when
  the chip is in the boot-ROM's **HID-ISP mode** (where `0x33` returns result `== 3` = "ISP
  ready", then `0x08` reads flash). On JA11 hardware `0x33` **times out** and `0x08` returns
  all-zeros (`docs/CDC-PROTOCOL.md`, verified @ `0x0`/`0x80000`/`0x83000`).
- Reason it times out: this chip's **runtime firmware** (`31B2:0111`, and the fully-booted
  FiiO product `2972:0102`) does **not** implement the ISP set on its HID `0xFF01` collection.
  That collection is the **app/EQ dispatcher** (`DISPATCHER-TRACE.md`, on-chip
  `FUN_ram_00083010`) whose only cases are `'W'/'R'/'S'/'C'/9` — and `'R'` reads an internal
  **EQ-coefficient table**, never flash. There is no `0x08` case, so `0x08`/`0x33` fall through
  → zeros / no reply. (HIGH — matches both the on-chip decompile and hardware.)
- The boot ROM's ISP on *this* part is exposed over **CDC serial (`8888:CDC0`)**, reached only
  by the `0x54 "T12345678"` unlock, **which reboots the device** out of `2972:0102`. There is
  **no ISP-enter that keeps the device in `2972:0102`/`31B2:0111` and turns on HID `0x08`
  reads** — the read primitives in the exe assume a HID-ISP boot mode this variant simply
  doesn't offer over HID.
- Even in the CDC bootloader the read *response is not receivable as data*: the only reply
  channel there is ACK bytes + the INF size/CRC string. Flash base is moot (write path uses
  `base = flag<<15 = 0`; the app image loads at virtual `0x80000`/`0x83000`), because no
  command returns flash contents.

### 5.4 Q4 — Bottom line

**(b) No general firmware READ is exposed by the known protocol.** Neither channel gives back
flash bytes:

- **Normal-mode HID:** the general reader (`FUN_0054d170`, cmd `0x08`) exists in the exe but is
  inert on this chip — the runtime firmware has no flash-read command (`0x33` never reaches
  ISP-ready), so `0x08` returns zeros.
- **CDC bootloader:** its 10 tokens contain no read/dump; **INF** yields only *(size, CRC32)*
  of the installed image, not its contents.

A true backup therefore requires **dumping the resident `KT_USB_BOOT` ROM / lower-flash
region** — the code below load address `0x80000` that ships in the part separately from the
`KT_Helios` OTA app image (`DISPATCHER-TRACE.md` §"missing-ROM callees":
`func_0x00024b4c`, `func_0x00016c14/00`, `func_0x000154f8` all live below `0x80000` and are
absent from `JA11_V2.2.bin`). That ROM is not reachable through any command in the vendor
tool; extracting it needs either a working HID-ISP `0x08` path (which this silicon does not
expose) or hardware means (JTAG/SWD/glitch, or a chip-specific KTMicro factory ISP), none of
which are in the reversed protocol.

**Concrete hardware experiments worth running (to confirm, not to dump):**
1. Fresh CDC bootloader (`8888:CDC0`) → `KTM` (`1e 4b 54 4d`), wait `0x78`; then send INF
   `F0 49 4E 46` and capture the ASCII-hex reply. Expected: 8+ hex chars encoding
   *(flash size, CRC32)* of the currently-installed firmware — proves INF is a
   fingerprint-only readback and gives you the installed image's identity. (No flash write —
   safe.)
2. (Negative control, already done) normal-mode `0x08` word-read at `0x0/0x80000/0x83000`
   returns zeros; re-confirms no HID flash read.

Confidence: Q1 read-primitive existence + framing HIGH; Q2 INF format HIGH; Q3 non-feasibility
HIGH (decompile + prior hardware); Q4 conclusion HIGH.
