//! The CDC bootloader **download state machine**, transport-independent.
//!
//! ## Why this file exists
//!
//! Today the sequence lives inline in `cmd_flash_cdc` (`flasher/src/main.rs:907-995`), welded to
//! `rusb` closures. That means the one piece of logic that can brick a dongle is the one piece
//! that **cannot be unit-tested**. This module lifts it into `proto/` — generic over
//! [`crate::proto::cdc::Transport`] — so it is exercised by a fake bootloader with no hardware,
//! and so a serial transport ([`crate::serialtransport`]) can drive it on macOS.
//!
//! The sequence is transcribed step-for-step from the existing code (which is itself transcribed
//! from the vendor's `FUN_00b8c380`). **It is intended to be behaviourally identical.** Diff it
//! against `main.rs` before trusting it; any difference is a bug here, not an improvement.
//!
//! ### Intentional differences from the inline version
//!
//! 1. **Progress is a callback, not `println!`.** The caller decides what to print, so the
//!    "every 8th packet" throttling moves to `main.rs`. This module is silent.
//! 2. **Errors are [`CdcError`], not `String`.** `Timeout`/`Disconnected` survive as typed
//!    variants instead of being flattened into text, which the journal layer can act on.
//! 3. **Timings are a struct**, so tests can run the whole sequence in milliseconds.
//!
//! ### What is NOT changed
//!
//! Token order, ACK bytes, the >=13-byte `CHP` blob requirement, the drain-before-send
//! discipline, and the accumulate-until-ACK read loop. All byte-for-byte.
//!
//! > **Status: untested against hardware.** Unit-tested against the fake below only.

use std::time::{Duration, Instant};

use crate::proto::cdc::{CdcError, Transport};
use crate::proto::ktcdc::{ack, token, Packet};

/// The `CHP` reply is a chip-info blob, not a bare ACK; the vendor waits for at least this
/// many bytes before continuing (`main.rs`, `chp.len() < 13`).
pub const CHP_MIN_LEN: usize = 13;

/// Every wall-clock constant the sequence depends on, in one place so tests can shrink them.
///
/// Defaults reproduce the values currently hard-coded in `main.rs`.
#[derive(Clone, Copy, Debug)]
pub struct Timings {
    /// Per-command send/ACK budget. `main.rs:27` `TIMEOUT`.
    pub io: Duration,
    /// `KSTA` triggers the flash erase; the vendor waits ~6 s, we allow 8.
    pub erase: Duration,
    /// Per-data-packet ACK budget.
    pub block: Duration,
    /// How long a drain read waits before deciding the RX buffer is empty.
    pub drain: Duration,
    /// Upper bound on a single read slice while accumulating.
    pub poll: Duration,
    /// Lower bound on a single read slice (avoids a zero-timeout spin).
    pub poll_min: Duration,
    /// Read slice while accumulating the `CHP` blob.
    pub chip_poll: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Timings {
            io: Duration::from_millis(800),
            erase: Duration::from_millis(8000),
            block: Duration::from_millis(2000),
            drain: Duration::from_millis(20),
            poll: Duration::from_millis(300),
            poll_min: Duration::from_millis(5),
            chip_poll: Duration::from_millis(200),
        }
    }
}

impl Timings {
    /// Millisecond-scale timings for unit tests.
    pub fn fast() -> Self {
        Timings {
            io: Duration::from_millis(50),
            erase: Duration::from_millis(50),
            block: Duration::from_millis(50),
            drain: Duration::from_millis(1),
            poll: Duration::from_millis(5),
            poll_min: Duration::from_millis(1),
            chip_poll: Duration::from_millis(5),
        }
    }
}

