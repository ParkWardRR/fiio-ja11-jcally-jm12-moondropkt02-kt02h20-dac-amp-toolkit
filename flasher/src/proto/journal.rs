//! The **operation journal** and **recovery-state model** (roadmap M2 "flash as a durable
//! transaction").
//!
//! A flash is a sequence of destructive, interruptible steps. If the cable is yanked during
//! `program`, or the host sleeps, or the serial handle dies, the device is left in a known —
//! and *recoverable* — intermediate state, **but only if we wrote down what we were doing
//! before we did it.** This module is that write-ahead log plus the logic that reads a
//! journal back and decides the single safe next action.
//!
//! Design rules (all enforced here, all hardware-free and tested):
//!   1. Persist the plan (staged image hash, device fingerprint, transport) and each stage
//!      transition **before** issuing the corresponding destructive command.
//!   2. The journal is append-only JSON-lines-friendly and human-inspectable.
//!   3. Never infer success from USB re-enumeration alone — a post-reset reprobe must confirm
//!      the expected identity (that confirmation is recorded as its own entry).
//!   4. If the device identity changes unexpectedly after a reset, **stop** and preserve
//!      evidence; do not auto-select another image.
//!
//! ## Working-agent notes
//! - The real writer (M2, blocked on M1's wire codec) must call [`Journal::record`] right
//!   *before* each `expect`/send of a destructive command, and persist with [`Journal::save`]
//!   after each append (append-safe: rewrite the whole small file, or switch to append-only
//!   lines if you prefer — keep it crash-safe).
//! - `ktflash recover <journal.json>` surfaces [`Journal::safe_next_action`]; wire the actual
//!   re-drive once the writer exists.
//! - Store journals under a user-local data dir (e.g. `$XDG_DATA_HOME/ktflash` or
//!   `~/Library/Application Support/ktflash`), keyed by `op_id`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::fingerprint::DeviceFingerprint;

/// Pure resolver for the journal data directory, so the precedence is unit-testable without
/// touching the real environment. Precedence: `KTFLASH_DATA_DIR` → `$XDG_DATA_HOME/ktflash` →
/// `~/Library/Application Support/ktflash` (macOS) → `~/.local/share/ktflash`.
fn resolve_data_dir(
    ktflash_data_dir: Option<String>,
    xdg_data_home: Option<String>,
    home: Option<String>,
    is_macos: bool,
) -> PathBuf {
    if let Some(d) = ktflash_data_dir.filter(|s| !s.is_empty()) {
        return PathBuf::from(d);
    }
    if let Some(x) = xdg_data_home.filter(|s| !s.is_empty()) {
        return PathBuf::from(x).join("ktflash");
    }
    let base = PathBuf::from(home.filter(|s| !s.is_empty()).unwrap_or_else(|| ".".into()));
    if is_macos {
        base.join("Library/Application Support/ktflash")
    } else {
        base.join(".local/share/ktflash")
    }
}

/// The user-local directory where operation journals live.
pub fn data_dir() -> PathBuf {
    resolve_data_dir(
        std::env::var("KTFLASH_DATA_DIR").ok(),
        std::env::var("XDG_DATA_HOME").ok(),
        std::env::var("HOME").ok(),
        cfg!(target_os = "macos"),
    )
}

/// Ensure the data dir exists and return the journal path for `op_id`.
pub fn journal_path(op_id: &str) -> std::io::Result<PathBuf> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("{op_id}.journal.json")))
}

/// The flashing stages, in order. Mirrors [`super::cdc::FlashStage`] but is the *persisted*
/// form (serde) and includes explicit post-reset confirmation states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    /// Plan validated and staged; no destructive command sent yet.
    Staged,
    /// Handshake with the bootloader completed.
    Handshake,
    /// Erase command acknowledged — flash is now blank; MUST reprogram.
    Erased,
    /// At least one program chunk written, image not yet complete.
    Programming,
    /// All chunks written; verification not yet confirmed.
    Programmed,
    /// Protocol-level verify passed.
    Verified,
    /// Reset issued; normal-mode identity not yet re-confirmed.
    ResetIssued,
    /// Post-reset reprobe confirmed the expected normal-mode identity. Success.
    Confirmed,
    /// A step failed; `detail` explains. The device may be mid-operation.
    Failed,
    /// Post-reset identity differs from expectation — halt, preserve evidence.
    IdentityMismatch,
}

