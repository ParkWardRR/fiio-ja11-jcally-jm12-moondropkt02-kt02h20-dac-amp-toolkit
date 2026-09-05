//! ktflash — a small, pretty tool to identify and drive KTMicro `KT02H20` USB‑C DAC
//! dongles (the FiiO JA11 family) from **macOS + OrbStack**.
//!
//!   run with no args      → the live TUI dashboard
//!   ktflash probe         → one‑shot device report (scriptable)
//!   ktflash handshake     → normal mode: send the 0x4B/0x33 HID frame, read reply
//!   ktflash unlock        → send "T12345678" → reboot into the CDC bootloader
//!   ktflash bootdiag      → probe the 0x8888:0xCDC0 bootloader bulk pipe
//!
//! `unlock`/`handshake`/`bootdiag` *claim the USB interface*, which macOS refuses
//! (`IOHIDFamily` owns it → LIBUSB_ERROR_ACCESS). Run them inside the OrbStack Linux
//! guest (see ../orbstack/). Identification (`probe`, the TUI) works natively on macOS.

use rusb::{Context, Direction, TransferType, UsbContext};
use std::time::Duration;

mod boottransport;
mod cmd_flash_cdc;
mod proto;
mod serialtransport;
mod tui;
mod usbtransport;

const KTMICRO_VID: u16 = 0x31b2; // stock KTMicro KT02H20 dongles
const NORMAL_PID: u16 = 0x0111;
const FIIO_VID: u16 = 0x2972; // FiiO / JadeAudio
const JA11_PID: u16 = 0x0102;
const BOOT_VID: u16 = 0x8888; // KT_USB_BOOT CDC bootloader
const BOOT_PID: u16 = 0xcdc0;
const TIMEOUT: Duration = Duration::from_millis(800);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Ja11,       // already running FiiO JA11 firmware
    KtStock,    // stock KT02H20 (0x31b2:0x0111)
    KtFamily,   // some other KTMicro 0x31b2 dongle
    Bootloader, // in KT_USB_BOOT ISP mode (0x8888:0xcdc0)
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Ja11 => "JadeAudio JA11",
            Mode::KtStock => "KT02H20 (stock)",
            Mode::KtFamily => "KTMicro dongle",
            Mode::Bootloader => "KT_USB_BOOT bootloader",
        }
    }
}

#[derive(Clone)]
pub struct Iface {
    pub num: u8,
    pub class: u8,
    pub sub: u8,
    pub proto: u8,
    pub eps: Vec<String>,
}

#[derive(Clone)]
pub struct Dev {
    pub vid: u16,
    pub pid: u16,
    pub mfr: String,
    pub product: String,
    pub serial: String,
    pub mode: Mode,
    pub ifaces: Vec<Iface>,
}

fn classify(vid: u16, pid: u16) -> Option<Mode> {
    match (vid, pid) {
        (BOOT_VID, BOOT_PID) => Some(Mode::Bootloader),
        (FIIO_VID, JA11_PID) => Some(Mode::Ja11),
        (KTMICRO_VID, NORMAL_PID) => Some(Mode::KtStock),
        (KTMICRO_VID, _) => Some(Mode::KtFamily),
        _ => None,
    }
}

fn class_name(c: u8) -> &'static str {
    match c {
        1 => "Audio",
        2 => "CDC",
        3 => "HID",
        10 => "CDC-Data",
        _ => "?",
    }
}

