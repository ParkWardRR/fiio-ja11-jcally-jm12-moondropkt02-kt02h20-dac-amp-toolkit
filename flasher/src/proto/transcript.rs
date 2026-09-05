//! The M0 **normalized capture transcript** — the in-repo, review-friendly, regression-
//! testable representation of a bootloader USB capture.
//!
//! Raw `.pcapng` is the ground truth but is awkward to diff, review, and test against. A
//! transcript is a small JSON document: an ordered list of frames, each with a direction, a
//! best-guess stage, the raw bytes (hex), and an optional decode. The decoder
//! [`Transcript::from_tshark`] turns the exact `tshark` invocation in the roadmap/README
//! into this shape.
//!
//! This module is pure data + serde; it never touches hardware, so `bootdiag --replay` and
//! CI can validate fixtures anywhere.

use serde::{Deserialize, Serialize};

/// Which way a frame travelled on the bootloader's CDC endpoints
/// (`0x03` OUT from host, `0x83` IN from device).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Out,
    In,
}

impl Direction {
    /// Map a USB endpoint address to a direction (bit 7 set == IN).
    pub fn from_endpoint(ep: u8) -> Direction {
        if ep & 0x80 != 0 {
            Direction::In
        } else {
            Direction::Out
        }
    }

    pub fn arrow(self) -> &'static str {
        match self {
            Direction::Out => "OUT",
            Direction::In => "IN ",
        }
    }
}

/// The vendor state-machine phase a frame belongs to, if known. Mirrors the named states
/// scraped from the vendor tool (`Shake hand` → `Erase` → `Program` → success).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Handshake,
    Erase,
    Program,
    Verify,
    Reset,
    /// Unclassified — the default, and the fallback for unrecognized JSON values.
    #[default]
    #[serde(other)]
    Unknown,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Stage::Handshake => "handshake",
            Stage::Erase => "erase",
            Stage::Program => "program",
            Stage::Verify => "verify",
            Stage::Reset => "reset",
            Stage::Unknown => "unknown",
        }
    }
}

/// A provisional decode of a frame's payload. All fields optional — fill in what a capture
/// actually proves, and mark the rest absent rather than guessed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decoded {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_length: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

/// One captured frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub direction: Direction,
    #[serde(default)]
    pub stage: Stage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_ms: Option<u64>,
    /// Raw bytes as lowercase hex (no separators required on read).
    pub bytes_hex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded: Option<Decoded>,
}

impl Frame {
    pub fn new(direction: Direction, bytes: &[u8]) -> Frame {
        Frame {
            direction,
            stage: Stage::Unknown,
            timestamp_ms: None,
            bytes_hex: to_hex(bytes),
            decoded: None,
        }
    }

    /// Decode this frame's hex back into raw bytes.
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        from_hex(&self.bytes_hex)
    }
}

/// A whole capture: metadata plus an ordered list of frames.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    /// Where this came from (capture filename, "synthetic", tool + version, …).
    pub source: String,
    #[serde(default)]
    pub note: String,
    /// Free-form device identity (marketing name / VID:PID / fingerprint).
    #[serde(default)]
    pub device: String,
    pub frames: Vec<Frame>,
}

