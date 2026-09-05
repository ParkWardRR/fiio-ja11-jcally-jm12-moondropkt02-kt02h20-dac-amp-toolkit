//! Replacement for `cmd_flash_cdc` in `flasher/src/main.rs`.
//!
//! This is **not a new module** — it is the body of the existing command, rewritten to:
//!
//! 1. drive [`crate::proto::ktcdc_driver::Driver`] instead of inline `rusb` closures, and
//! 2. accept `--transport auto|serial|usb` and `--port <dev>` so macOS can flash over the CDC
//!    tty (`docs/MACOS-NATIVE.md`).
//!
//! Paste the two functions below over the existing `cmd_flash_cdc` (`main.rs:837-996`). The
//! dry-run output, the warning banner, the `--yes` gate, and the flag/base derivation are
//! **unchanged** — only the hardware-driving half is different.
//!
//! `open_bootloader()` (`main.rs:787`) becomes dead once `cmd_bootdiag` is migrated too; leave
//! it until then.
//!
//! > **Status: untested.** Compiles against stubs; never run.

use std::time::Duration;

use crate::boottransport::{self, Preference};
use crate::proto::fingerprint::DeviceFingerprint;
use crate::proto::ktcdc_driver::{Driver, Progress};
use crate::proto::ktcdc_journal::JournalRecorder;
use crate::proto::{self, image::KtHelios};

/// How often to print a packet line during the write. The inline version printed every 8th
/// packet plus the final one; the driver now reports every packet and we throttle here.
const PROGRESS_EVERY: usize = 8;