/// Enumerate every recognised dongle (any mode).
pub fn scan() -> Vec<Dev> {
    let ctx = match Context::new() {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let mut out = vec![];
    let devs = match ctx.devices() {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    for dev in devs.iter() {
        let d = match dev.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };
        let mode = match classify(d.vendor_id(), d.product_id()) {
            Some(m) => m,
            None => continue,
        };
        let (mfr, product, serial) = match dev.open() {
            Ok(h) => {
                let lang = h.read_languages(TIMEOUT).ok().and_then(|l| l.first().copied());
                let g = |idx: u8| -> String {
                    match (lang, idx) {
                        (Some(l), i) if i != 0 => {
                            h.read_string_descriptor(l, i, TIMEOUT).unwrap_or_default()
                        }
                        _ => String::new(),
                    }
                };
                (
                    g(d.manufacturer_string_index().unwrap_or(0)),
                    g(d.product_string_index().unwrap_or(0)),
                    g(d.serial_number_string_index().unwrap_or(0)),
                )
            }
            Err(_) => (String::new(), String::new(), String::new()),
        };
        let mut ifaces = vec![];
        if let Ok(cfg) = dev.active_config_descriptor() {
            for iface in cfg.interfaces() {
                for id in iface.descriptors() {
                    let eps = id
                        .endpoint_descriptors()
                        .map(|e| {
                            let dir = if e.direction() == Direction::In { "IN" } else { "OUT" };
                            let tt = match e.transfer_type() {
                                TransferType::Interrupt => "intr",
                                TransferType::Bulk => "bulk",
                                TransferType::Isochronous => "iso",
                                TransferType::Control => "ctrl",
                            };
                            format!("0x{:02x} {dir} {tt}", e.address())
                        })
                        .collect();
                    ifaces.push(Iface {
                        num: id.interface_number(),
                        class: id.class_code(),
                        sub: id.sub_class_code(),
                        proto: id.protocol_code(),
                        eps,
                    });
                }
            }
        }
        out.push(Dev { vid: d.vendor_id(), pid: d.product_id(), mfr, product, serial, mode, ifaces });
    }
    out
}

// ---------- headless CLI ----------

fn cmd_probe() -> Result<(), String> {
    let devs = scan();
    if devs.is_empty() {
        return Err("no KTMicro-family / FiiO JA11 / bootloader device found".into());
    }
    for d in &devs {
        println!(
            "● {:04x}:{:04x}  [{}]  {:?} — {:?}  serial={:?}",
            d.vid, d.pid, d.mode.label(), d.mfr, d.product, d.serial
        );
        for i in &d.ifaces {
            println!(
                "    IF{} {} ({}/{}/{})  [{}]",
                i.num, class_name(i.class), i.class, i.sub, i.proto, i.eps.join(", ")
            );
        }
    }
    Ok(())
}

fn open_hid(ctx: &Context) -> Result<(rusb::DeviceHandle<Context>, u8, u8, u8), String> {
    for dev in ctx.devices().map_err(|e| e.to_string())?.iter() {
        let d = dev.device_descriptor().map_err(|e| e.to_string())?;
        let is_normal = (d.vendor_id() == KTMICRO_VID && d.product_id() == NORMAL_PID)
            || (d.vendor_id() == FIIO_VID && d.product_id() == JA11_PID);
        if !is_normal {
            continue;
        }
        let cfg = dev.active_config_descriptor().map_err(|e| e.to_string())?;
        for iface in cfg.interfaces() {
            for id in iface.descriptors() {
                if id.class_code() != 3 {
                    continue;
                }
                let mut out_ep = None;
                let mut in_ep = None;
                for e in id.endpoint_descriptors() {
                    if e.transfer_type() == TransferType::Interrupt {
                        match e.direction() {
                            Direction::Out => out_ep = Some(e.address()),
                            Direction::In => in_ep = Some(e.address()),
                        }
                    }
                }
                if let (Some(o), Some(i)) = (out_ep, in_ep) {
                    let h = dev.open().map_err(|e| format!("open: {e}"))?;
                    let _ = h.set_auto_detach_kernel_driver(true);
                    h.claim_interface(id.interface_number()).map_err(|e| {
                        format!("claim iface {}: {e}\n  macOS blocks this — run inside the OrbStack guest (see ../orbstack/).", id.interface_number())
                    })?;
                    return Ok((h, id.interface_number(), o, i));
                }
            }
        }
    }
    Err("no normal-mode dongle with a HID interface found (KTMicro 0x31b2:0x0111 or FiiO 0x2972:0x0102)".into())
}

/// Send the "T12345678" unlock; returns a human message. Used by CLI + TUI.
pub fn try_unlock() -> Result<String, String> {
    let ctx = Context::new().map_err(|e| e.to_string())?;
    let (h, iface, out_ep, _in) = open_hid(&ctx)?;
    let mut pkt = vec![0x54u8];
    pkt.extend_from_slice(b"12345678");
    pkt.push(0x00);
    let n = h.write_interrupt(out_ep, &pkt, TIMEOUT).map_err(|e| format!("write unlock: {e}"))?;
    let _ = h.release_interface(iface);
    Ok(format!("sent T12345678 ({n}B) → device rebooting to bootloader {BOOT_VID:04x}:{BOOT_PID:04x}"))
}

fn cmd_unlock() -> Result<(), String> {
    println!("{}", try_unlock()?);
    Ok(())
}

fn cmd_handshake() -> Result<(), String> {
    let ctx = Context::new().map_err(|e| e.to_string())?;
    let (h, iface, out_ep, in_ep) = open_hid(&ctx)?;
    let frame = [0x4bu8, 0, 0, 0, 0, 0x33, 0x00, 0, 0, 0, 0];
    h.write_interrupt(out_ep, &frame, TIMEOUT).map_err(|e| format!("write: {e}"))?;
    let mut resp = [0u8; 16];
    match h.read_interrupt(in_ep, &mut resp, TIMEOUT) {
        Ok(nr) if nr >= 11 => {
            let v = u32::from_le_bytes([resp[7], resp[8], resp[9], resp[10]]);
            println!("status word = {v} (expect 3 in ISP)");
        }
        Ok(nr) => println!("short reply ({nr} bytes)"),
        Err(e) => println!("no reply ({e})"),
    }
    let _ = h.release_interface(iface);
    Ok(())
}

/// Parse and validate a `KT_Helios` firmware image (no hardware needed).
fn cmd_image(path: &str) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    let img = proto::image::KtHelios::parse(&bytes).map_err(|e| e.to_string())?;
    println!("● {path}  ({} bytes)", bytes.len());
    println!("    magic       {}", img.magic);
    println!("    chip        {}", img.chip);
    println!("    declared    {} (0x{:X})", img.declared_size, img.declared_size);
    println!("    build       {} {}", img.build_hash, img.build_date);
    println!("    ENTY        load 0x{:08X}  size 0x{:X}", img.enty_load_addr, img.enty_size);
    for (lvl, msg) in img.findings(bytes.len()) {
        println!("    {} {msg}", lvl.tag());
    }
    if img.is_structurally_valid(bytes.len()) {
        Ok(())
    } else {
        Err("image failed structural validation (see ✗ above)".into())
    }
}

