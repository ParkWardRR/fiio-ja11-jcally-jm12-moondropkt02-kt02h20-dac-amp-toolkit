//! Journaling for the `flash-cdc` write path.
//!
//! ## The gap this closes
//!
//! ROADMAP Appendix A: *"Destructive writes ship **only** with recovery semantics."*
//!
//! That holds for `ktflash flash --apply`, which builds a [`Journal`] and records stages as it
//! goes (`main.rs:492-553`). It does **not** hold for `ktflash flash-cdc` — the path that is
//! actually proven on hardware and that users are told to run. It writes flash with no journal
//! at all, so an interrupted `flash-cdc` leaves nothing for `ktflash recover` to read, and the
//! operator is left guessing whether the erase happened.
//!
//! `main.rs:484-485` already names this: *"wiring it to `ktcdc` (with per-stage journaling) is a
//! future consolidation."* This module is that wiring.
//!
//! ## Ordering discipline
//!
//! [`Journal::record`] is documented as "call this **before** issuing the destructive command it
//! describes". So the recorder keys off [`Progress::Sending`] — the pre-write edge — for
//! anything destructive, and off the success edges only for stages that genuinely mean "this
//! completed".
//!
//! | driver event | journal stage | why this edge |
//! |---|---|---|
//! | `Step{KTM}` | `Handshake` | non-destructive; success edge is fine |
//! | `Sending{KSTA}` | `Erased` | **KSTA triggers the erase.** Once it is on the wire the flash is gone whether or not we see the ACK |
//! | `Packet{..}` | `Programming(bytes)` | bytes *confirmed* written, so the success edge is correct here |
//! | `Step{STP}` | `Programmed` | all packets ACKed |
//! | `Step{INF}` | `Verified` | flag=1 read-back verify passed |
//! | `Sending{RESET}` | `ResetIssued` | after RESET the device re-enumerates and may stop answering |
//! | error | `Failed` | detail carries the error text |
//!
//! `Confirmed` is deliberately **not** recorded here: it means "post-reset reprobe saw the
//! expected normal-mode identity", which this module cannot observe. The caller records it
//! after a successful reprobe — see [`JournalRecorder::confirm`].
//!
//! > **Status: untested against hardware.** Unit-tested only.

use std::path::{Path, PathBuf};

use crate::proto::fingerprint::DeviceFingerprint;
use crate::proto::journal::{journal_path, Journal, Stage};
use crate::proto::ktcdc_driver::Progress;

/// How often to persist a `Programming` progress record, in packets.
///
/// Every packet would mean a file write per kilobyte of firmware — enough I/O to slow the flash
/// and add failure modes to the thing whose job is surviving failure. Every 16 packets bounds
/// the "how far did it get" uncertainty to ~16 KB, and recovery reflashes from the start anyway
/// unless the protocol proves resume is safe (it does not, yet).
const JOURNAL_EVERY_PACKETS: usize = 16;

/// Owns a [`Journal`] and drives it from [`Progress`] events.
pub struct JournalRecorder {
    journal: Journal,
    path: PathBuf,
    bytes_done: usize,
    packets_since_save: usize,
    /// First I/O error while persisting, if any. Recorded so the caller can warn loudly: a
    /// journal that silently stopped being written is worse than no journal.
    write_error: Option<String>,
}

impl JournalRecorder {
    /// Create a journal for this operation and write its `Staged` entry immediately — before
    /// any command reaches the device, so even an instant failure leaves a trace.
    pub fn begin(
        device: DeviceFingerprint,
        image_sha256: impl Into<String>,
        image_len: usize,
        tool_version: &str,
        detail: impl Into<String>,
    ) -> Result<Self, String> {
        let op_id = format!(
            "{}-{}",
            crate::proto::plan::now_epoch_secs(),
            device.short().replace(':', "-")
        );
        let path = journal_path(&op_id).map_err(|e| {
            format!("create journal dir {}: {e}", crate::proto::journal::data_dir().display())
        })?;
        let mut journal =
            Journal::new(op_id, tool_version, image_sha256, image_len, device);
        journal.record(Stage::Staged, crate::proto::plan::now_epoch_secs(), detail);
        journal
            .save(&path)
            .map_err(|e| format!("write journal {}: {e}", path.display()))?;
        Ok(JournalRecorder {
            journal,
            path,
            bytes_done: 0,
            packets_since_save: 0,
            write_error: None,
        })
    }

