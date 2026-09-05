# rizin command log — how the flasher was disassembled

Reproducible rizin steps against `JadeAudio JA11 Upgrade Tool.exe` (`EXE`). PE ImageBase is
`0x400000`; rizin and Ghidra agree on addresses. No Python involved.

```sh
EXE="JadeAudio JA11 Upgrade Tool.exe"

# --- transport: HID function-name strings + their single GetProcAddress resolver ---
rizin -q -c 'izz~HidD_;izz~hid.dll;izz~GetProcAddress' "$EXE"
rizin -q -c '/v4 0x0113e136' "$EXE"          # code ref to "HidD_SetFeature"
rizin -q -c 'pd 90 @ 0x01090250' "$EXE"      # resolver: LoadLibrary + GetProcAddress table

# --- analyse once, save a project (aac/aar recognise statically-linked hidapi) ---
rizin -q -c 'e anal.timeout=240; af @ 0x0109094e; aac; aar; Ps ja11.rzdb' "$EXE"

# --- hidapi functions rizin recognised, and the real send/recv path ---
rizin -q -c 'Po ja11.rzdb; afl~hid' "$EXE"
rizin -q -c 'Po ja11.rzdb; axt @ 0x01090930' "$EXE"   # hid_send_feature_report -> NO callers
rizin -q -c 'Po ja11.rzdb; axt @ 0x010909c0' "$EXE"   # hid_write -> the flash call sites
rizin -q -c 'Po ja11.rzdb; axt @ 0x01090780' "$EXE"   # hid_read_timeout callers

# --- decode each command builder's 11-byte report (0x4b | addr | cmd | 00 | data) ---
rizin -q -c 'pd 40 @ 0x0054d0e0' "$EXE"      # handshake cmd 0x33
rizin -q -c 'pd 40 @ 0x005583a0' "$EXE"      # write-word cmd 0x88
rizin -q -c 'pd 40 @ 0x006246c0' "$EXE"      # erase cmd 0x21

# --- the serial/CDC bootloader engine (QSerialPort state machine) ---
rizin -q -c 'izz~SerialRecive;izz~ReadSerial;izz~QSerialPort::' "$EXE"
rizin -q -c '/v4 0x0113e450' "$EXE"          # "OUT: " -> the serial send wrapper FUN_00b8bfd0
```

## Firmware image (no disassembler needed)

```sh
xxd 'JadeAudio JA11_V2.2.bin' | head          # KT_Helios magic, KT02H20B, Size, ENTY table
strings -a 'JadeAudio JA11_V2.2.bin' | head    # FIIO, JadeAudio JA11, embedded serial
```

Then Ghidra headless with [`DecompHID.java`](DecompHID.java) for clean C of the command
builders + orchestrator.