/// Build a fingerprint from a scanned device. NOTE (working agent): `descriptor_sha256` here
/// is a *deterministic personality hash* of the interface/endpoint table, not the raw USB
/// config descriptor — good enough to distinguish personalities, but upgrade it to a hash of
/// the raw descriptor bytes (via libusb) for the strongest manifest matches. `bcd_device` is
/// not captured by `scan()` yet; add it to `Dev` if you want bcd-gated manifest rules.
fn fingerprint_from_dev(d: &Dev) -> proto::fingerprint::DeviceFingerprint {
    let mut fp = proto::fingerprint::DeviceFingerprint::new(d.vid, d.pid);
    fp.manufacturer = (!d.mfr.is_empty()).then(|| d.mfr.clone());
    fp.product = (!d.product.is_empty()).then(|| d.product.clone());
    fp.serial_present = !d.serial.is_empty();
    let mut canon = String::new();
    for i in &d.ifaces {
        canon.push_str(&format!(
            "IF{} {}/{}/{} [{}]\n",
            i.num, i.class, i.sub, i.proto, i.eps.join(",")
        ));
    }
    fp.descriptor_sha256 = Some(proto::plan::sha256_hex(canon.as_bytes()));
    fp
}

/// The first attached non-bootloader device, fingerprinted. Read-only; works on macOS.
fn probe_fingerprint() -> Option<proto::fingerprint::DeviceFingerprint> {
    scan().iter().find(|d| !matches!(d.mode, Mode::Bootloader)).map(fingerprint_from_dev)
}

/// `ktflash fingerprint` — emit a structured fingerprint for every recognised device (JSON).
fn cmd_fingerprint() -> Result<(), String> {
    let devs = scan();
    if devs.is_empty() {
        return Err("no recognised device found to fingerprint".into());
    }
    let fps: Vec<_> = devs.iter().map(fingerprint_from_dev).collect();
    println!("{}", serde_json::to_string_pretty(&fps).map_err(|e| e.to_string())?);
    Ok(())
}

/// `ktflash recover <journal.json>` — read a flash journal and report the single safe next
/// action (roadmap M2 recovery-state model). Hardware-free; the actual re-drive is wired once
/// the CDC writer exists.
fn cmd_recover(path: &str) -> Result<(), String> {
    let j = proto::journal::Journal::load(std::path::Path::new(path))?;
    let sha = &j.image_sha256;
    println!(
        "journal {}  device {}  image {}… ({} bytes)",
        j.op_id,
        j.device.short(),
        &sha[..sha.len().min(12)],
        j.image_len
    );
    println!("last recorded stage: {:?}", j.last_stage());
    println!("safe next action:    {:?}", j.safe_next_action());
    Ok(())
}