/// One append-only journal record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub stage: Stage,
    pub epoch_secs: u64,
    #[serde(default)]
    pub detail: String,
    /// For `Programming`: bytes written so far, so a restart knows where it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_done: Option<usize>,
}

/// The durable record of one flash transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    /// Unique per operation (e.g. `"20260904T1505-31b2-0111"`).
    pub op_id: String,
    pub tool_version: String,
    /// The staged image's SHA-256 — what a restart MUST reprogram (never a different image).
    pub image_sha256: String,
    pub image_len: usize,
    /// The device this operation targets, as fingerprinted before starting.
    pub device: DeviceFingerprint,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

/// The single safe next action derived from a journal's latest state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Nothing destructive happened; safe to cancel or (re)start cleanly.
    CancelOrBegin,
    /// The device is (partly) erased/written — it MUST be reprogrammed with the *staged*
    /// image. Re-enter the bootloader and reflash from the start (or from `bytes_done` if the
    /// protocol proves resume is safe).
    ReflashStaged { from_bytes: usize },
    /// Programmed but verify unknown — verify, or reflash the staged image.
    VerifyOrReflash,
    /// Reset issued but normal mode not yet confirmed — wait and reprobe; if the bootloader
    /// reappears, reflash the staged image.
    WaitAndReprobe,
    /// Done and confirmed — no action needed.
    Done,
    /// Identity changed unexpectedly — STOP. Preserve evidence; a human must intervene. Do
    /// not auto-select another image.
    Halt { reason: String },
}

impl Journal {
    pub fn new(
        op_id: impl Into<String>,
        tool_version: impl Into<String>,
        image_sha256: impl Into<String>,
        image_len: usize,
        device: DeviceFingerprint,
    ) -> Journal {
        Journal {
            op_id: op_id.into(),
            tool_version: tool_version.into(),
            image_sha256: image_sha256.into(),
            image_len,
            device,
            entries: Vec::new(),
        }
    }

    /// Append a stage transition. Call this **before** issuing the destructive command it
    /// describes, then [`save`](Self::save).
    pub fn record(&mut self, stage: Stage, epoch_secs: u64, detail: impl Into<String>) {
        self.entries.push(Entry { stage, epoch_secs, detail: detail.into(), bytes_done: None });
    }

    /// Append a `Programming` progress record.
    pub fn record_progress(&mut self, epoch_secs: u64, bytes_done: usize) {
        self.entries.push(Entry {
            stage: Stage::Programming,
            epoch_secs,
            detail: String::new(),
            bytes_done: Some(bytes_done),
        });
    }

    pub fn last_stage(&self) -> Option<Stage> {
        self.entries.last().map(|e| e.stage)
    }