impl Transcript {
    pub fn from_json(s: &str) -> Result<Transcript, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }

    pub fn to_json_pretty(&self) -> String {
        // to_string_pretty only fails on non-serializable types, which ours never are.
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Basic structural validation with no hardware: every frame's hex must decode.
    /// Returns the total number of bytes across all frames.
    pub fn validate(&self) -> Result<usize, String> {
        let mut total = 0;
        for (i, f) in self.frames.iter().enumerate() {
            let b = f.bytes().map_err(|e| format!("frame {i}: bad hex: {e}"))?;
            total += b.len();
        }
        Ok(total)
    }

    /// Build a transcript from the output of the documented tshark invocation:
    ///
    /// ```text
    /// tshark -r cap.pcapng -Y "usb.transfer_type==0x03 && usb.endpoint_address in {0x03,0x83}" \
    ///        -T fields -e usb.endpoint_address -e usb.capdata
    /// ```
    ///
    /// Each non-empty line is `<endpoint>\t<capdata-hex>`. Endpoint and capdata may be
    /// rendered by tshark with `0x` prefixes and `:`-separated hex; both are tolerated.
    pub fn from_tshark(text: &str, source: &str) -> Transcript {
        let mut frames = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut cols = line.split('\t');
            let ep_field = cols.next().unwrap_or("").trim();
            let data_field = cols.next().unwrap_or("").trim();
            if data_field.is_empty() {
                continue;
            }
            let ep = parse_u8_maybe_hex(ep_field).unwrap_or(0);
            let bytes = match from_hex(data_field) {
                Ok(b) if !b.is_empty() => b,
                _ => continue,
            };
            frames.push(Frame::new(Direction::from_endpoint(ep), &bytes));
        }
        Transcript {
            source: source.to_string(),
            note: "decoded from tshark fields; stages/decodes not yet classified".to_string(),
            device: String::new(),
            frames,
        }
    }
}

/// Lowercase hex, no separators.
pub fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Parse hex, tolerating whitespace, `:` and `,` separators and an optional `0x` prefix.
pub fn from_hex(s: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = s
        .trim()
        .trim_start_matches("0x")
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':' && *c != ',' && *c != '_')
        .collect();
    if !cleaned.len().is_multiple_of(2) {
        return Err(format!("odd number of hex digits ({})", cleaned.len()));
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

fn parse_u8_maybe_hex(s: &str) -> Option<u8> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u8::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<u8>().ok().or_else(|| u8::from_str_radix(s, 16).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let bytes = [0x00u8, 0x03, 0x83, 0xff, 0x10];
        assert_eq!(to_hex(&bytes), "000383ff10");
        assert_eq!(from_hex("00:03:83:ff:10").unwrap(), bytes);
        assert_eq!(from_hex("0x000383ff10").unwrap(), bytes);
        assert_eq!(from_hex(" 00 03 83 ff 10 ").unwrap(), bytes);
    }

    #[test]
    fn odd_hex_is_rejected() {
        assert!(from_hex("abc").is_err());
    }

    #[test]
    fn direction_from_endpoint() {
        assert_eq!(Direction::from_endpoint(0x03), Direction::Out);
        assert_eq!(Direction::from_endpoint(0x83), Direction::In);
    }

    #[test]
    fn tshark_lines_become_frames() {
        let text = "0x03\t01\n0x83\t06\n\n0x03\t02:00:10:00:00:00\n";
        let t = Transcript::from_tshark(text, "cap.pcapng");
        assert_eq!(t.frames.len(), 3);
        assert_eq!(t.frames[0].direction, Direction::Out);
        assert_eq!(t.frames[1].direction, Direction::In);
        assert_eq!(t.frames[2].bytes().unwrap(), vec![0x02, 0x00, 0x10, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn json_round_trips_and_validates() {
        let t = Transcript {
            source: "synthetic".into(),
            note: "".into(),
            device: "JA11 2972:0102".into(),
            frames: vec![Frame::new(Direction::Out, &[0x01]), Frame::new(Direction::In, &[0x06])],
        };
        let json = t.to_json_pretty();
        let back = Transcript::from_json(&json).unwrap();
        assert_eq!(t, back);
        assert_eq!(back.validate().unwrap(), 2);
    }

    #[test]
    fn stage_defaults_and_unknown_parse() {
        // missing stage -> default unknown; unrecognized string -> unknown
        let f: Frame = serde_json::from_str(r#"{"direction":"out","bytes_hex":"01"}"#).unwrap();
        assert_eq!(f.stage, Stage::Unknown);
        let f2: Frame =
            serde_json::from_str(r#"{"direction":"in","stage":"frobnicate","bytes_hex":"06"}"#).unwrap();
        assert_eq!(f2.stage, Stage::Unknown);
    }
}
