//! Post-reset reprobe: deciding whether a flash actually *worked*.
//!
//! ## Why this exists
//!
//! ROADMAP Phase 3.4 — *"journal each stage before its destructive command **and confirm
//! success by a post-reset reprobe**"*. The first half landed with
//! [`crate::proto::ktcdc_journal`]; this is the second.
//!
//! Without it, a journal tops out at [`Stage::ResetIssued`](crate::proto::journal::Stage), whose
//! recovery action is `WaitAndReprobe` — i.e. "we sent RESET and then stopped paying attention".
//! `ktflash recover` can never say *done*, and a flash that reset into a non-booting image looks
//! exactly like one that worked.
//!
//! ## Deliberately conservative
//!
//! Two rules that matter more than they look:
//!
//! 1. **An ISP-mode device anywhere on the bus means "not confirmed."** If `8888:cdc0` is still
//!    present we report [`ReprobeOutcome::StillInBootloader`] even if some other runtime dongle
//!    is also attached. Claiming success while a device sits in the bootloader is the one wrong
//!    answer here: it tells the operator to walk away from a dongle that needs reflashing.
//! 2. **Identity mismatch is only ever reported against an explicit expectation.**
//!    [`Stage::IdentityMismatch`](crate::proto::journal::Stage) is a *halt* state that tells a
//!    human to preserve evidence and stop. Inferring an expected VID:PID and then halting on it
//!    would manufacture alarm from a guess, so a mismatch requires the caller to have said what
//!    it expected (`--expect`).
//!
//! This module is transport- and hardware-free: callers observe the bus and hand the result to
//! [`decide`], so the policy is unit-testable and the I/O stays in `main.rs`.

use crate::proto::fingerprint::DeviceFingerprint;

/// What a caller saw on the bus after issuing `RESET`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ReprobeObservation {
    /// Is any `8888:cdc0` device still enumerated?
    pub bootloader_present: bool,
    /// The first non-bootloader dongle found, if any.
    pub runtime: Option<DeviceFingerprint>,
}

/// The verdict on a completed flash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReprobeOutcome {
    /// The dongle came back in normal mode and matched expectations (or none were given).
    Confirmed(DeviceFingerprint),
    /// It came back as something other than what the caller said to expect. **Halt.**
    Mismatch { found: DeviceFingerprint, expected: DeviceFingerprint },
    /// A device is still in ISP mode — the reset did not take, or the image does not boot.
    StillInBootloader,
    /// Nothing recognisable enumerated within the timeout.
    NoDevice,
}

impl ReprobeOutcome {
    /// Did the flash demonstrably succeed?
    pub fn is_success(&self) -> bool {
        matches!(self, ReprobeOutcome::Confirmed(_))
    }

    /// Should the operator stop and preserve evidence rather than retry?
    pub fn is_halt(&self) -> bool {
        matches!(self, ReprobeOutcome::Mismatch { .. })
    }

    /// One line for the journal's `detail` field.
    pub fn journal_detail(&self) -> String {
        match self {
            ReprobeOutcome::Confirmed(fp) => {
                format!("post-reset reprobe saw {}", fp.short())
            }
            ReprobeOutcome::Mismatch { found, expected } => format!(
                "post-reset identity {} does not match the expected {}",
                found.short(),
                expected.short()
            ),
            ReprobeOutcome::StillInBootloader => {
                "device is still in the 8888:cdc0 bootloader after RESET".to_string()
            }
            ReprobeOutcome::NoDevice => {
                "no recognised device re-enumerated after RESET".to_string()
            }
        }
    }

    /// What the operator should do next, in plain words.
    pub fn advice(&self) -> &'static str {
        match self {
            ReprobeOutcome::Confirmed(_) => {
                "Flash confirmed. Play audio through the dongle to check it end-to-end."
            }
            ReprobeOutcome::Mismatch { .. } => {
                "STOP. The device came back as something else. Preserve the journal and \
                 descriptors; do not flash another image on a guess."
            }
            ReprobeOutcome::StillInBootloader => {
                "The image likely does not boot. Unplug/replug; if it still enumerates as \
                 8888:cdc0, re-unlock and reflash a known-good image."
            }
            ReprobeOutcome::NoDevice => {
                "Unplug and replug the dongle, then run `ktflash probe`. If nothing appears, \
                 re-unlock and reflash a known-good image."
            }
        }
    }
}