/// Dispatch `ktflash flash …`.
fn cmd_flash(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("--plan") => {
            let path = args
                .get(1)
                .ok_or("usage: ktflash flash --plan <fw.bin> [--device VID:PID] [--manifest m.json]")?;
            let mut device = None;
            let mut manifest_path = None;
            let mut rest = args[2..].iter();
            while let Some(flag) = rest.next() {
                match flag.as_str() {
                    "--device" => {
                        device = Some(rest.next().ok_or("--device needs a VID:PID value")?.clone())
                    }
                    "--manifest" => {
                        manifest_path = Some(rest.next().ok_or("--manifest needs a path")?.clone())
                    }
                    other => return Err(format!("unknown flag for `flash --plan`: {other}")),
                }
            }
            cmd_flash_plan(path, device, manifest_path)
        }
        Some("--apply") => {
            let path = args
                .get(1)
                .ok_or("usage: ktflash flash --apply <plan.json> [--execute] [--force-unsupported-device VID:PID]")?;
            let execute = args[2..].iter().any(|a| a == "--execute");
            let force = args
                .iter()
                .position(|a| a == "--force-unsupported-device")
                .and_then(|i| args.get(i + 1).cloned());
            cmd_flash_apply(path, execute, force)
        }
        _ => Err("usage: ktflash flash --plan <fw.bin> | ktflash flash --apply <plan.json>".into()),
    }
}

/// Stage 1 of a flash: build + print a FlashPlan for an image (no hardware, non-destructive).
/// Device identity comes from `--device VID:PID` if given, else a live read-only probe;
/// `--manifest` supplies the allow-list that can upgrade the verdict to PROCEED.
fn cmd_flash_plan(
    path: &str,
    device_str: Option<String>,
    manifest_path: Option<String>,
) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    let device = match device_str {
        Some(s) => Some(proto::fingerprint::DeviceFingerprint::parse_short(&s)?),
        None => probe_fingerprint(),
    };
    let manifest = match manifest_path {
        Some(p) => {
            let s = std::fs::read_to_string(&p).map_err(|e| format!("read manifest {p}: {e}"))?;
            Some(proto::manifest::Manifest::from_json(&s).map_err(|e| format!("manifest {p}: {e}"))?)
        }
        None => None,
    };
    let plan = proto::plan::FlashPlan::build(
        path,
        &bytes,
        env!("CARGO_PKG_VERSION"),
        proto::plan::now_epoch_secs(),
        device,
        manifest.as_ref(),
    );
    println!("{}", plan.to_json_pretty());
    eprintln!("\n── gate ──");
    for g in &plan.gate {
        let mark = if g.passed { "✓" } else { "✗" };
        eprintln!("  {mark} {} [{}] — {}", g.name, g.confidence, g.detail);
    }
    eprintln!("decision: {}", plan.decision.label());
    match plan.decision {
        proto::plan::Decision::Refuse => Err("image rejected by the gate — not flashable".into()),
        proto::plan::Decision::NeedsConfirmation => {
            eprintln!(
                "save this plan and apply with:  ktflash flash --apply <plan.json>\n\
                 (a real apply will require --force-unsupported-device until a firmware manifest exists)"
            );
            Ok(())
        }
        proto::plan::Decision::Proceed => Ok(()),
    }
}

/// Stage 2 of a flash: re-validate the image against a saved plan, enforce the decision gate,
/// and (only with `--execute`) attempt the native flash. Without `--execute` this is a dry,
/// hardware-free re-verification.
fn cmd_flash_apply(path: &str, execute: bool, force: Option<String>) -> Result<(), String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let plan = proto::plan::FlashPlan::from_json(&s).map_err(|e| format!("parse {path}: {e}"))?;
    let bytes = std::fs::read(&plan.image_path)
        .map_err(|e| format!("read image {}: {e}", plan.image_path))?;
    plan.reverify_image(&bytes)?;
    println!(
        "plan re-verified: {} ({} bytes, sha256 {}…)",
        plan.image_path,
        plan.image_len,
        &plan.image_sha256[..plan.image_sha256.len().min(12)]
    );

    // Hard guardrail: never apply a plan the gate did not clear to PROCEED. REFUSE is absolute.
    // NeedsConfirmation is reject-by-default; the ONLY escape hatch is an explicit override that
    // repeats the exact target device id (roadmap M2), which is logged permanently in the journal.
    let mut override_note: Option<String> = None;
    match plan.decision {
        proto::plan::Decision::Refuse => {
            return Err("plan decision is REFUSE — the image failed the gate; will not flash".into())
        }
        proto::plan::Decision::NeedsConfirmation => {
            let plan_dev = plan
                .device
                .as_ref()
                .map(|d| d.short())
                .ok_or("plan decision is NEEDS CONFIRMATION and the plan has no device identity — \
                        re-run `flash --plan` with --device VID:PID (and ideally a --manifest)")?;
            match force {
                None => {
                    return Err("plan decision is NEEDS CONFIRMATION — refusing by default. Provide a \
                                --manifest that covers this image+device, or override with \
                                --force-unsupported-device VID:PID (dangerous)"
                        .into())
                }
                Some(ref id) => {
                    // The override must repeat the exact device the plan targets.
                    let given = proto::fingerprint::DeviceFingerprint::parse_short(id)?.short();
                    if given != plan_dev {
                        return Err(format!(
                            "--force-unsupported-device {given} does not match the plan's device {plan_dev} — refusing"
                        ));
                    }
                    eprintln!(
                        "⚠️  OVERRIDE: forcing an UNSUPPORTED/UNVERIFIED flash for {plan_dev}. This can \
                         brick the device. Proceeding because you repeated the exact device id."
                    );
                    override_note = Some(format!("FORCED override for unsupported device {plan_dev}"));
                }
            }
        }
        proto::plan::Decision::Proceed => {}
    }

    if !execute {
        let mode = if override_note.is_some() { "FORCED" } else { "PROCEED" };
        println!("dry run OK ({mode}) — pass --execute to attempt the flash on the attached bootloader");
        return Ok(());
    }
    run_native_flash(&plan, &bytes, override_note)
}

