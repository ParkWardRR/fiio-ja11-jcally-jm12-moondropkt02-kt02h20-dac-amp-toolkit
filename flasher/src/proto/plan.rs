//! The M2 **image gate** and **flash plan** — the safety layer that must pass *before* any
//! destructive erase/program.
//!
//! The roadmap is explicit: `KT_Helios` magic is necessary but **not sufficient**. A
//! structurally valid image can still be wrong for a specific board. So a flash is a
//! two-stage, scriptable transaction:
//!
//! ```text
//! ktflash flash --plan firmware.bin        # emits this FlashPlan (no device touched)
//! ktflash flash --apply plan-<ts>.json     # requires that exact, unchanged plan
//! ```
//!
//! `--plan` computes the image's SHA-256, parses/validates the `KT_Helios` header, runs the
//! gate, and records a [`Decision`]. `--apply` re-reads the image, re-hashes it, and refuses
//! unless the hash still matches the plan.
//!
//! This module is pure and hardware-free (it only reasons about bytes), so the gate logic is
//! unit-tested without a dongle. The actual write it authorizes is still blocked on the CDC
//! wire codec (roadmap M1) — `--apply` says so honestly rather than pretending.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::fingerprint::DeviceFingerprint;
use super::image::{KtHelios, Level};
use super::manifest::{Manifest, RiskLevel, Verdict};

/// SHA-256 of `bytes` as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    super::transcript::to_hex(&digest)
}

/// One gate check and its result. `confidence` mirrors [`super::Confidence`] as a string so
/// the plan JSON is self-describing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
    pub confidence: String,
}

/// The gate's overall verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Decision {
    /// Image valid **and** device-family compatibility proven — safe to apply.
    Proceed,
    /// Image structurally valid, but compatibility with a specific device is unproven.
    /// Rejected by default; requires an explicit, logged override (roadmap M2).
    NeedsConfirmation,
    /// Image failed structural validation — never write it.
    Refuse,
}

impl Decision {
    pub fn label(self) -> &'static str {
        match self {
            Decision::Proceed => "PROCEED",
            Decision::NeedsConfirmation => "NEEDS CONFIRMATION (rejected by default)",
            Decision::Refuse => "REFUSE",
        }
    }
}

/// A durable, inspectable record of exactly what a flash would do. Emitted by `--plan`,
/// consumed by `--apply`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlashPlan {
    pub tool_version: String,
    pub created_epoch_secs: u64,
    pub image_path: String,
    pub image_len: usize,
    pub image_sha256: String,
    /// Parsed header summary (absent if the image didn't parse).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_header: Option<ImageSummary>,
    /// The device this plan is intended for (from `probe` or `--device`), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceFingerprint>,
    pub gate: Vec<GateCheck>,
    pub decision: Decision,
}

/// Serializable subset of [`KtHelios`] for the plan record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageSummary {
    pub magic: String,
    pub chip: String,
    pub declared_size: u32,
    pub build_hash: String,
    pub build_date: String,
    pub enty_load_addr: u32,
    pub enty_size: u32,
}

impl From<&KtHelios> for ImageSummary {
    fn from(h: &KtHelios) -> Self {
        ImageSummary {
            magic: h.magic.clone(),
            chip: h.chip.clone(),
            declared_size: h.declared_size,
            build_hash: h.build_hash.clone(),
            build_date: h.build_date.clone(),
            enty_load_addr: h.enty_load_addr,
            enty_size: h.enty_size,
        }
    }
}

