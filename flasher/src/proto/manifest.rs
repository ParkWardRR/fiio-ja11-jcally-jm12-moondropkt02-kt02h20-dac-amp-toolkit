//! The **firmware manifest** (roadmap M2 image gate + firmware provenance/catalog).
//!
//! A structurally valid `KT_Helios` image is *not* automatically safe to write to a given
//! device. The manifest is the allow-list that turns "this parses" into "this specific image
//! is known-good for this specific device family". It maps an image SHA-256 to:
//!   - provenance (source URL, version, redistribution status),
//!   - the device patterns it may be written to, and
//!   - the **minimum match strength** required and a **risk level**.
//!
//! Guardrail philosophy: **fail closed.** No manifest, unknown image, or a too-weak device
//! match all yield "not proven" — the gate then refuses-by-default (needs an explicit,
//! logged override). Only a manifest hit whose device match meets `min_match` proves safety.
//!
//! Pure data + logic, unit-tested without hardware.
//!
//! ## Working-agent notes
//! - Ship a curated `manifest.json` in-repo as evidence accrues (roadmap M6). Do **not** add
//!   an entry until you have a real hash + provenance + at least one restore-verified device.
//! - Prefer `min_match: verified` (descriptor-hash) for cross-flashes; `vid-pid-only` is only
//!   acceptable for same-model reflashes where the personality can't change.
//! - Keep redistribution honest: if a blob can't be redistributed, store the hash + source
//!   URL, not the bytes.

use serde::{Deserialize, Serialize};

use super::fingerprint::{DeviceFingerprint, DevicePattern, MatchStrength};

/// How dangerous flashing this image is understood to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RiskLevel {
    /// Restore-verified on this device family: read → flash → restore cycle proven.
    RestoreVerified,
    /// Flash + core function verified, but restore not proven.
    Verified,
    /// Flashed successfully somewhere, limited evidence.
    Experimental,
    /// Known to misbehave/brick on some boards — never auto-proceed.
    Unsafe,
}

/// One catalogued firmware image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Hex SHA-256 of the image bytes — the primary key.
    pub image_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redistribution: Option<String>,
    /// Devices this image may be written to.
    pub allowed_devices: Vec<DevicePattern>,
    /// Minimum match strength required to consider a device compatible.
    #[serde(default = "default_min_match")]
    pub min_match: MatchStrength,
    pub risk_level: RiskLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

fn default_min_match() -> MatchStrength {
    MatchStrength::Verified
}

/// The verdict of checking an image+device pair against the manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Image + device meet the entry's bar; safe to proceed. Carries the achieved strength.
    Proven { strength: MatchStrength, risk: RiskLevel },
    /// Image is catalogued but the device match is too weak or the risk is `Unsafe`.
    Insufficient { got: MatchStrength, need: MatchStrength, risk: RiskLevel, reason: String },
    /// This image hash isn't in the manifest at all.
    UnknownImage,
    /// No fingerprint was supplied, so device compatibility can't be evaluated.
    NoDevice,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub entries: Vec<ManifestEntry>,
}

