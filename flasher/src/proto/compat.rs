//! Machine-readable **compatibility records** (roadmap M6/Phase 6).
//!
//! Keeps "technically flashes" strictly separate from "safe to recommend". A record is only
//! allowed to *claim* as much as its evidence supports — [`CompatRecord::check`] enforces that a
//! `Verified`/`RestoreVerified` confidence is backed by the corresponding functional results, so
//! nobody can mark a dongle "recommended" on a shared VID/PID and an anecdote.
//!
//! Audio-only success is not enough: mic, gain, buttons, channel balance, suspend/resume, and
//! the control channel all matter, and each is tracked as an explicit [`Outcome`] (defaulting to
//! `Untested`, never silently "pass").
//!
//! Pure data + validation; unit-tested with no hardware. Mirrors Appendix C of `ROADMAP.md`.
//!
//! ## Working-agent notes
//! - Ship a curated `compat.json` in-repo as evidence accrues; gate additions on `check()`
//!   passing. `ktflash compat --template` emits a blank record; `--validate <f>` checks a file.
//! - Fill `usb_normal.descriptor_sha256` from a real probe so the record ties to a concrete
//!   USB personality (see `fingerprint` notes), not just VID/PID.

use serde::{Deserialize, Serialize};

/// The confidence ladder. Ordered: a bigger value is a stronger claim. `Unsupported`/`Unsafe`
/// are terminal side-states, kept lowest so they never out-rank a real result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Confidence {
    /// Known to be harmful/bricking — never recommend.
    Unsafe,
    /// Explicitly out of scope / won't work.
    Unsupported,
    /// Shared VID/PID or community report only; unproven. The safe default.
    #[default]
    Candidate,
    /// Someone observed a flash, limited evidence.
    Observed,
    /// Flash + core functions verified on a real unit.
    Verified,
    /// Full read → flash → restore → verify cycle proven.
    RestoreVerified,
}