/// The **plan/apply** flash path: runs the safety gate + journal, opens the real bootloader
/// transport, and drives the `Message`-based CDC [`Session`]. That `Session` uses
/// [`proto::cdc::PendingCodec`], so its on-device write fails fast with `Unreversed` and
/// touches nothing — **the shipped native write is `ktflash flash-cdc`** (byte-exact framing
/// in [`proto::ktcdc`]). This path is kept for the plan/manifest/journal machinery; wiring it
/// to `ktcdc` (with per-stage journaling) is a future consolidation.
fn run_native_flash(
    plan: &proto::plan::FlashPlan,
    bytes: &[u8],
    override_note: Option<String>,
) -> Result<(), String> {
    use proto::cdc::{FlashStage, PendingCodec, RetryPolicy, Session};
    use proto::journal::{Journal, Stage};

    let device = plan
        .device
        .clone()
        .or_else(probe_fingerprint)
        .ok_or("no device fingerprint available for the journal (attach the device or set --device at plan time)")?;
    let op_id = format!("{}-{}", proto::plan::now_epoch_secs(), device.short().replace(':', "-"));
    let jpath = proto::journal::journal_path(&op_id)
        .map_err(|e| format!("create journal dir {}: {e}", proto::journal::data_dir().display()))?;

    let mut journal = Journal::new(
        op_id,
        env!("CARGO_PKG_VERSION"),
        plan.image_sha256.clone(),
        plan.image_len,
        device,
    );
    // Permanently record a forced override, if any, before anything else.
    let staged_detail = match &override_note {
        Some(note) => format!("plan re-verified; {note}; opening bootloader"),
        None => "plan re-verified; opening bootloader".to_string(),
    };
    journal.record(Stage::Staged, proto::plan::now_epoch_secs(), staged_detail);
    journal.save(&jpath).map_err(|e| format!("write journal {}: {e}", jpath.display()))?;

    let transport = usbtransport::RusbBootloaderTransport::open()?;
    let mut session = Session::new(transport, PendingCodec, RetryPolicy::default());

    // TODO(working-agent): confirm the real program chunk size from an M0 capture.
    const CHUNK: usize = 256;

    // NOTE: this coarse journaling via the progress callback is a starting point. The robust
    // approach (see proto::journal notes) is for the writer to record each stage *before*
    // issuing its destructive command. Revisit when KtCdcCodec lands.
    let result = {
        let jpath = jpath.clone();
        session.flash(bytes, CHUNK, |stage, done, total| {
            let jstage = match stage {
                FlashStage::Handshake => Stage::Handshake,
                FlashStage::Erase => Stage::Erased,
                FlashStage::Program => Stage::Programming,
                FlashStage::Verify => Stage::Programmed, // all chunks sent, verify pending
                FlashStage::Reset => Stage::ResetIssued,
                FlashStage::Done => Stage::Verified,
                FlashStage::Idle | FlashStage::Failed => return,
            };
            if stage == FlashStage::Program {
                journal.record_progress(proto::plan::now_epoch_secs(), done);
            } else {
                journal.record(jstage, proto::plan::now_epoch_secs(), format!("{done}/{total} bytes"));
            }
            let _ = journal.save(&jpath);
        })
    };

    match result {
        Ok(()) => {
            // A real success still requires a post-reset reprobe to confirm identity before
            // claiming Confirmed (roadmap M2). TODO(working-agent): reprobe here and record
            // Confirmed or IdentityMismatch accordingly.
            journal.record(Stage::Confirmed, proto::plan::now_epoch_secs(), "flash reported success");
            let _ = journal.save(&jpath);
            println!("flash complete; journal: {}", jpath.display());
            Ok(())
        }
        Err(e) => {
            journal.record(Stage::Failed, proto::plan::now_epoch_secs(), e.to_string());
            let _ = journal.save(&jpath);
            Err(format!("flash failed: {e}\nrecovery: ktflash recover {}", jpath.display()))
        }
    }
}