/// Turn an observation into a verdict. Pure — see the module docs for the two rules.
pub fn decide(
    obs: &ReprobeObservation,
    expected: Option<&DeviceFingerprint>,
) -> ReprobeOutcome {
    // Rule 1: an ISP-mode device on the bus outranks anything else we might have seen.
    if obs.bootloader_present {
        return ReprobeOutcome::StillInBootloader;
    }
    let Some(found) = obs.runtime.clone() else {
        return ReprobeOutcome::NoDevice;
    };
    // Rule 2: only ever mismatch against an expectation the caller actually stated.
    if let Some(want) = expected {
        if found.vid != want.vid || found.pid != want.pid {
            return ReprobeOutcome::Mismatch { found, expected: want.clone() };
        }
    }
    ReprobeOutcome::Confirmed(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(vid: u16, pid: u16) -> DeviceFingerprint {
        DeviceFingerprint::new(vid, pid)
    }

    fn saw(runtime: Option<DeviceFingerprint>) -> ReprobeObservation {
        ReprobeObservation { bootloader_present: false, runtime }
    }

    #[test]
    fn a_dongle_back_in_normal_mode_confirms_when_nothing_was_expected() {
        let obs = saw(Some(fp(0x2972, 0x0102)));
        assert_eq!(decide(&obs, None), ReprobeOutcome::Confirmed(fp(0x2972, 0x0102)));
        assert!(decide(&obs, None).is_success());
    }

    #[test]
    fn a_matching_expectation_confirms() {
        let want = fp(0x2972, 0x0102);
        let obs = saw(Some(fp(0x2972, 0x0102)));
        assert!(decide(&obs, Some(&want)).is_success());
    }

    #[test]
    fn a_wrong_identity_halts_instead_of_retrying() {
        // The safety model refuses to pick another image automatically when the device is not
        // what we expected — a human has to look.
        let want = fp(0x2972, 0x0102);
        let obs = saw(Some(fp(0x31b2, 0x0111)));
        let out = decide(&obs, Some(&want));
        assert!(out.is_halt());
        assert!(!out.is_success());
        assert!(out.advice().contains("STOP"));
        match out {
            ReprobeOutcome::Mismatch { found, expected } => {
                assert_eq!(found.short(), "31b2:0111");
                assert_eq!(expected.short(), "2972:0102");
            }
            other => panic!("expected Mismatch, got {other:?}"),
        }
    }

    #[test]
    fn an_unexpected_identity_is_not_a_halt_when_nothing_was_expected() {
        // Manufacturing a halt from an inferred expectation would turn a guess into an alarm.
        let obs = saw(Some(fp(0x31b2, 0x0111)));
        let out = decide(&obs, None);
        assert!(!out.is_halt());
        assert!(out.is_success());
    }

    #[test]
    fn a_device_still_in_isp_mode_is_never_a_success() {
        let obs = ReprobeObservation { bootloader_present: true, runtime: None };
        assert_eq!(decide(&obs, None), ReprobeOutcome::StillInBootloader);
        assert!(!decide(&obs, None).is_success());
    }

    #[test]
    fn a_bootloader_outranks_a_healthy_looking_runtime_device() {
        // Two dongles attached, one of them stuck in ISP mode. Reporting success here would
        // tell the operator to walk away from a device that still needs reflashing.
        let obs = ReprobeObservation {
            bootloader_present: true,
            runtime: Some(fp(0x2972, 0x0102)),
        };
        let want = fp(0x2972, 0x0102);
        assert_eq!(decide(&obs, Some(&want)), ReprobeOutcome::StillInBootloader);
    }

    #[test]
    fn nothing_on_the_bus_is_reported_distinctly_from_a_stuck_bootloader() {
        // These need different advice: "replug and probe" vs "the image does not boot".
        let empty = decide(&saw(None), None);
        let stuck = decide(&ReprobeObservation { bootloader_present: true, runtime: None }, None);
        assert_eq!(empty, ReprobeOutcome::NoDevice);
        assert_ne!(empty.advice(), stuck.advice());
        assert!(stuck.advice().contains("does not boot"));
    }

    #[test]
    fn every_outcome_has_a_journal_detail_and_advice() {
        let outcomes = [
            ReprobeOutcome::Confirmed(fp(0x2972, 0x0102)),
            ReprobeOutcome::Mismatch { found: fp(1, 2), expected: fp(3, 4) },
            ReprobeOutcome::StillInBootloader,
            ReprobeOutcome::NoDevice,
        ];
        for o in outcomes {
            assert!(!o.journal_detail().is_empty(), "{o:?} has no journal detail");
            assert!(!o.advice().is_empty(), "{o:?} has no advice");
        }
    }

    #[test]
    fn the_confirmed_detail_names_the_device_that_came_back() {
        let d = ReprobeOutcome::Confirmed(fp(0x2972, 0x0102)).journal_detail();
        assert!(d.contains("2972:0102"), "{d}");
    }
}