    /// Same, but at an explicit path — used by tests, and by callers that want the journal
    /// somewhere other than the user data dir.
    pub fn begin_at(
        path: PathBuf,
        device: DeviceFingerprint,
        image_sha256: impl Into<String>,
        image_len: usize,
        tool_version: &str,
        detail: impl Into<String>,
    ) -> Result<Self, String> {
        let op_id = format!("manual-{}", device.short().replace(':', "-"));
        let mut journal = Journal::new(op_id, tool_version, image_sha256, image_len, device);
        journal.record(Stage::Staged, crate::proto::plan::now_epoch_secs(), detail);
        journal
            .save(&path)
            .map_err(|e| format!("write journal {}: {e}", path.display()))?;
        Ok(JournalRecorder {
            journal,
            path,
            bytes_done: 0,
            packets_since_save: 0,
            write_error: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Any error encountered while persisting. Non-empty means recovery data may be stale.
    pub fn write_error(&self) -> Option<&str> {
        self.write_error.as_deref()
    }

    fn save(&mut self) {
        if let Err(e) = self.journal.save(&self.path) {
            // Keep only the first error: subsequent ones are almost certainly the same cause,
            // and we must not let journal I/O abort a flash that is already in progress.
            if self.write_error.is_none() {
                self.write_error = Some(format!("{}: {e}", self.path.display()));
            }
        }
    }

    fn record(&mut self, stage: Stage, detail: impl Into<String>) {
        self.journal.record(stage, crate::proto::plan::now_epoch_secs(), detail);
        self.save();
    }

    /// Feed one driver event in. Call this from the `flash-cdc` progress callback.
    pub fn observe(&mut self, ev: &Progress<'_>) {
        match *ev {
            // --- pre-write edges: destructive, must be durable BEFORE the bytes go out ---
            Progress::Sending { tag: "KSTA" } => {
                self.record(Stage::Erased, "KSTA sent — flash erase triggered");
            }
            Progress::Sending { tag: "RESET" } => {
                self.record(Stage::ResetIssued, "RESET sent — device should re-enumerate");
            }
            Progress::Sending { .. } => {}

            // --- success edges ---
            Progress::Step { tag: "KTM", .. } => {
                self.record(Stage::Handshake, "bootloader handshake ok");
            }
            Progress::Step { tag: "STP", .. } => {
                self.record(
                    Stage::Programmed,
                    format!("all packets acked ({} bytes)", self.bytes_done),
                );
            }
            Progress::Step { tag: "INF", .. } => {
                self.record(Stage::Verified, "INF read-back verify passed");
            }
            Progress::Step { .. } => {}
            Progress::ChipInfo { .. } | Progress::Erased { .. } => {}

            Progress::Packet { payload_len, index, total, .. } => {
                self.bytes_done += payload_len;
                self.packets_since_save += 1;
                let last = index + 1 == total;
                if self.packets_since_save >= JOURNAL_EVERY_PACKETS || last {
                    self.packets_since_save = 0;
                    self.journal
                        .record_progress(crate::proto::plan::now_epoch_secs(), self.bytes_done);
                    self.save();
                }
            }

            Progress::Finished => {}
        }
    }

    /// Record a failure. Call this on **any** error path out of the driver — the whole point of
    /// the journal is that an aborted flash leaves an actionable record.
    pub fn fail(&mut self, detail: impl Into<String>) {
        self.record(Stage::Failed, detail);
    }

    /// Record post-reset success. Only call after a reprobe actually saw the expected
    /// normal-mode identity — `Confirmed` is what makes `ktflash recover` say "done".
    pub fn confirm(&mut self, detail: impl Into<String>) {
        self.record(Stage::Confirmed, detail);
    }

    /// Record that the post-reset device is not what we expected. This is a halt state: the
    /// recovery model deliberately refuses to pick another image automatically.
    pub fn identity_mismatch(&mut self, detail: impl Into<String>) {
        self.record(Stage::IdentityMismatch, detail);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::journal::RecoveryAction;
    use crate::proto::ktcdc::plan_stream;

    fn recorder(dir: &Path) -> JournalRecorder {
        JournalRecorder::begin_at(
            dir.join("t.journal.json"),
            DeviceFingerprint::new(0x31b2, 0x0111),
            "abc123",
            4096,
            "test",
            "unit test",
        )
        .expect("begin")
    }

    fn tmpdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ktflash-journal-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_fresh_journal_says_nothing_destructive_happened() {
        let d = tmpdir();
        let r = recorder(&d);
        assert_eq!(r.journal().safe_next_action(), RecoveryAction::CancelOrBegin);
        assert!(r.path().exists(), "the Staged entry must be on disk immediately");
    }

    #[test]
    fn ksta_being_sent_is_enough_to_mark_the_flash_erased() {
        // The property this whole module exists for: dying between "KSTA on the wire" and its
        // ACK must still tell the operator the dongle needs reflashing.
        let d = tmpdir();
        let mut r = recorder(&d);
        r.observe(&Progress::Step { tag: "KTM", reply: &[0x78] });
        r.observe(&Progress::Sending { tag: "KSTA" });
        // …crash here…
        let reloaded = Journal::load(r.path()).expect("journal on disk");
        assert_eq!(
            reloaded.safe_next_action(),
            RecoveryAction::ReflashStaged { from_bytes: 0 },
            "an erased device must demand a reflash, not 'cancel or begin'"
        );
    }

    #[test]
    fn progress_records_confirmed_bytes_and_survives_a_reload() {
        let d = tmpdir();
        let mut r = recorder(&d);
        r.observe(&Progress::Sending { tag: "KSTA" });
        let img: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let plan = plan_stream(&img, 0);
        for (index, p) in plan.iter().enumerate() {
            r.observe(&Progress::Packet {
                index,
                total: plan.len(),
                addr: p.addr,
                payload_len: p.payload_len,
                is_final: p.is_final,
                reply: &[0xa5],
            });
        }
        let reloaded = Journal::load(r.path()).unwrap();
        let expected: usize = plan.iter().map(|p| p.payload_len).sum();
        assert_eq!(reloaded.max_bytes_done(), expected, "final packet must always be journalled");
        match reloaded.safe_next_action() {
            RecoveryAction::ReflashStaged { from_bytes } => assert_eq!(from_bytes, expected),
            other => panic!("expected ReflashStaged, got {other:?}"),
        }
    }

    #[test]
    fn the_journal_is_not_rewritten_for_every_single_packet() {
        // Bounded I/O: a file write per kilobyte would slow the flash and add failure modes to
        // the mechanism whose job is surviving failure.
        let d = tmpdir();
        let mut r = recorder(&d);
        for index in 0..JOURNAL_EVERY_PACKETS * 3 {
            r.observe(&Progress::Packet {
                index,
                total: 1000, // never "last"
                addr: 0,
                payload_len: 1024,
                is_final: false,
                reply: &[0xa5],
            });
        }
        let n = r.journal().entries.iter().filter(|e| e.bytes_done.is_some()).count();
        assert_eq!(n, 3, "expected one progress entry per {JOURNAL_EVERY_PACKETS} packets");
    }

    #[test]
    fn a_full_successful_run_ends_in_reset_issued_not_confirmed() {
        // Confirmed requires a post-reset reprobe, which the driver cannot observe. Getting
        // this wrong would make `ktflash recover` claim success for a device that never
        // came back.
        let d = tmpdir();
        let mut r = recorder(&d);
        r.observe(&Progress::Step { tag: "KTM", reply: &[0x78] });
        r.observe(&Progress::Sending { tag: "KSTA" });
        r.observe(&Progress::Erased { reply: &[0x78] });
        r.observe(&Progress::Packet {
            index: 0, total: 1, addr: 0, payload_len: 1008, is_final: true, reply: &[0xa5],
        });
        r.observe(&Progress::Step { tag: "STP", reply: &[0x78] });
        r.observe(&Progress::Sending { tag: "RESET" });
        r.observe(&Progress::Step { tag: "RESET", reply: &[0x78] });
        r.observe(&Progress::Finished);

        assert_eq!(r.journal().last_stage(), Some(Stage::ResetIssued));
        assert_eq!(r.journal().safe_next_action(), RecoveryAction::WaitAndReprobe);

        r.confirm("reprobe saw 2972:0102");
        assert_eq!(r.journal().safe_next_action(), RecoveryAction::Done);
    }

    #[test]
    fn a_failure_after_the_erase_still_demands_a_reflash() {
        let d = tmpdir();
        let mut r = recorder(&d);
        r.observe(&Progress::Sending { tag: "KSTA" });
        r.observe(&Progress::Packet {
            index: 0, total: 100, addr: 0, payload_len: 1008, is_final: false, reply: &[0xa5],
        });
        r.fail("packet 1: timed out");
        match Journal::load(r.path()).unwrap().safe_next_action() {
            RecoveryAction::ReflashStaged { .. } => {}
            other => panic!("a failure after erase must demand a reflash, got {other:?}"),
        }
    }

    #[test]
    fn a_failure_before_anything_destructive_is_safe_to_cancel() {
        let d = tmpdir();
        let mut r = recorder(&d);
        r.observe(&Progress::Step { tag: "KTM", reply: &[0x78] });
        r.fail("CHP(info): expected >=13 bytes");
        assert_eq!(
            Journal::load(r.path()).unwrap().safe_next_action(),
            RecoveryAction::CancelOrBegin,
            "failing before KSTA must not scare the user into reflashing a healthy dongle"
        );
    }

    #[test]
    fn an_unwritable_journal_is_reported_not_silently_ignored() {
        // A journal that stopped being written is worse than no journal, because the operator
        // trusts it. Persisting must never abort a flash in progress, but it must be visible.
        let d = tmpdir().join("nonexistent-subdir");
        let mut r = JournalRecorder::begin_at(
            d.join("t.json"),
            DeviceFingerprint::new(0x31b2, 0x0111),
            "abc",
            1,
            "test",
            "x",
        )
        .err();
        assert!(r.take().is_some(), "begin_at must fail loudly if it cannot write");
    }
}