    /// The furthest byte offset any `Programming` entry recorded.
    pub fn max_bytes_done(&self) -> usize {
        self.entries.iter().filter_map(|e| e.bytes_done).max().unwrap_or(0)
    }

    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> Result<Journal, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }

    /// Persist the journal. Rewrites the whole (small) file, which is crash-safe enough for a
    /// human-inspectable log; the caller is expected to call this after every [`record`].
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_json_pretty())
    }

    pub fn load(path: &std::path::Path) -> Result<Journal, String> {
        let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Journal::from_json(&s)
    }

    /// The heart of the recovery model: given where we got to, what is the *one* safe next
    /// action? See the state table in `docs`/`ROADMAP.md`.
    pub fn safe_next_action(&self) -> RecoveryAction {
        match self.last_stage() {
            None | Some(Stage::Staged) => RecoveryAction::CancelOrBegin,
            Some(Stage::Handshake) => RecoveryAction::CancelOrBegin,
            Some(Stage::Erased) => RecoveryAction::ReflashStaged { from_bytes: 0 },
            Some(Stage::Programming) => {
                RecoveryAction::ReflashStaged { from_bytes: self.max_bytes_done() }
            }
            Some(Stage::Programmed) => RecoveryAction::VerifyOrReflash,
            Some(Stage::Verified) | Some(Stage::ResetIssued) => RecoveryAction::WaitAndReprobe,
            Some(Stage::Confirmed) => RecoveryAction::Done,
            Some(Stage::IdentityMismatch) => RecoveryAction::Halt {
                reason: "post-reset device identity does not match the target — preserve \
                         evidence and do not flash another image"
                    .into(),
            },
            Some(Stage::Failed) => {
                // A failure's safe action depends on the furthest destructive stage reached.
                if self.entries.iter().any(|e| {
                    matches!(e.stage, Stage::Erased | Stage::Programming | Stage::Programmed)
                }) {
                    RecoveryAction::ReflashStaged { from_bytes: self.max_bytes_done() }
                } else {
                    RecoveryAction::CancelOrBegin
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal() -> Journal {
        Journal::new("op1", "test", "abc", 1000, DeviceFingerprint::new(0x31b2, 0x0111))
    }

    #[test]
    fn nothing_done_is_cancel_or_begin() {
        let j = journal();
        assert_eq!(j.safe_next_action(), RecoveryAction::CancelOrBegin);
    }

    #[test]
    fn erased_requires_reflash_from_zero() {
        let mut j = journal();
        j.record(Stage::Handshake, 1, "");
        j.record(Stage::Erased, 2, "erase acked");
        assert_eq!(j.safe_next_action(), RecoveryAction::ReflashStaged { from_bytes: 0 });
    }

    #[test]
    fn interrupted_program_resumes_from_furthest_offset() {
        let mut j = journal();
        j.record(Stage::Erased, 2, "");
        j.record_progress(3, 256);
        j.record_progress(4, 512);
        // cable yanked here — last stage is Programming
        assert_eq!(j.last_stage(), Some(Stage::Programming));
        assert_eq!(j.safe_next_action(), RecoveryAction::ReflashStaged { from_bytes: 512 });
    }

    #[test]
    fn programmed_offers_verify_or_reflash() {
        let mut j = journal();
        j.record(Stage::Programmed, 5, "all chunks sent");
        assert_eq!(j.safe_next_action(), RecoveryAction::VerifyOrReflash);
    }

    #[test]
    fn reset_without_confirmation_waits_and_reprobes() {
        let mut j = journal();
        j.record(Stage::Verified, 6, "");
        j.record(Stage::ResetIssued, 7, "");
        assert_eq!(j.safe_next_action(), RecoveryAction::WaitAndReprobe);
    }

    #[test]
    fn confirmed_is_done() {
        let mut j = journal();
        j.record(Stage::Confirmed, 8, "reprobe ok");
        assert_eq!(j.safe_next_action(), RecoveryAction::Done);
    }

    #[test]
    fn identity_mismatch_halts() {
        let mut j = journal();
        j.record(Stage::IdentityMismatch, 9, "saw 2972:0102, expected 31b2:0111");
        assert!(matches!(j.safe_next_action(), RecoveryAction::Halt { .. }));
    }

    #[test]
    fn failure_after_erase_still_requires_reflash() {
        let mut j = journal();
        j.record(Stage::Erased, 2, "");
        j.record(Stage::Failed, 3, "serial handle died");
        assert_eq!(j.safe_next_action(), RecoveryAction::ReflashStaged { from_bytes: 0 });
    }

    #[test]
    fn failure_before_anything_destructive_is_safe() {
        let mut j = journal();
        j.record(Stage::Handshake, 1, "");
        j.record(Stage::Failed, 2, "handshake timeout");
        assert_eq!(j.safe_next_action(), RecoveryAction::CancelOrBegin);
    }

    #[test]
    fn data_dir_precedence() {
        // explicit override wins
        assert_eq!(
            resolve_data_dir(Some("/tmp/kt".into()), Some("/xdg".into()), Some("/home/u".into()), true),
            PathBuf::from("/tmp/kt")
        );
        // then XDG
        assert_eq!(
            resolve_data_dir(None, Some("/xdg".into()), Some("/home/u".into()), true),
            PathBuf::from("/xdg/ktflash")
        );
        // then macOS Application Support
        assert_eq!(
            resolve_data_dir(None, None, Some("/Users/u".into()), true),
            PathBuf::from("/Users/u/Library/Application Support/ktflash")
        );
        // then XDG default on non-macOS
        assert_eq!(
            resolve_data_dir(None, None, Some("/home/u".into()), false),
            PathBuf::from("/home/u/.local/share/ktflash")
        );
        // empty strings are ignored
        assert_eq!(
            resolve_data_dir(Some(String::new()), Some(String::new()), Some("/home/u".into()), false),
            PathBuf::from("/home/u/.local/share/ktflash")
        );
    }

    #[test]
    fn journal_round_trips_json() {
        let mut j = journal();
        j.record(Stage::Erased, 2, "x");
        j.record_progress(3, 256);
        assert_eq!(Journal::from_json(&j.to_json_pretty()).unwrap(), j);
    }
}
