# Bootloader capture fixtures (roadmap M0)

Normalized, in-repo transcripts of the `0x8888:0xCDC0` CDC bootloader exchange. Raw
`.pcapng` is ground truth but is awkward to diff, review, and regression-test; a transcript
is a small JSON document that `ktflash bootdiag --replay <file>` can decode and validate
**with no hardware attached**, and that CI can check on every change.

> ⚠️ `synthetic-success.json` is **not a real capture** — its bytes use ktflash's
> `ReferenceCodec` (a stand-in for the real, still-unreversed KTMicro wire framing). It
> exists to demonstrate the format and exercise `--replay`. Real captures will show payloads
> as `<raw>` on replay until the KTMicro wire codec (`KtCdcCodec`) lands.

## Format

```jsonc
{
  "source": "cap-2026-09-04.pcapng",   // capture file / tool+version / "synthetic"
  "note":   "…",                        // anything a reviewer should know
  "device": "JCALLY JM12 8888:cdc0",    // free-form device identity
  "frames": [
    {
      "direction": "out",              // "out" (host→device, ep 0x03) | "in" (ep 0x83)
      "stage": "program",             // handshake|erase|program|verify|reset|unknown
      "timestamp_ms": 230,             // optional, relative to capture start
      "bytes_hex": "03…",             // raw payload; separators/0x prefix tolerated on read
      "decoded": {                      // optional provisional decode — omit what you can't prove
        "command": "WRITE",
        "address": 0,
        "payload_length": 4,
        "checksum": "…"
      }
    }
  ]
}
```

Only `direction` and `bytes_hex` are required per frame. Fill in `decoded` fields **only**
where a capture actually proves them; leave the rest absent rather than guessed.

## Capturing a real trace

On the Windows capture box (Windows is a capture source only — never a user path):

```powershell
USBPcapCMD.exe   # pick the dongle's root hub, write cap.pcapng, then run one vendor flash
```

Decode to transcript-ready fields on macOS/Linux:

```sh
tshark -r cap.pcapng \
  -Y "usb.transfer_type==0x03 && usb.endpoint_address in {0x03,0x83}" \
  -T fields -e usb.endpoint_address -e usb.capdata
```

`proto::transcript::Transcript::from_tshark()` turns that exact output into a transcript
(direction is inferred from the endpoint address; stages/decodes start `unknown` and get
classified as M1 recovers the framing).

## The corpus we want (M0)

Not just one success — the failure/edge traces are what make the flasher *recoverable*:

| Fixture | Establishes |
|---|---|
| success, image A | baseline sequence, chunking, ACKs, terminal reset |
| success, image B (different size) | separates constants from image-derived length/addr/checksum |
| cancel before erase | whether the vendor emits abort/cleanup frames |
| bad-image rejection | host-side vs. bootloader-side validation |
| disconnect during program | recovery / reconnect behaviour |
| verify failure (if safely inducible) | verify semantics + safe recovery |

Keep raw `.pcapng` only where redistribution is safe; otherwise keep the transcript plus the
capture's hash and reproduction notes.