/// Emitted as the sequence advances. Borrowed, so the caller prints without allocating.
///
/// Note the pairing: [`Progress::Sending`] fires **before** a command goes out,
/// [`Progress::Step`] after it is ACKed. The journal recorder
/// ([`crate::proto::ktcdc_journal`]) needs the *before* edge — `journal.record()` is documented
/// as "call this before issuing the destructive command it describes", because a crash between
/// sending `KSTA` and seeing its ACK still leaves the flash erased.
#[derive(Debug)]
pub enum Progress<'a> {
    /// A command is about to be written to the wire. Nothing has been sent yet.
    Sending { tag: &'a str },
    /// A fixed-token command was ACKed.
    Step { tag: &'a str, reply: &'a [u8] },
    /// The `CHP` chip-info blob came back.
    ChipInfo { reply: &'a [u8] },
    /// `KSTA` completed — the flash erase is done.
    Erased { reply: &'a [u8] },
    /// One data packet was written and ACKed. Fires for **every** packet; throttle in the caller.
    Packet {
        index: usize,
        total: usize,
        addr: u32,
        payload_len: usize,
        is_final: bool,
        reply: &'a [u8],
    },
    /// `RESET` was ACKed; the device should re-enumerate into the new firmware.
    Finished,
}

/// Add a command tag to a protocol error without flattening typed variants.
fn tag_err(tag: &str, e: CdcError) -> CdcError {
    match e {
        CdcError::Protocol(m) => CdcError::Protocol(format!("{tag}: {m}")),
        other => other,
    }
}

/// Drives the bootloader download sequence over any [`Transport`].
pub struct Driver<'a, T: Transport + ?Sized> {
    pipe: &'a mut T,
    pub timings: Timings,
}

impl<'a, T: Transport + ?Sized> Driver<'a, T> {
    pub fn new(pipe: &'a mut T) -> Self {
        Driver { pipe, timings: Timings::default() }
    }

    pub fn with_timings(pipe: &'a mut T, timings: Timings) -> Self {
        Driver { pipe, timings }
    }

    /// One read, mapping a timeout to "no bytes" rather than an error — the accumulate loops
    /// below decide when a timeout is actually fatal.
    fn read_once(&mut self, timeout: Duration) -> Result<Vec<u8>, CdcError> {
        match self.pipe.recv(timeout) {
            Ok(v) => Ok(v),
            Err(CdcError::Timeout) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Discard bytes already sitting in RX. Mirrors the vendor's post-ACK buffer clear
    /// (`FUN_006cb910`) — without it, a stale ACK satisfies the *next* command's wait.
    fn drain(&mut self) -> Result<(), CdcError> {
        let slice = self.timings.drain;
        while !self.read_once(slice)?.is_empty() {}
        Ok(())
    }

    /// Drain, send, then accumulate reads until one of `wants` appears or `total` elapses.
    ///
    /// Accumulation is required: ACKs batch and arrive split across reads, so a single read is
    /// not reliable. This mirrors the vendor searching an accumulated buffer with
    /// `QByteArray::indexOf`.
    fn exchange(
        &mut self,
        tag: &str,
        out: &[u8],
        wants: &[u8],
        total: Duration,
    ) -> Result<Vec<u8>, CdcError> {
        self.drain()?;
        self.pipe.send(out).map_err(|e| tag_err(tag, e))?;
        let deadline = Instant::now() + total;
        let mut acc: Vec<u8> = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let slice = left.min(self.timings.poll).max(self.timings.poll_min);
            acc.extend_from_slice(&self.read_once(slice)?);
            if acc.iter().any(|b| wants.contains(b)) {
                return Ok(acc);
            }
            if Instant::now() >= deadline {
                return Err(CdcError::Protocol(format!(
                    "{tag}: expected {wants:02x?}, got {acc:02x?} after {total:?} — aborting"
                )));
            }
        }
    }

    /// A fixed-token command expecting a bare [`ack::ACCEPT`].
    fn step(
        &mut self,
        tag: &str,
        out: &[u8],
        cb: &mut dyn FnMut(Progress<'_>),
    ) -> Result<(), CdcError> {
        cb(Progress::Sending { tag });
        let budget = self.timings.io;
        let reply = self.exchange(tag, out, &[ack::ACCEPT], budget)?;
        cb(Progress::Step { tag, reply: &reply });
        Ok(())
    }

    /// Run the full download sequence.
    ///
    /// `flag` selects the handshake path exactly as `cmd_flash_cdc` derives it from image byte
    /// `0x0F`: `0` = CHP-only then direct RESET, `1` = VER+KEY up front and INF-verify before
    /// RESET. **The flag=1 path is not hardware-verified** (same caveat as the inline version).
    ///
    /// Requires a *fresh* bootloader — the state machine is one-shot, so `unlock` must run
    /// immediately before this.
    pub fn run(
        &mut self,
        plan: &[Packet],
        flag: u32,
        cb: &mut dyn FnMut(Progress<'_>),
    ) -> Result<(), CdcError> {
        self.step("KTM", &token::KTM, cb)?;

        if flag != 0 {
            self.step("VER", &token::VER, cb)?; // flag=1 (secure/keyed) — NOT hardware-verified
            self.step("KEY", &token::KEY, cb)?;
        }

        // CHP answers with a chip-info blob, not a bare ACK, so it can't use `step`.
        cb(Progress::Sending { tag: "CHP" });
        self.drain()?;
        self.pipe.send(&token::CHP).map_err(|e| tag_err("CHP", e))?;
        let mut chp: Vec<u8> = Vec::new();
        let chp_deadline = Instant::now() + self.timings.io;
        while chp.len() < CHP_MIN_LEN && Instant::now() < chp_deadline {
            let slice = self.timings.chip_poll;
            chp.extend_from_slice(&self.read_once(slice)?);
        }
        if chp.len() < CHP_MIN_LEN {
            return Err(CdcError::Protocol(format!(
                "CHP(info): expected >={CHP_MIN_LEN} bytes, got {chp:02x?} — aborting"
            )));
        }
        cb(Progress::ChipInfo { reply: &chp });

        self.step("ERASE", &token::ERASE_SETUP, cb)?;
        self.step("PWO", &token::PWO, cb)?;

        // KSTA triggers the erase — it needs the long budget, not `io`.
        //
        // This `Sending` edge is the safety-critical one: once KSTA is on the wire the flash is
        // being erased, whether or not we live to see the ACK. A journal that only recorded
        // `Erased` on success would tell a crashed operator "nothing destructive happened".
        cb(Progress::Sending { tag: "KSTA" });
        let erase_budget = self.timings.erase;
        let reply = self.exchange("KSTA", &token::KSTA, &[ack::ACCEPT], erase_budget)?;
        cb(Progress::Erased { reply: &reply });

        let total = plan.len();
        let block_budget = self.timings.block;
        for (index, p) in plan.iter().enumerate() {
            let reply = self
                .exchange("DATA", &p.bytes, &[ack::BLOCK, ack::DONE], block_budget)
                .map_err(|e| match e {
                    CdcError::Protocol(m) => CdcError::Protocol(format!(
                        "packet {index} @0x{:05x}: {m} (image partial; re-unlock + reflash to recover)",
                        p.addr
                    )),
                    other => other,
                })?;
            cb(Progress::Packet {
                index,
                total,
                addr: p.addr,
                payload_len: p.payload_len,
                is_final: p.is_final,
                reply: &reply,
            });
        }

        self.step("STP", &token::STP, cb)?;
        if flag != 0 {
            self.step("INF", &token::INF, cb)?; // flag=1 only: read-back verify before RESET
        }
        self.step("RESET", &token::RESET, cb)?; // "ZRST" — reboot into the new firmware
        cb(Progress::Finished);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::ktcdc::plan_stream;
    use std::collections::VecDeque;

    /// How the fake should misbehave, so failure paths get covered too.
    #[derive(Default, Clone, Copy)]
    struct Faults {
        /// Split every ACK across two reads (a junk byte, then the ACK).
        split_acks: bool,
        /// Answer `CHP` with fewer than [`CHP_MIN_LEN`] bytes.
        short_chip_info: bool,
        /// Accept `KSTA` but never ACK it — i.e. die during the erase.
        silent_ksta: bool,
        /// Go silent starting at this data-packet index.
        silent_from_packet: Option<usize>,
        /// Report a disconnect at this data-packet index.
        disconnect_at_packet: Option<usize>,
    }

    /// A stand-in for the KT_USB_BOOT bootloader that speaks the real token protocol.
    ///
    /// Deliberately dumb: it matches whole tokens and treats anything else as a data packet.
    struct FakeKt {
        rx: VecDeque<Vec<u8>>,
        sent: Vec<Vec<u8>>,
        packets_seen: usize,
        faults: Faults,
    }

    impl FakeKt {
        fn new(faults: Faults) -> Self {
            FakeKt { rx: VecDeque::new(), sent: Vec::new(), packets_seen: 0, faults }
        }

        /// Tags of the fixed-token commands, in the order they were received.
        fn tags(&self) -> Vec<&'static str> {
            self.sent.iter().filter_map(|b| classify(b)).collect()
        }

        fn queue_ack(&mut self, byte: u8) {
            if self.faults.split_acks {
                self.rx.push_back(vec![0x00]); // noise first — forces accumulation
                self.rx.push_back(vec![byte]);
            } else {
                self.rx.push_back(vec![byte]);
            }
        }
    }

    fn classify(b: &[u8]) -> Option<&'static str> {
        match b {
            _ if b == token::KTM => Some("KTM"),
            _ if b == token::VER => Some("VER"),
            _ if b == token::KEY => Some("KEY"),
            _ if b == token::CHP => Some("CHP"),
            _ if b == token::ERASE_SETUP => Some("ERASE"),
            _ if b == token::PWO => Some("PWO"),
            _ if b == token::KSTA => Some("KSTA"),
            _ if b == token::STP => Some("STP"),
            _ if b == token::INF => Some("INF"),
            _ if b == token::RESET => Some("RESET"),
            _ => None,
        }
    }

    impl Transport for FakeKt {
        fn send(&mut self, bytes: &[u8]) -> Result<(), CdcError> {
            self.sent.push(bytes.to_vec());
            match classify(bytes) {
                Some("CHP") => {
                    let n = if self.faults.short_chip_info { 4 } else { CHP_MIN_LEN + 2 };
                    self.rx.push_back(vec![0x5au8; n]);
                }
                Some("KSTA") if self.faults.silent_ksta => {} // accepted, never ACKed
                Some(_) => self.queue_ack(ack::ACCEPT),
                // Anything unrecognised is a data packet.
                None => {
                    let i = self.packets_seen;
                    self.packets_seen += 1;
                    if self.faults.disconnect_at_packet == Some(i) {
                        return Err(CdcError::Disconnected);
                    }
                    if self.faults.silent_from_packet.map(|s| i >= s).unwrap_or(false) {
                        return Ok(()); // accepted, but never ACKed
                    }
                    self.queue_ack(ack::BLOCK);
                }
            }
            Ok(())
        }

        fn recv(&mut self, _timeout: Duration) -> Result<Vec<u8>, CdcError> {
            match self.rx.pop_front() {
                Some(v) => Ok(v),
                None => Err(CdcError::Timeout),
            }
        }
    }

    fn image() -> Vec<u8> {
        // Content is irrelevant to sequencing — only the packet plan's shape matters here.
        (0..4096u32).map(|i| (i % 251) as u8).collect()
    }

    fn run_with(faults: Faults, flag: u32) -> (FakeKt, Vec<Packet>, Result<(), CdcError>, usize) {
        let img = image();
        let plan = plan_stream(&img, 0);
        let mut fake = FakeKt::new(faults);
        let mut packet_events = 0usize;
        let res = {
            let mut d = Driver::with_timings(&mut fake, Timings::fast());
            d.run(&plan, flag, &mut |p| {
                if matches!(p, Progress::Packet { .. }) {
                    packet_events += 1;
                }
            })
        };
        (fake, plan, res, packet_events)
    }

    #[test]
    fn flag0_runs_the_documented_token_sequence() {
        let (fake, _plan, res, _) = run_with(Faults::default(), 0);
        assert!(res.is_ok(), "{res:?}");
        assert_eq!(
            fake.tags(),
            vec!["KTM", "CHP", "ERASE", "PWO", "KSTA", "STP", "RESET"],
            "flag=0 must not send VER/KEY/INF"
        );
    }

    #[test]
    fn flag1_inserts_ver_key_up_front_and_inf_before_reset() {
        let (fake, _plan, res, _) = run_with(Faults::default(), 1);
        assert!(res.is_ok(), "{res:?}");
        assert_eq!(
            fake.tags(),
            vec!["KTM", "VER", "KEY", "CHP", "ERASE", "PWO", "KSTA", "STP", "INF", "RESET"]
        );
    }

    #[test]
    fn every_planned_packet_is_sent_in_order() {
        let (fake, plan, res, events) = run_with(Faults::default(), 0);
        assert!(res.is_ok(), "{res:?}");
        assert_eq!(fake.packets_seen, plan.len());
        assert_eq!(events, plan.len(), "one Progress::Packet per planned packet");

        let data_sent: Vec<&Vec<u8>> =
            fake.sent.iter().filter(|b| classify(b).is_none()).collect();
        assert_eq!(data_sent.len(), plan.len());
        for (sent, planned) in data_sent.iter().zip(plan.iter()) {
            assert_eq!(*sent, &planned.bytes, "packet bytes must be the planner's, verbatim");
        }
    }

    #[test]
    fn acks_split_across_reads_are_accumulated() {
        let faults = Faults { split_acks: true, ..Default::default() };
        let (fake, plan, res, _) = run_with(faults, 0);
        assert!(res.is_ok(), "a noise byte before the ACK must not abort: {res:?}");
        assert_eq!(fake.packets_seen, plan.len());
    }

    #[test]
    fn short_chip_info_aborts_before_erasing() {
        let faults = Faults { short_chip_info: true, ..Default::default() };
        let (fake, _plan, res, _) = run_with(faults, 0);
        match res {
            Err(CdcError::Protocol(m)) => assert!(m.contains("CHP(info)"), "{m}"),
            other => panic!("expected a CHP protocol error, got {other:?}"),
        }
        assert!(
            !fake.tags().contains(&"ERASE"),
            "must not reach the erase step after a bad CHP — this is the safety-critical ordering"
        );
    }

    #[test]
    fn a_missing_block_ack_aborts_with_packet_context() {
        let faults = Faults { silent_from_packet: Some(2), ..Default::default() };
        let (_fake, _plan, res, _) = run_with(faults, 0);
        match res {
            Err(CdcError::Protocol(m)) => {
                assert!(m.contains("packet 2"), "error must name the packet index: {m}");
                assert!(m.contains("re-unlock + reflash"), "must state the recovery path: {m}");
            }
            other => panic!("expected a data-packet protocol error, got {other:?}"),
        }
    }

    #[test]
    fn disconnect_mid_stream_stays_a_typed_error() {
        let faults = Faults { disconnect_at_packet: Some(1), ..Default::default() };
        let (_fake, _plan, res, _) = run_with(faults, 0);
        assert_eq!(
            res,
            Err(CdcError::Disconnected),
            "Disconnected must survive as a variant so the journal layer can act on it"
        );
    }

    /// Record the command edges as `>TAG` (about to send) / `<TAG` (ACKed).
    fn edge_log(faults: Faults, flag: u32) -> (Vec<String>, Result<(), CdcError>) {
        let img = image();
        let plan = plan_stream(&img, 0);
        let mut fake = FakeKt::new(faults);
        let mut log: Vec<String> = Vec::new();
        let res = {
            let mut d = Driver::with_timings(&mut fake, Timings::fast());
            d.run(&plan, flag, &mut |p| match p {
                Progress::Sending { tag } => log.push(format!(">{tag}")),
                Progress::Step { tag, .. } => log.push(format!("<{tag}")),
                Progress::ChipInfo { .. } => log.push("<CHP".into()),
                Progress::Erased { .. } => log.push("<KSTA".into()),
                _ => {}
            })
        };
        (log, res)
    }

    #[test]
    fn every_command_announces_itself_before_going_on_the_wire() {
        let (log, res) = edge_log(Faults::default(), 0);
        assert!(res.is_ok(), "{res:?}");
        assert_eq!(
            &log[..10],
            &[">KTM", "<KTM", ">CHP", "<CHP", ">ERASE", "<ERASE", ">PWO", "<PWO", ">KSTA", "<KSTA"],
            "each command must emit Sending before Step"
        );
        assert_eq!(log.last().map(String::as_str), Some("<RESET"));
    }

    #[test]
    fn ksta_is_announced_even_when_the_erase_never_acks() {
        // THE safety property behind Progress::Sending. KSTA triggers the erase; if it goes out
        // and we then die waiting for the ACK, the flash is erased anyway. A journal driven off
        // the success edge alone would tell a crashed operator "nothing destructive happened"
        // and invite them to walk away from a half-erased dongle.
        let faults = Faults { silent_ksta: true, ..Default::default() };
        let (log, res) = edge_log(faults, 0);
        assert!(res.is_err(), "the erase should have timed out");
        assert!(log.contains(&">KSTA".to_string()), "KSTA must be announced: {log:?}");
        assert!(!log.contains(&"<KSTA".to_string()), "…but never confirmed: {log:?}");
    }

    #[test]
    fn stale_rx_bytes_are_drained_before_each_command() {
        let img = image();
        let plan = plan_stream(&img, 0);
        let mut fake = FakeKt::new(Faults::default());
        // Pre-load junk that would satisfy the first wait if it were not drained.
        fake.rx.push_back(vec![ack::ACCEPT, ack::ACCEPT, ack::ACCEPT]);
        let res = {
            let mut d = Driver::with_timings(&mut fake, Timings::fast());
            d.run(&plan, 0, &mut |_| {})
        };
        assert!(res.is_ok(), "{res:?}");
    }
}