/// `ktflash flash-cdc --image <fw.bin> [--flag 0|1] [--base 0xADDR]
///                    [--transport auto|serial|usb] [--port <dev>] [--execute --yes]`
///
/// The **native CDC bootloader write**. Byte framing from `proto::ktcdc` (decompiled +
/// unit-tested); sequencing from `proto::ktcdc_driver` (unit-tested against a fake). Default is
/// a **dry run** that only builds and summarises the packet stream — pass `--execute` to drive
/// hardware. `--execute` requires a **fresh** bootloader (unlock immediately before; the state
/// machine is one-shot).
pub fn cmd_flash_cdc(args: &[String]) -> Result<(), String> {
    use proto::ktcdc::plan_stream;

    let (mut image_path, mut flag_override, mut base_override) = (None, None, None);
    let (mut execute, mut acked) = (false, false);
    let mut transport_pref = Preference::default();
    let mut port: Option<String> = None;

    let mut it = args.iter();
    while let Some(flag_arg) = it.next() {
        match flag_arg.as_str() {
            "--image" => image_path = Some(it.next().ok_or("--image needs a path")?.clone()),
            "--flag" => {
                flag_override = Some(crate::parse_num(it.next().ok_or("--flag needs 0 or 1")?)? as u32)
            }
            "--base" => {
                base_override =
                    Some(crate::parse_num(it.next().ok_or("--base needs an address")?)? as u32)
            }
            "--transport" => {
                transport_pref = it
                    .next()
                    .ok_or("--transport needs auto|serial|usb")?
                    .parse::<Preference>()?
            }
            "--port" => port = Some(it.next().ok_or("--port needs a device path")?.clone()),
            "--execute" => execute = true,
            "--yes" => acked = true,
            other => return Err(format!("unknown flag for flash-cdc: {other}")),
        }
    }

    let path = image_path.ok_or(
        "usage: ktflash flash-cdc --image <fw.bin> [--flag 0|1] [--transport auto|serial|usb] [--port <dev>] [--execute --yes]",
    )?;
    let image = std::fs::read(&path).map_err(|e| format!("read {path}: {e}"))?;
    if !proto::image::looks_like_kt_helios(&image) {
        return Err(format!("{path} is not a KT_Helios image — refusing to flash"));
    }
    let img = KtHelios::parse(&image).map_err(|e| e.to_string())?;

    // The vendor derives `flag` from image byte 0x0F ('1' => secure/keyed variant, else 0); it
    // selects the handshake path (flag=0 = CHP-only; flag=1 = VER+KEY+INF-verify) and the write
    // base (flag<<15). `--flag` overrides for experiments.
    let flag = flag_override.unwrap_or(if image.get(0x0F) == Some(&b'1') { 1 } else { 0 });
    let base = base_override.unwrap_or(flag << 15);
    let plan = plan_stream(&image, base);
    let data_pkts = plan.iter().filter(|p| !p.is_final).count();
    let wire: usize = plan.iter().map(|p| p.bytes.len()).sum();

    println!("flash-cdc plan for {path}");
    println!("  image      {} bytes  chip={:?} magic={:?}", image.len(), img.chip, img.magic);
    println!(
        "  flag={flag} ({})  write base=0x{base:05x}  (from image[0x0F]={:#04x})",
        if flag == 0 { "CHP-only, direct RESET" } else { "VER+KEY+INF-verify" },
        image.get(0x0F).copied().unwrap_or(0)
    );
    println!("  packets    {data_pkts} data + 1 final = {} total, {wire} wire bytes", plan.len());
    let f0 = &plan[0];
    let fin = plan.last().unwrap();
    println!(
        "  first pkt  addr=0x{:05x} len={} : {}",
        f0.addr,
        f0.payload_len,
        crate::hexline(&f0.bytes, 8)
    );
    println!(
        "  final pkt  addr=0x{:05x} len={} : {}",
        fin.addr,
        fin.payload_len,
        crate::hexline(&fin.bytes, 12)
    );

    if !execute {
        println!("\n[dry run] no hardware touched. Re-run with --execute --yes on a FRESH bootloader to write.");
        println!("  (unlock immediately before --execute: the bootloader state machine is one-shot.)");
        return Ok(());
    }

    // Require --yes before any destructive write. ktflash cannot read firmware back off the
    // device, so a botched cross-flash needs a working image to recover — the KT_USB_BOOT ROM
    // survives an app-flash and stays reflashable, but only if you have an image to retry with.
    if !acked {
        eprintln!("\n\x1b[1;33m╔══════════════════════════════════════════════════════════════════════╗");
        eprintln!("║  ⚠  BACK UP YOUR FIRMWARE FIRST — this ERASES and rewrites flash.     ║");
        eprintln!("║                                                                      ║");
        eprintln!("║  ktflash CANNOT read firmware off the device, so there is no          ║");
        eprintln!("║  automatic backup. If this cross-flash goes wrong, you need a          ║");
        eprintln!("║  working image on hand to retry — a bad write with no image leaves     ║");
        eprintln!("║  the dongle bootloader-only indefinitely. Save a known-good image      ║");
        eprintln!("║  before you start (the vendor's updater / your own copy).             ║");
        eprintln!("║                                                                      ║");
        eprintln!("║  A wrong or mismatched image will fail to boot. Proceed at your risk. ║");
        eprintln!("╚══════════════════════════════════════════════════════════════════════╝\x1b[0m");
        return Err(
            "refusing to write without acknowledgement — save a known-good image, then re-run with --yes".into(),
        );
    }
    eprintln!("\n⚠  Writing flash. Ensure you have a working firmware image saved for recovery.");

    // ---- hardware write ----
    let (mut pipe, label) = boottransport::open(transport_pref, port.as_deref())?;
    println!("\n[execute] transport: {label}");

    // Open the operation journal BEFORE the first command. Until now `flash-cdc` wrote flash
    // with no recovery record at all, so an interrupted write left `ktflash recover` nothing to
    // read (ROADMAP Appendix A: "destructive writes ship only with recovery semantics").
    // The target is fingerprinted as the bootloader itself, because that is what we are
    // talking to — the normal-mode identity is not observable from here.
    let mut recorder = JournalRecorder::begin(
        DeviceFingerprint::new(crate::BOOT_VID, crate::BOOT_PID),
        proto::plan::sha256_hex(&image),
        image.len(),
        env!("CARGO_PKG_VERSION"),
        format!("flash-cdc flag={flag} base=0x{base:05x} via {label}"),
    )?;
    println!("[execute] journal: {}", recorder.path().display());
    println!("[execute] driving the bootloader (flag={flag} path)…");

    let total = plan.len();
    let result = {
        let mut driver = Driver::new(pipe.as_mut());
        driver.run(&plan, flag, &mut |ev| {
            recorder.observe(&ev);
            print_progress(ev, total);
        })
    };

    if let Err(e) = result {
        let msg = e.to_string();
        recorder.fail(&msg);
        // Tell the user what the journal says to do, rather than leaving them to guess.
        eprintln!("\n\x1b[1;31mflash failed:\x1b[0m {msg}");
        eprintln!("journal: {}", recorder.path().display());
        eprintln!("next safe step: ktflash recover {}", recorder.path().display());
        return Err(msg);
    }

    if let Some(werr) = recorder.write_error() {
        // A journal that silently stopped being written is worse than no journal, because the
        // operator trusts it.
        eprintln!("\n⚠  the operation journal could not be fully written: {werr}");
        eprintln!("   recovery data may be incomplete — note where the flash got to manually.");
    }

    println!("\nUPGRADE FIRMWARE SUCESS — device should re-enumerate to normal JA11 mode.");
    println!("journal: {}", recorder.path().display());
    println!(
        "  Once the dongle re-enumerates in normal mode, confirm with `ktflash probe`.\n  \
         The journal stays at `reset-issued` until a reprobe confirms the new identity."
    );
    Ok(())
}