impl Manifest {
    pub fn from_json(s: &str) -> Result<Manifest, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }

    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Find the entry for an image hash (case-insensitive).
    pub fn entry_for(&self, image_sha256: &str) -> Option<&ManifestEntry> {
        self.entries.iter().find(|e| e.image_sha256.eq_ignore_ascii_case(image_sha256))
    }

    /// Evaluate an image+device pair. **Fails closed** — anything short of a strong-enough,
    /// non-`Unsafe` match is not `Proven`.
    pub fn evaluate(&self, image_sha256: &str, device: Option<&DeviceFingerprint>) -> Verdict {
        let entry = match self.entry_for(image_sha256) {
            Some(e) => e,
            None => return Verdict::UnknownImage,
        };
        let fp = match device {
            Some(fp) => fp,
            None => return Verdict::NoDevice,
        };
        let got = entry
            .allowed_devices
            .iter()
            .map(|p| p.score(fp))
            .max()
            .unwrap_or(MatchStrength::NoMatch);

        if entry.risk_level == RiskLevel::Unsafe {
            return Verdict::Insufficient {
                got,
                need: entry.min_match,
                risk: entry.risk_level,
                reason: "manifest marks this image as unsafe".into(),
            };
        }
        if got >= entry.min_match {
            Verdict::Proven { strength: got, risk: entry.risk_level }
        } else {
            Verdict::Insufficient {
                got,
                need: entry.min_match,
                risk: entry.risk_level,
                reason: format!("device match {got:?} is weaker than required {:?}", entry.min_match),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(min: MatchStrength, risk: RiskLevel, pat: DevicePattern) -> ManifestEntry {
        ManifestEntry {
            image_sha256: "aa".into(),
            version: None,
            source_url: None,
            redistribution: None,
            allowed_devices: vec![pat],
            min_match: min,
            risk_level: risk,
            notes: None,
        }
    }

    fn pat_vidpid() -> DevicePattern {
        DevicePattern { vid: 0x31b2, pid: 0x0111, bcd_device: None, descriptor_sha256: None }
    }

    #[test]
    fn unknown_image_and_no_device() {
        let m = Manifest { entries: vec![entry(MatchStrength::VidPidOnly, RiskLevel::Verified, pat_vidpid())] };
        assert_eq!(m.evaluate("ff", Some(&DeviceFingerprint::new(0x31b2, 0x0111))), Verdict::UnknownImage);
        assert_eq!(m.evaluate("aa", None), Verdict::NoDevice);
    }

    #[test]
    fn weak_match_is_insufficient() {
        // requires Verified but device only offers VID/PID
        let m = Manifest { entries: vec![entry(MatchStrength::Verified, RiskLevel::Verified, pat_vidpid())] };
        let v = m.evaluate("aa", Some(&DeviceFingerprint::new(0x31b2, 0x0111)));
        assert!(matches!(v, Verdict::Insufficient { got: MatchStrength::VidPidOnly, need: MatchStrength::Verified, .. }));
    }

    #[test]
    fn strong_enough_match_proves() {
        let m = Manifest { entries: vec![entry(MatchStrength::VidPidOnly, RiskLevel::Verified, pat_vidpid())] };
        let v = m.evaluate("AA", Some(&DeviceFingerprint::new(0x31b2, 0x0111)));
        assert_eq!(v, Verdict::Proven { strength: MatchStrength::VidPidOnly, risk: RiskLevel::Verified });
    }

    #[test]
    fn unsafe_never_proves_even_on_strong_match() {
        let pat = DevicePattern {
            vid: 0x31b2,
            pid: 0x0111,
            bcd_device: None,
            descriptor_sha256: Some("dd".into()),
        };
        let m = Manifest { entries: vec![entry(MatchStrength::Verified, RiskLevel::Unsafe, pat)] };
        let mut fp = DeviceFingerprint::new(0x31b2, 0x0111);
        fp.descriptor_sha256 = Some("dd".into());
        assert!(matches!(m.evaluate("aa", Some(&fp)), Verdict::Insufficient { .. }));
    }

    #[test]
    fn manifest_json_round_trips() {
        let m = Manifest { entries: vec![entry(MatchStrength::Verified, RiskLevel::RestoreVerified, pat_vidpid())] };
        assert_eq!(Manifest::from_json(&m.to_json_pretty()).unwrap(), m);
    }

    #[test]
    fn min_match_defaults_to_verified() {
        // an entry JSON without min_match should default to the safe (Verified) bar
        let json = r#"{"entries":[{"image_sha256":"aa","allowed_devices":[{"vid":12722,"pid":273}],"risk_level":"verified"}]}"#;
        let m = Manifest::from_json(json).unwrap();
        assert_eq!(m.entries[0].min_match, MatchStrength::Verified);
    }
}