/// `ktflash transcript --from-tshark <fields.txt> [--out t.json]` — turn the output of the
/// documented `tshark` invocation into a normalized, replayable transcript (roadmap Phase 2 M0).
/// Hardware-free: it just decodes text. Feed the result to `ktflash bootdiag --replay`.
fn cmd_transcript(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("--from-tshark") => {
            let path = args
                .get(1)
                .ok_or("usage: ktflash transcript --from-tshark <fields.txt> [--out t.json]")?;
            let mut out = None;
            let mut rest = args[2..].iter();
            while let Some(flag) = rest.next() {
                match flag.as_str() {
                    "--out" => out = Some(rest.next().ok_or("--out needs a path")?.clone()),
                    other => return Err(format!("unknown flag for transcript: {other}")),
                }
            }
            let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
            let t = proto::transcript::Transcript::from_tshark(&text, path);
            let json = t.to_json_pretty();
            match out {
                Some(p) => {
                    std::fs::write(&p, &json).map_err(|e| format!("write {p}: {e}"))?;
                    println!("wrote {} frame(s) to {p}", t.frames.len());
                }
                None => println!("{json}"),
            }
            Ok(())
        }
        _ => Err("usage: ktflash transcript --from-tshark <fields.txt> [--out t.json]".into()),
    }
}

/// `ktflash compat --template | --validate <matrix.json>` — the M6 compatibility matrix
/// (roadmap Phase 6). Hardware-free; validates that each record's claimed confidence is backed
/// by its evidence, and flags which entries are actually recommendable.
fn cmd_compat(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("--template") => {
            let m = proto::compat::CompatMatrix { records: vec![proto::compat::template()] };
            println!("{}", m.to_json_pretty());
            Ok(())
        }
        Some("--validate") => {
            let p = args.get(1).ok_or("usage: ktflash compat --validate <matrix.json>")?;
            let s = std::fs::read_to_string(p).map_err(|e| format!("read {p}: {e}"))?;
            let m = proto::compat::CompatMatrix::from_json(&s).map_err(|e| format!("parse {p}: {e}"))?;
            println!("compat: {} record(s)", m.records.len());
            for (i, r) in m.records.iter().enumerate() {
                let rec = if r.is_recommended() { "  (recommended)" } else { "" };
                println!("  [{i}] {} — {:?}{rec}", r.device.marketing_name, r.confidence);
            }
            let problems = m.check_all();
            if problems.is_empty() {
                println!("ok — all records validate");
                Ok(())
            } else {
                for (i, ps) in &problems {
                    for p in ps {
                        eprintln!("  ✗ [{i}] {p}");
                    }
                }
                Err(format!("{} record(s) failed validation", problems.len()))
            }
        }
        _ => Err("usage: ktflash compat --template | ktflash compat --validate <matrix.json>".into()),
    }
}

fn parse_num(s: &str) -> Result<u64, String> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).map_err(|e| format!("bad hex {s:?}: {e}")),
        None => s.parse::<u64>().map_err(|e| format!("bad number {s:?}: {e}")),
    }
}

