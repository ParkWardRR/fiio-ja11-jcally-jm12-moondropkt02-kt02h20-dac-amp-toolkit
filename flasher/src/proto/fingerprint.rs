//! Structured **device fingerprint** and matching (roadmap M2 device-family gate + M6).
//!
//! The single most important guardrail in this whole project: **VID/PID alone is not proof
//! that a firmware image is safe for a given board.** Many KT02H20-based dongles share
//! `31b2:0111`; flashing a JA11 image onto a differently-wired board can brick it. So a
//! fingerprint carries more than VID/PID — `bcdDevice`, strings, and (ideally) a hash of the
//! raw descriptors — and matching reports a *strength* so callers can refuse weak matches.
//!
//! Pure data + logic; no hardware. `main.rs` builds a fingerprint from a live `probe`, but
//! the matching rules are unit-tested here without a dongle.
//!
//! ## Working-agent notes
//! - The strongest signal we can cheaply capture is `descriptor_sha256`: a hash of the
//!   active config descriptor bytes. Populate it in `main.rs` from the raw descriptor once
//!   you can read it (rusb exposes `Device::config_descriptor` raw bytes via libusb; if that
//!   is awkward, hash the reconstructed interface/endpoint table deterministically). Until
//!   it's populated, matches can only reach [`MatchStrength::VidPidOnly`].
//! - Board revision / PCB markings can't be read over USB — carry them as free-form notes in
//!   the manifest, not here.

use serde::{Deserialize, Serialize};

/// How strongly a fingerprint matched a manifest's allowed-device pattern. Ordered: a bigger
/// value is a stronger, safer match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MatchStrength {
    /// Nothing matched — different device entirely.
    NoMatch,
    /// VID/PID matched, but nothing stronger. **Not sufficient** to prove board safety.
    VidPidOnly,
    /// VID/PID plus a descriptor-hash match — the same USB personality, high confidence.
    Verified,
}

/// A concrete device identity, as observed by `probe`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceFingerprint {
    pub vid: u16,
    pub pid: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bcd_device: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    #[serde(default)]
    pub serial_present: bool,
    /// Hex SHA-256 of the raw active config descriptor, when available. The strong signal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor_sha256: Option<String>,
}

impl DeviceFingerprint {
    pub fn new(vid: u16, pid: u16) -> Self {
        DeviceFingerprint { vid, pid, ..Default::default() }
    }

    /// Compact `"vid:pid"` label, e.g. `"31b2:0111"`.
    pub fn short(&self) -> String {
        format!("{:04x}:{:04x}", self.vid, self.pid)
    }

    /// Parse a `"vid:pid"` string (hex, optional `0x`), for manual `--device` entry.
    pub fn parse_short(s: &str) -> Result<DeviceFingerprint, String> {
        let (v, p) = s.split_once(':').ok_or("expected VID:PID, e.g. 31b2:0111")?;
        let hx = |x: &str| u16::from_str_radix(x.trim().trim_start_matches("0x"), 16);
        let vid = hx(v).map_err(|e| format!("bad VID {v:?}: {e}"))?;
        let pid = hx(p).map_err(|e| format!("bad PID {p:?}: {e}"))?;
        Ok(DeviceFingerprint::new(vid, pid))
    }
}

/// A manifest's rule for "which devices may receive this image". Deliberately explicit — a
/// pattern must at minimum pin VID and PID, and may require a descriptor hash for a
/// [`MatchStrength::Verified`] match.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicePattern {
    pub vid: u16,
    pub pid: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bcd_device: Option<u16>,
    /// If set, a fingerprint must carry this exact descriptor hash to be `Verified`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor_sha256: Option<String>,
}

impl DevicePattern {
    /// Score how well `fp` matches this pattern.
    pub fn score(&self, fp: &DeviceFingerprint) -> MatchStrength {
        if self.vid != fp.vid || self.pid != fp.pid {
            return MatchStrength::NoMatch;
        }
        if let Some(want_bcd) = self.bcd_device {
            if fp.bcd_device != Some(want_bcd) {
                return MatchStrength::NoMatch;
            }
        }
        match (&self.descriptor_sha256, &fp.descriptor_sha256) {
            // Pattern demands a descriptor hash: only Verified if the device supplies a match.
            (Some(want), Some(got)) if want.eq_ignore_ascii_case(got) => MatchStrength::Verified,
            (Some(_), _) => MatchStrength::VidPidOnly,
            // Pattern doesn't demand a hash: VID/PID (+bcd) is the ceiling.
            (None, _) => MatchStrength::VidPidOnly,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_and_parse_round_trip() {
        let fp = DeviceFingerprint::new(0x31b2, 0x0111);
        assert_eq!(fp.short(), "31b2:0111");
        assert_eq!(DeviceFingerprint::parse_short("31b2:0111").unwrap(), fp);
        assert_eq!(DeviceFingerprint::parse_short("0x31B2:0x0111").unwrap(), fp);
        assert!(DeviceFingerprint::parse_short("garbage").is_err());
    }

    #[test]
    fn vid_pid_mismatch_is_no_match() {
        let pat = DevicePattern { vid: 0x31b2, pid: 0x0111, bcd_device: None, descriptor_sha256: None };
        assert_eq!(pat.score(&DeviceFingerprint::new(0x2972, 0x0102)), MatchStrength::NoMatch);
    }

    #[test]
    fn vid_pid_only_without_descriptor() {
        let pat = DevicePattern { vid: 0x31b2, pid: 0x0111, bcd_device: None, descriptor_sha256: None };
        assert_eq!(pat.score(&DeviceFingerprint::new(0x31b2, 0x0111)), MatchStrength::VidPidOnly);
    }

    #[test]
    fn descriptor_hash_upgrades_to_verified() {
        let pat = DevicePattern {
            vid: 0x31b2,
            pid: 0x0111,
            bcd_device: None,
            descriptor_sha256: Some("ABCD".into()),
        };
        let mut fp = DeviceFingerprint::new(0x31b2, 0x0111);
        assert_eq!(pat.score(&fp), MatchStrength::VidPidOnly); // no hash on device yet
        fp.descriptor_sha256 = Some("abcd".into()); // case-insensitive
        assert_eq!(pat.score(&fp), MatchStrength::Verified);
        fp.descriptor_sha256 = Some("0000".into());
        assert_eq!(pat.score(&fp), MatchStrength::VidPidOnly); // wrong hash → not verified
    }

    #[test]
    fn bcd_device_must_match_when_required() {
        let pat = DevicePattern {
            vid: 0x31b2,
            pid: 0x0111,
            bcd_device: Some(0x0100),
            descriptor_sha256: None,
        };
        let mut fp = DeviceFingerprint::new(0x31b2, 0x0111);
        assert_eq!(pat.score(&fp), MatchStrength::NoMatch);
        fp.bcd_device = Some(0x0100);
        assert_eq!(pat.score(&fp), MatchStrength::VidPidOnly);
    }

    #[test]
    fn strength_is_ordered() {
        assert!(MatchStrength::Verified > MatchStrength::VidPidOnly);
        assert!(MatchStrength::VidPidOnly > MatchStrength::NoMatch);
    }
}