impl FlashPlan {
    /// Build a plan for `bytes` (read from `image_path`). `device` is an optional identity
    /// string the plan is intended for. Pure: no I/O, no hardware.
    pub fn build(
        image_path: &str,
        bytes: &[u8],
        tool_version: &str,
        created_epoch_secs: u64,
        device: Option<DeviceFingerprint>,
        manifest: Option<&Manifest>,
    ) -> FlashPlan {
        let mut gate = Vec::new();
        let sha = sha256_hex(bytes);

        let parsed = KtHelios::parse(bytes);
        let mut has_error = false;
        let header = match &parsed {
            Ok(h) => {
                gate.push(GateCheck {
                    name: "kt-helios-magic".into(),
                    passed: true,
                    detail: format!("magic \"{}\", chip \"{}\"", h.magic, h.chip),
                    confidence: "static-inferred".into(),
                });
                for (level, msg) in h.findings(bytes.len()) {
                    let passed = level != Level::Error;
                    has_error |= !passed;
                    gate.push(GateCheck {
                        name: "image-structure".into(),
                        passed,
                        detail: msg,
                        confidence: "static-inferred".into(),
                    });
                }
                Some(ImageSummary::from(h))
            }
            Err(e) => {
                has_error = true;
                gate.push(GateCheck {
                    name: "kt-helios-magic".into(),
                    passed: false,
                    detail: e.to_string(),
                    confidence: "static-inferred".into(),
                });
                None
            }
        };

        // Device-family compatibility — evaluated against the firmware manifest, and it
        // **fails closed**: no manifest, unknown image, missing device, or a too-weak match
        // all leave this unproven, so a structurally valid image lands on NeedsConfirmation
        // (reject-by-default) rather than Proceed. Only a strong-enough, non-Unsafe manifest
        // hit proves it. VID/PID alone is never sufficient on its own (roadmap M2).
        let (device_family_proven, detail, confidence) = match manifest {
            None => (
                false,
                "no firmware manifest supplied; board-level compatibility is unproven \
                 (VID/PID alone is not sufficient)"
                    .to_string(),
                "hypothetical",
            ),
            Some(m) => match m.evaluate(&sha, device.as_ref()) {
                Verdict::Proven { strength, risk } => (
                    risk != RiskLevel::Unsafe,
                    format!("manifest match ({strength:?}); risk {risk:?}"),
                    "static-inferred",
                ),
                Verdict::Insufficient { got, need, risk, reason } => (
                    false,
                    format!("insufficient match: {reason} (got {got:?}, need {need:?}, risk {risk:?})"),
                    "static-inferred",
                ),
                Verdict::UnknownImage => {
                    (false, "image SHA-256 is not in the manifest".to_string(), "static-inferred")
                }
                Verdict::NoDevice => (
                    false,
                    "no device fingerprint supplied; cannot evaluate compatibility \
                     (use --device VID:PID or run with the dongle attached)"
                        .to_string(),
                    "hypothetical",
                ),
            },
        };
        gate.push(GateCheck {
            name: "device-family-match".into(),
            passed: device_family_proven,
            detail,
            confidence: confidence.into(),
        });

        let decision = if has_error {
            Decision::Refuse
        } else if device_family_proven {
            Decision::Proceed
        } else {
            Decision::NeedsConfirmation
        };

        FlashPlan {
            tool_version: tool_version.to_string(),
            created_epoch_secs,
            image_path: image_path.to_string(),
            image_len: bytes.len(),
            image_sha256: sha,
            image_header: header,
            device,
            gate,
            decision,
        }
    }

    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> Result<FlashPlan, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }

    /// Re-validate `bytes` (freshly read for `--apply`) against this plan. Errors if the
    /// image identity has changed since the plan was produced.
    pub fn reverify_image(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() != self.image_len {
            return Err(format!(
                "image length changed since plan: plan {} bytes, now {} bytes",
                self.image_len,
                bytes.len()
            ));
        }
        let sha = sha256_hex(bytes);
        if sha != self.image_sha256 {
            return Err(format!(
                "image SHA-256 changed since plan:\n  plan {}\n  now  {}",
                self.image_sha256, sha
            ));
        }
        Ok(())
    }
}