/// A tri-state functional result — the default is `Untested`, so silence never reads as success.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Pass,
    Fail,
    #[default]
    Untested,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlashStatus {
    Success,
    Failed,
    #[default]
    NotAttempted,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsbId {
    pub vid: String,
    pub pid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bcd_device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_descriptor_sha256: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub marketing_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_revision: Option<String>,
    pub usb_normal: UsbId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootloader: Option<UsbId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redistribution: Option<String>,
}

/// The functional outcomes that decide whether a flash is *recommendable*, not just possible.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionalResult {
    pub flash_status: FlashStatus,
    pub postflash_audio: Outcome,
    pub microphone: Outcome,
    pub app_control_channel: Outcome,
    pub volume_gain: Outcome,
    pub buttons: Outcome,
    pub channel_balance: Outcome,
    pub suspend_resume: Outcome,
    #[serde(default)]
    pub restore_tested: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_before_descriptor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_after_descriptor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootloader_capture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_log: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contributor_attestation: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatRecord {
    pub device: DeviceInfo,
    pub firmware: FirmwareRef,
    #[serde(default)]
    pub result: FunctionalResult,
    #[serde(default)]
    pub evidence: Evidence,
    pub confidence: Confidence,
}

impl CompatRecord {
    /// Validate that the *claimed* confidence is actually supported by the evidence. Returns a
    /// list of problems (empty = OK). This is the guardrail that stops over-claiming.
    pub fn check(&self) -> Vec<String> {
        let mut problems = Vec::new();

        if self.firmware.sha256.len() != 64 || !self.firmware.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            problems.push(format!(
                "firmware.sha256 must be 64 hex chars (got {} chars)",
                self.firmware.sha256.len()
            ));
        }
        if self.device.usb_normal.vid.is_empty() || self.device.usb_normal.pid.is_empty() {
            problems.push("device.usb_normal needs both vid and pid".into());
        }

        // Confidence must be earned.
        if self.confidence >= Confidence::Verified {
            if self.result.flash_status != FlashStatus::Success {
                problems.push("confidence >= verified requires result.flash_status = success".into());
            }
            if self.result.postflash_audio != Outcome::Pass {
                problems.push("confidence >= verified requires result.postflash_audio = pass".into());
            }
            if self.evidence.normal_after_descriptor.is_none() {
                problems.push("confidence >= verified requires evidence.normal_after_descriptor".into());
            }
        }
        if self.confidence == Confidence::RestoreVerified && !self.result.restore_tested {
            problems.push("confidence = restore-verified requires result.restore_tested = true".into());
        }
        problems
    }

    /// True only when nothing regressed that a user would notice, and a flash actually succeeded.
    /// Deliberately conservative: any tested-and-failed function, or an unverified confidence,
    /// disqualifies "recommended".
    pub fn is_recommended(&self) -> bool {
        if self.confidence < Confidence::Verified || !self.check().is_empty() {
            return false;
        }
        let r = &self.result;
        r.flash_status == FlashStatus::Success
            && r.postflash_audio == Outcome::Pass
            && r.microphone != Outcome::Fail
            && r.app_control_channel != Outcome::Fail
            && r.volume_gain != Outcome::Fail
            && r.buttons != Outcome::Fail
            && r.channel_balance != Outcome::Fail
            && r.suspend_resume != Outcome::Fail
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatMatrix {
    #[serde(default)]
    pub records: Vec<CompatRecord>,
}

impl CompatMatrix {
    pub fn from_json(s: &str) -> Result<CompatMatrix, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
    /// Validate every record; returns `(index, problems)` for each record with issues.
    pub fn check_all(&self) -> Vec<(usize, Vec<String>)> {
        self.records
            .iter()
            .enumerate()
            .filter_map(|(i, r)| {
                let p = r.check();
                if p.is_empty() { None } else { Some((i, p)) }
            })
            .collect()
    }
}

/// A blank record with the fields laid out, to hand to a contributor.
pub fn template() -> CompatRecord {
    CompatRecord {
        device: DeviceInfo {
            marketing_name: "JCALLY JM12".into(),
            board_revision: None,
            usb_normal: UsbId {
                vid: "0x31b2".into(),
                pid: "0x0111".into(),
                ..Default::default()
            },
            bootloader: Some(UsbId { vid: "0x8888".into(), pid: "0xcdc0".into(), ..Default::default() }),
        },
        firmware: FirmwareRef {
            filename: Some("JadeAudio JA11_V2.2.bin".into()),
            sha256: "0".repeat(64),
            ..Default::default()
        },
        result: FunctionalResult::default(),
        evidence: Evidence::default(),
        confidence: Confidence::Candidate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> CompatRecord {
        let mut r = template();
        r.firmware.sha256 = "a".repeat(64);
        r
    }

    #[test]
    fn template_candidate_passes_basic_checks() {
        assert!(base().check().is_empty());
    }

    #[test]
    fn bad_sha_is_flagged() {
        let mut r = base();
        r.firmware.sha256 = "xyz".into();
        assert!(r.check().iter().any(|p| p.contains("sha256")));
    }

    #[test]
    fn verified_requires_success_audio_and_after_descriptor() {
        let mut r = base();
        r.confidence = Confidence::Verified;
        let p = r.check();
        assert!(p.iter().any(|x| x.contains("flash_status")));
        assert!(p.iter().any(|x| x.contains("postflash_audio")));
        assert!(p.iter().any(|x| x.contains("normal_after_descriptor")));

        r.result.flash_status = FlashStatus::Success;
        r.result.postflash_audio = Outcome::Pass;
        r.evidence.normal_after_descriptor = Some("sha".into());
        assert!(r.check().is_empty());
    }

    #[test]
    fn restore_verified_requires_restore_tested() {
        let mut r = base();
        r.confidence = Confidence::RestoreVerified;
        r.result.flash_status = FlashStatus::Success;
        r.result.postflash_audio = Outcome::Pass;
        r.evidence.normal_after_descriptor = Some("sha".into());
        assert!(r.check().iter().any(|p| p.contains("restore_tested")));
        r.result.restore_tested = true;
        assert!(r.check().is_empty());
    }

    #[test]
    fn candidate_is_never_recommended() {
        assert!(!base().is_recommended());
    }

    #[test]
    fn a_failed_function_blocks_recommendation() {
        let mut r = base();
        r.confidence = Confidence::Verified;
        r.result.flash_status = FlashStatus::Success;
        r.result.postflash_audio = Outcome::Pass;
        r.evidence.normal_after_descriptor = Some("sha".into());
        assert!(r.is_recommended());
        r.result.microphone = Outcome::Fail;
        assert!(!r.is_recommended());
        r.result.microphone = Outcome::Untested; // untested is allowed, just not failed
        assert!(r.is_recommended());
    }

    #[test]
    fn confidence_is_ordered() {
        assert!(Confidence::RestoreVerified > Confidence::Verified);
        assert!(Confidence::Verified > Confidence::Candidate);
        assert!(Confidence::Candidate > Confidence::Unsupported);
    }

    #[test]
    fn matrix_round_trips_and_checks() {
        let m = CompatMatrix { records: vec![base(), template()] };
        let back = CompatMatrix::from_json(&m.to_json_pretty()).unwrap();
        assert_eq!(m, back);
        // template()'s sha is 64 zeros (valid hex) and confidence Candidate → no problems
        assert!(back.check_all().is_empty());
    }
}
