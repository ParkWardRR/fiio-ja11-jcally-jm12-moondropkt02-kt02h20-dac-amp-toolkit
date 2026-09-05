# Reverting a JA11‑flashed dongle back to stock

Cross‑flashing to JA11 is **reversible** — the KTMicro bootloader will always take another
image — but only if you have a **stock image to flash back**. This page covers what you need
and the honest state of stock backups.

## The catch: you need a matching stock `.bin`

The flash **overwrites** the original firmware. To restore, you flash a stock image the same
way you flashed JA11 (same tool/flow, different `.bin`). Two cases:

| You want to restore to… | You need | Reality |
|---|---|---|
| **The exact original** (e.g. a Moondrop KT02H20 as it shipped) | that vendor's *specific* stock `.bin` | Often unpublished. Back it up **before** flashing, or source it from the vendor's own updater. |
| **A known‑good stock** (accept a JCALLY JM12 identity) | `jcally_jm12_v1.3.bin` + `Jm12_Firmware_Tool.exe` | Available from **JCALLY's AliExpress store FAQ**. Same chip; flashes fine. The device will then *identify as a JM12* (different USB ID/tuning), not its original brand. |

> [!IMPORTANT]
> **Back up before you flash.** The clean move is to dump the running firmware *first*. This
> repo's confirmed flash did **not** do that (lesson learned) — so that specific unit's
> original image is gone and can only be restored from a *matching* stock `.bin`, not a
> byte‑exact backup.

## How to flash stock back (same as JA11, different image)

1. Get a stock image (see table above).
2. Follow [`FLASHING.md`](FLASHING.md) (macOS + OrbStack) but use the
   **stock `.bin`** instead of `JA11_V2.2.bin`.
   - With **JCALLY's** `Jm12_Firmware_Tool.exe`, bootloader entry is by **holding the tiny
     reset button while plugging in** (hardware), vs. FiiO's software `T12345678` unlock.
3. The device re‑enumerates with the stock USB ID/product string (e.g. back to a JCALLY /
   KTMicro identity).

## Community stock‑image effort (help wanted)

To make reverts painless for everyone, we want **clean stock dumps** archived here (or
linked), one per device model. Contributing a dump:

> [!IMPORTANT]
> **There is no software way to read a KT02H20's firmware** — the bootloader has no read
> command and the normal‑mode `0x08` read is inert on this chip (see
> [`CDC-PROTOCOL.md`](CDC-PROTOCOL.md)). So a "stock dump" can only come from the manufacturer's
> distributed image, or a **hardware** read (JTAG/SWD/chip‑off). Save your original image before
> flashing — it cannot be recovered off the device afterward.

1. Obtain the model's **original manufacturer firmware image** (vendor updater package, or a
   hardware dump via JTAG/SWD/chip‑off — not possible over USB).
2. Verify it parses as a `KT_Helios` image: `ktflash image stock.bin`.
3. Open a PR/issue with the `.bin` (or a link), the model, and its stock `VID:PID` +
   product string, so others can restore that exact device.

Planned: dump a stock **JCALLY** unit specifically so there's a first‑class restore image in
the repo. Until then, JCALLY's official `jcally_jm12_v1.3.bin` is the reliable fallback.

## Recovery if a flash goes wrong mid‑way

If the device is stuck in **bootloader mode** (`8888:cdc0` / shows a COM port, no audio),
it is **not bricked** — the bootloader is still listening. Re‑run the flash with a valid
image (JA11 or stock). A power‑cycle with no flash usually drops it back to whatever image
is currently valid; if none is valid, it stays in the bootloader waiting for a write.