/// `ktflash dump --addr 0xADDR --len N [--out file]` — a raw normal-mode `0x08` word-read
/// **diagnostic**. It does **NOT** read firmware flash on the KT02H20.
///
/// Confirmed on hardware + by full RE (see `docs/CDC-PROTOCOL.md`, `research/cdc-re-findings.md`):
/// the runtime `0xFF01` HID collection is an audio/EQ dispatcher with no `0x08` flash-read case,
/// so these reads return zeros / don't reach flash. **There is no software firmware backup on
/// this silicon** — keep the manufacturer's original image. This command is retained only as a
/// low-level probe of the `0x08` word-read behaviour, not as a backup.
fn cmd_dump(args: &[String]) -> Result<(), String> {
    let (mut addr, mut len, mut out) = (None, None, None);
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--addr" => addr = Some(parse_num(it.next().ok_or("--addr needs a value")?)?),
            "--len" => len = Some(parse_num(it.next().ok_or("--len needs a value")?)?),
            "--out" => out = Some(it.next().ok_or("--out needs a path")?.clone()),
            other => return Err(format!("unknown flag for dump: {other}")),
        }
    }
    let addr = addr.ok_or("usage: ktflash dump --addr 0xADDR --len N [--out file]")? as u32;
    let len = len.ok_or("usage: ktflash dump --addr 0xADDR --len N [--out file]")? as usize;

    eprintln!("raw 0x08 word-read diagnostic: {len} bytes @ 0x{addr:08x} (normal-mode HID).");
    eprintln!("NOTE: this does NOT read firmware flash on the KT02H20 — expect zeros. There is");
    eprintln!("no software firmware backup on this chip (see docs/CDC-PROTOCOL.md). Not a backup.");

    let ctx = Context::new().map_err(|e| e.to_string())?;
    let (h, iface, out_ep, in_ep) = open_hid(&ctx)?;
    let mut buf = Vec::with_capacity(len);
    let mut a = addr;
    let mut result: Result<(), String> = Ok(());
    while buf.len() < len {
        let frame = proto::frame::command_frame(a, proto::frame::cmd::READ, 0);
        if let Err(e) = h.write_interrupt(out_ep, &frame, TIMEOUT) {
            result = Err(format!("write @0x{a:08x}: {e}"));
            break;
        }
        let mut resp = [0u8; 16];
        match h.read_interrupt(in_ep, &mut resp, TIMEOUT) {
            Ok(_) => match proto::frame::result_word(&resp) {
                Some(w) => buf.extend_from_slice(&w.to_le_bytes()),
                None => {
                    result = Err(format!("short reply @0x{a:08x}"));
                    break;
                }
            },
            Err(e) => {
                result = Err(format!("read @0x{a:08x}: {e}"));
                break;
            }
        }
        a = a.wrapping_add(4);
    }
    let _ = h.release_interface(iface);
    result?;

    buf.truncate(len);
    match out {
        Some(p) => {
            std::fs::write(&p, &buf).map_err(|e| format!("write {p}: {e}"))?;
            println!("wrote {} bytes to {p}  (sha256 {})", buf.len(), proto::plan::sha256_hex(&buf));
        }
        None => println!("{}", proto::transcript::to_hex(&buf)),
    }
    Ok(())
}

/// Decode + validate a normalized capture transcript with no hardware (roadmap M0).
fn cmd_bootdiag_replay(path: &str) -> Result<(), String> {
    use proto::cdc::FrameCodec;
    let s = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let t = proto::transcript::Transcript::from_json(&s).map_err(|e| format!("parse {path}: {e}"))?;
    let total = t.validate()?;
    println!("replay {path} — {} frame(s), {total} bytes  [{}]", t.frames.len(), t.source);
    if !t.note.is_empty() {
        println!("  note: {}", t.note);
    }
    let codec = proto::cdc::ReferenceCodec;
    for (i, f) in t.frames.iter().enumerate() {
        let bytes = f.bytes().map_err(|e| format!("frame {i}: {e}"))?;
        // ReferenceCodec decodes synthetic fixtures; real captures show as <raw> until the
        // KTMicro wire codec lands (roadmap M1).
        let decoded = codec
            .decode(&bytes)
            .map(|m| format!("{m:?}"))
            .unwrap_or_else(|_| "<raw>".to_string());
        println!(
            "  [{i:>3}] {} {:<9} {:>4}B  {decoded}",
            f.direction.arrow(),
            f.stage.name(),
            bytes.len()
        );
    }
    println!("ok — {} frame(s) decoded/validated (no hardware touched)", t.frames.len());
    Ok(())
}

fn cmd_bootdiag() -> Result<(), String> {
    let ctx = Context::new().map_err(|e| e.to_string())?;
    let dev = ctx
        .devices()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|d| {
            d.device_descriptor()
                .map(|dd| dd.vendor_id() == BOOT_VID && dd.product_id() == BOOT_PID)
                .unwrap_or(false)
        })
        .ok_or("bootloader not present — run `unlock` first")?;
    let cfg = dev.active_config_descriptor().map_err(|e| e.to_string())?;
    let mut target = None;
    for iface in cfg.interfaces() {
        for id in iface.descriptors() {
            if id.class_code() != 10 {
                continue;
            }
            let (mut o, mut i) = (None, None);
            for e in id.endpoint_descriptors() {
                if e.transfer_type() == TransferType::Bulk {
                    match e.direction() {
                        Direction::Out => o = Some(e.address()),
                        Direction::In => i = Some(e.address()),
                    }
                }
            }
            if let (Some(o), Some(i)) = (o, i) {
                target = Some((id.interface_number(), o, i));
            }
        }
    }
    let (iface, out_ep, in_ep) = target.ok_or("no CDC-data bulk interface")?;
    let h = dev.open().map_err(|e| format!("open: {e}"))?;
    let _ = h.set_auto_detach_kernel_driver(true);
    h.claim_interface(iface).map_err(|e| format!("claim iface {iface}: {e} (OrbStack/Linux only)"))?;
    let _ = h.write_control(0x21, 0x20, 0, 0, &[0x00, 0xc2, 0x01, 0x00, 0x00, 0x00, 0x08], TIMEOUT);
    let _ = h.write_control(0x21, 0x22, 0x0003, 0, &[], TIMEOUT);
    println!("bootloader claimed; bulk out=0x{out_ep:02x} in=0x{in_ep:02x}.");
    println!("(the CDC download framing is implemented in `flash-cdc`; this just proves the pipe is live)");
    let _ = h.release_interface(iface);
    Ok(())
}

