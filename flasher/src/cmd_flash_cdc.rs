//! `ktflash flash-cdc` — the native CDC bootloader write.
//!
//! Split out of `main.rs`, where the state machine used to be inline and welded to `rusb`
//! closures. The pieces now are:
//!
//! - [`crate::proto::ktcdc`] — byte-exact framing (decompiled, unit-tested).
//! - [`crate::proto::ktcdc_driver`] — the sequencing, generic over
//!   [`Transport`](crate::proto::cdc::Transport) so it is testable with no hardware.
//! - [`crate::boottransport`] — serial (CDC tty) or libusb bulk. Serial is what lets macOS
//!   flash without OrbStack (`docs/MACOS-NATIVE.md`).
//! - [`crate::proto::ktcdc_journal`] — the operation journal, recorded *before* each
//!   destructive command.
//! - [`crate::proto::postflash`] — the post-reset reprobe that decides whether it worked.
//!
//! Together those two last items are ROADMAP Phase 3.4: *"journal each stage before its
//! destructive command and confirm success by a post-reset reprobe."*
//!
//! > **Status: the write path has not been re-tested on hardware since the refactor.** The
//! > sequence is transcribed from the version that was proven on 2026-09-05, and the logic is
//! > unit-tested, but no dongle has been flashed through this code.

use std::time::Duration;

use crate::boottransport::{self, Preference};
use crate::proto::fingerprint::DeviceFingerprint;
use crate::proto::ktcdc_driver::{Driver, Progress};
use crate::proto::ktcdc_journal::JournalRecorder;
use crate::proto::{self, image::KtHelios};

/// How often to print a packet line during the write. The inline version printed every 8th
/// packet plus the final one; the driver now reports every packet and we throttle here.
const PROGRESS_EVERY: usize = 8;

/// How long to wait for the dongle to come back after `RESET`.
///
/// Re-enumeration plus the host settling on a new address is usually a second or two; 20s is
/// slack for a slow hub or a USB/IP hop without making a genuinely dead device hang the tool.
const DEFAULT_REPROBE_SECS: u64 = 20;

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
    let mut expected: Option<DeviceFingerprint> = None;
    let mut reprobe = true;
    let mut reprobe_secs = DEFAULT_REPROBE_SECS;

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
            "--expect" => {
                let s = it.next().ok_or("--expect needs VID:PID (e.g. 2972:0102)")?;
                expected = Some(DeviceFingerprint::parse_short(s)?);
            }
            "--no-reprobe" => reprobe = false,
            "--reprobe-timeout" => {
                reprobe_secs = crate::parse_num(
                    it.next().ok_or("--reprobe-timeout needs a number of seconds")?,
                )?
            }
            "--execute" => execute = true,
            "--yes" => acked = true,
            other => return Err(format!("unknown flag for flash-cdc: {other}")),
        }
    }

    let path = image_path.ok_or(
        "usage: ktflash flash-cdc --image <fw.bin> [--flag 0|1] [--base 0xADDR]\n\
         \x20                    [--transport auto|serial|usb] [--port <dev>]\n\
         \x20                    [--expect VID:PID] [--no-reprobe] [--reprobe-timeout SECS]\n\
         \x20                    [--execute --yes]",
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

    println!("\nUPGRADE FIRMWARE SUCESS — device should re-enumerate to normal mode.");

    // ---- post-reset reprobe (ROADMAP Phase 3.4) ----
    // Without this the journal tops out at `reset-issued`, whose recovery action is
    // "wait and reprobe" — so `ktflash recover` could never say *done*, and a flash that reset
    // into a non-booting image looked exactly like one that worked.
    if !reprobe {
        println!("journal: {}", recorder.path().display());
        println!(
            "  --no-reprobe given, so the journal stays at `reset-issued`.\n  \
             Confirm by hand with `ktflash probe`."
        );
        return Ok(());
    }

    println!("\n[reprobe] waiting up to {}s for the dongle to re-enumerate…", reprobe_secs);
    let outcome = wait_for_reprobe(expected.as_ref(), Duration::from_secs(reprobe_secs));

    match &outcome {
        proto::postflash::ReprobeOutcome::Confirmed(fp) => {
            recorder.confirm(outcome.journal_detail());
            println!("[reprobe] ✅ {} — {}", fp.short(), outcome.advice());
        }
        proto::postflash::ReprobeOutcome::Mismatch { .. } => {
            // A halt state: the recovery model deliberately refuses to auto-select another image.
            recorder.identity_mismatch(outcome.journal_detail());
            eprintln!("\n\x1b[1;31m[reprobe] {}\x1b[0m", outcome.journal_detail());
            eprintln!("{}", outcome.advice());
            eprintln!("journal: {}", recorder.path().display());
            return Err(outcome.journal_detail());
        }
        _ => {
            // Leave the journal at `reset-issued` (→ WaitAndReprobe) rather than recording a
            // failure: the write itself completed, and the honest state is "reset issued,
            // outcome unknown".
            eprintln!("\n\x1b[1;33m[reprobe] {}\x1b[0m", outcome.journal_detail());
            eprintln!("{}", outcome.advice());
            eprintln!("journal: {}", recorder.path().display());
            eprintln!("next safe step: ktflash recover {}", recorder.path().display());
            return Err(outcome.journal_detail());
        }
    }

    println!("journal: {}", recorder.path().display());
    Ok(())
}

/// Poll the bus until the dongle settles, then apply [`proto::postflash::decide`].
///
/// Polls rather than returning on the first look: the device drops off the bus on RESET and the
/// host takes a moment to re-enumerate it, so an immediate check reliably reports `NoDevice`.
/// Returns early only on a verdict that cannot improve with more waiting.
fn wait_for_reprobe(
    expected: Option<&DeviceFingerprint>,
    timeout: Duration,
) -> proto::postflash::ReprobeOutcome {
    use proto::postflash::{decide, ReprobeObservation};

    let deadline = std::time::Instant::now() + timeout;
    loop {
        let devs = crate::scan();
        let obs = ReprobeObservation {
            bootloader_present: devs.iter().any(|d| matches!(d.mode, crate::Mode::Bootloader)),
            runtime: devs
                .iter()
                .find(|d| !matches!(d.mode, crate::Mode::Bootloader))
                .map(crate::fingerprint_from_dev),
        };
        let verdict = decide(&obs, expected);
        // Confirmed and Mismatch are both stable answers; the other two may still resolve as
        // the host finishes enumerating, so keep looking until the deadline.
        if verdict.is_success() || verdict.is_halt() || std::time::Instant::now() >= deadline {
            return verdict;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
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

/// `cmd_bootdiag`'s `--send` mode — proves the pipe is alive over whichever transport is
/// available, so macOS gets a working liveness check too.
///
/// UNVERIFIED against hardware: sends `KTM` and expects the `0x78` accept byte. This is a
/// stronger liveness test than the default (non-advancing) `bootdiag`, but it **does advance the
/// one-shot state machine** — callers must re-`unlock` before `flash-cdc` afterward. Wired in as
/// an explicit opt-in (APPLY.md step 5's resolution: default stays non-advancing, `--send` opts
/// into this).
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