/// Best-effort wall-clock seconds since the Unix epoch (0 if the clock is before 1970).
pub fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth(total: usize) -> Vec<u8> {
        let mut b = vec![0u8; total.max(0x4C)];
        b[0x00..0x0D].copy_from_slice(b"KT_Helios_v1b");
        b[0x10..0x18].copy_from_slice(b"KT02H20B");
        b[0x18..0x1C].copy_from_slice(b"Size");
        b[0x1C..0x20].copy_from_slice(&(total as u32).to_le_bytes());
        b[0x40..0x44].copy_from_slice(b"ENTY");
        b[0x44..0x48].copy_from_slice(&0x0008_3000u32.to_le_bytes());
        b[0x48..0x4C].copy_from_slice(&0x10u32.to_le_bytes());
        b
    }

    #[test]
    fn sha256_matches_known_vector() {
        // SHA-256("abc")
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn valid_image_needs_confirmation_not_proceed() {
        let img = synth(0x4C);
        let plan = FlashPlan::build("fw.bin", &img, "test", 42, None, None);
        assert_eq!(plan.decision, Decision::NeedsConfirmation);
        assert_eq!(plan.image_len, img.len());
        assert_eq!(plan.image_sha256.len(), 64);
        assert!(plan.image_header.is_some());
        // the device-family check must be present and failing
        assert!(plan.gate.iter().any(|g| g.name == "device-family-match" && !g.passed));
    }

    #[test]
    fn structural_error_refuses() {
        let mut img = synth(1024);
        // declare a wrong size to force an error finding
        img[0x1C..0x20].copy_from_slice(&9999u32.to_le_bytes());
        let plan = FlashPlan::build("fw.bin", &img, "test", 42, None, None);
        assert_eq!(plan.decision, Decision::Refuse);
    }

    #[test]
    fn foreign_blob_refuses() {
        let plan = FlashPlan::build("x.bin", &[0u8; 8], "test", 42, None, None);
        assert_eq!(plan.decision, Decision::Refuse);
        assert!(plan.image_header.is_none());
    }

    #[test]
    fn plan_round_trips_json() {
        let img = synth(0x4C);
        let dev = super::super::fingerprint::DeviceFingerprint::parse_short("31b2:0111").unwrap();
        let plan = FlashPlan::build("fw.bin", &img, "test", 42, Some(dev), None);
        let back = FlashPlan::from_json(&plan.to_json_pretty()).unwrap();
        assert_eq!(plan, back);
    }

    #[test]
    fn manifest_match_proves_and_proceeds() {
        use super::super::fingerprint::{DeviceFingerprint, DevicePattern, MatchStrength};
        use super::super::manifest::{Manifest, ManifestEntry, RiskLevel};
        let img = synth(0x4C);
        let sha = sha256_hex(&img);
        let dev = DeviceFingerprint::parse_short("31b2:0111").unwrap();
        let manifest = Manifest {
            entries: vec![ManifestEntry {
                image_sha256: sha,
                version: Some("v1".into()),
                source_url: None,
                redistribution: None,
                allowed_devices: vec![DevicePattern {
                    vid: 0x31b2,
                    pid: 0x0111,
                    bcd_device: None,
                    descriptor_sha256: None,
                }],
                min_match: MatchStrength::VidPidOnly,
                risk_level: RiskLevel::Verified,
                notes: None,
            }],
        };
        let plan = FlashPlan::build("fw.bin", &img, "test", 42, Some(dev), Some(&manifest));
        assert_eq!(plan.decision, Decision::Proceed);

        // Same manifest but a different device → back to NeedsConfirmation (fails closed).
        let other = DeviceFingerprint::parse_short("2972:0102").unwrap();
        let plan2 = FlashPlan::build("fw.bin", &img, "test", 42, Some(other), Some(&manifest));
        assert_eq!(plan2.decision, Decision::NeedsConfirmation);
    }

    #[test]
    fn reverify_detects_changed_image() {
        let img = synth(0x4C);
        let plan = FlashPlan::build("fw.bin", &img, "test", 42, None, None);
        assert!(plan.reverify_image(&img).is_ok());
        let mut tampered = img.clone();
        tampered[0x20] ^= 0xFF;
        assert!(plan.reverify_image(&tampered).is_err());
        assert!(plan.reverify_image(&img[..img.len() - 1]).is_err());
    }
}