/// Compact hex preview of the first `n` bytes of a slice.
fn hexline(b: &[u8], n: usize) -> String {
    let n = n.min(b.len());
    let mut s = b[..n].iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ");
    if b.len() > n {
        s.push_str(" …");
    }
    s
}

const HELP: &str = "\
ktflash — KTMicro KT02H20 / FiiO JA11 dongle tool

USAGE:
  ktflash            live TUI dashboard (macOS-friendly, read-only)
  ktflash probe       one-shot device report
  ktflash fingerprint structured device fingerprint (JSON)   (no hardware write)
  ktflash handshake   normal mode HID handshake        (OrbStack/Linux)
  ktflash unlock      reboot into the CDC bootloader    (OrbStack/Linux)
  ktflash bootdiag    probe the bootloader bulk pipe    (OrbStack/Linux)
  ktflash image <f>   parse/validate a KT_Helios image  (no hardware)
  ktflash flash --plan <fw.bin> [--device VID:PID] [--manifest m.json]
                      build a flash plan + run the safety gate  (no hardware)
  ktflash flash --apply <plan.json>
                      re-verify an image against a saved plan   (no hardware yet)
  ktflash recover <journal.json>
                      report the safe next action for an interrupted flash
  ktflash dump --addr 0xADDR --len N [--out f]
                      raw 0x08 word-read diagnostic — does NOT read flash / NOT a backup
  ktflash flash-cdc --image <fw.bin> [--flag 0|1] [--base 0xADDR]
                    [--transport auto|serial|usb] [--port <dev>] [--execute --yes]
                      native CDC bootloader write; dry-run unless --execute.
                      --yes required to write: save a known-good image first (no read-back).
  ktflash compat --template | --validate <matrix.json>
                      emit/validate a compatibility record    (no hardware)
  ktflash transcript --from-tshark <fields.txt> [--out t.json]
                      convert tshark capture output to a transcript (no hardware)
  ktflash bootdiag --replay <t.json>
                      decode/validate a capture transcript      (no hardware)

Docs: https://github.com/ParkWardRR/fiio-ja11-jcally-jm12-moondropkt02-kt02h20-dac-amp-toolkit";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let r = match args.get(1).map(String::as_str) {
        None => tui::run(),
        Some("demo") => tui::run_demo(),
        Some("probe") => cmd_probe(),
        Some("fingerprint") => cmd_fingerprint(),
        Some("handshake") => cmd_handshake(),
        Some("unlock") => cmd_unlock(),
        Some("image") => match args.get(2) {
            Some(p) => cmd_image(p),
            None => Err("usage: ktflash image <fw.bin>".to_string()),
        },
        Some("flash") => cmd_flash(&args[2..]),
        Some("recover") => match args.get(2) {
            Some(p) => cmd_recover(p),
            None => Err("usage: ktflash recover <journal.json>".to_string()),
        },
        Some("dump") => cmd_dump(&args[2..]),
        Some("flash-cdc") => cmd_flash_cdc::cmd_flash_cdc(&args[2..]),
        Some("compat") => cmd_compat(&args[2..]),
        Some("transcript") => cmd_transcript(&args[2..]),
        Some("bootdiag") => match args.get(2).map(String::as_str) {
            Some("--replay") => match args.get(3) {
                Some(p) => cmd_bootdiag_replay(p),
                None => Err("usage: ktflash bootdiag --replay <transcript.json>".to_string()),
            },
            _ => cmd_bootdiag(),
        },
        Some("-h") | Some("--help") | Some("help") => {
            println!("{HELP}");
            return;
        }
        Some(other) => {
            eprintln!("unknown command '{other}'\n\n{HELP}");
            std::process::exit(2);
        }
    };
    if let Err(e) = r {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