/// Reproduces the inline version's output, including its every-8th-packet throttling.
fn print_progress(ev: Progress<'_>, total: usize) {
    match ev {
        // The pre-write edge is for the journal, not the user — printing it would double every
        // line. The one exception is the erase, which is the point of no return.
        Progress::Sending { tag: "KSTA" } => println!("  KSTA   -> (erasing flash…)"),
        Progress::Sending { .. } => {}
        Progress::Step { tag, reply } => println!("  {tag:<6} -> {reply:02x?}  ok"),
        Progress::ChipInfo { reply } => {
            println!("  CHP    -> {reply:02x?}  ({} bytes)", reply.len())
        }
        Progress::Erased { reply } => println!("  KSTA   -> {reply:02x?}  ok (erase complete)"),
        Progress::Packet { index, addr, payload_len, is_final, reply, .. } => {
            if index % PROGRESS_EVERY == 0 || is_final {
                println!("  pkt {index:>3}/{total} @0x{addr:05x} len={payload_len} -> {reply:02x?}");
            }
        }
        Progress::Finished => {}
    }
}

/// Replacement for `cmd_bootdiag`'s live half — proves the pipe is alive over whichever
/// transport is available, so macOS gets a working `bootdiag` too.
///
/// UNVERIFIED: sends `KTM` and expects the `0x78` accept byte. The inline `cmd_bootdiag` only
/// read the pipe without sending; sending a handshake is a better liveness test but **does
/// advance the one-shot state machine**, so it must stay out of any path that then expects a
/// fresh bootloader. Flagged for review before adoption.
// Remove this allow once `cmd_bootdiag` is migrated to call it (APPLY.md step 5).
#[allow(dead_code)]
pub fn bootdiag_live(pref: Preference, port: Option<&str>) -> Result<(), String> {
    use proto::ktcdc::{ack, token};

    let (mut pipe, label) = boottransport::open(pref, port)?;
    println!("bootloader transport: {label}");

    pipe.send(&token::KTM).map_err(|e| format!("send KTM: {e}"))?;
    let deadline = std::time::Instant::now() + Duration::from_millis(1500);
    let mut acc: Vec<u8> = Vec::new();
    while std::time::Instant::now() < deadline {
        match pipe.recv(Duration::from_millis(200)) {
            Ok(chunk) => acc.extend_from_slice(&chunk),
            Err(proto::cdc::CdcError::Timeout) => {}
            Err(e) => return Err(e.to_string()),
        }
        if acc.contains(&ack::ACCEPT) {
            println!("KTM -> {acc:02x?}  ✅ pipe is live");
            println!("(the download framing is implemented in `flash-cdc`; this just proves the pipe)");
            return Ok(());
        }
    }
    Err(format!("no {:#04x} accept byte from KTM; got {acc:02x?}", ack::ACCEPT))
}
