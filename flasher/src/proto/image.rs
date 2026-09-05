//! The `KT_Helios` firmware container (roadmap M2 image gate — the *parsing* half).
//!
//! Layout recovered by static analysis of `JadeAudio JA11_V2.2.bin` — see
//! `docs/PROTOCOL.md` and `docs/EXTRACTION.md`. The header is plaintext (entropy ≈ 6.5,
//! not encrypted/compressed):
//!
//! ```text
//!  0x00  magic   "KT_Helios_v1b"  (padded to 0x10)
//!  0x10  chip    "KT02H20B"       (8 bytes, NUL-padded)
//!  0x18  "Size"  literal          then u32-LE total length @ 0x1C
//!  0x20  build git hash           (ASCII, NUL-terminated, up to 0x30)
//!  0x30  build date / version     (ASCII, NUL-terminated, up to 0x40)
//!  0x40  "ENTY"  literal          then u32-LE load addr @ 0x44, u32-LE size @ 0x48
//! ```
//!
//! **Confidence:** the field *offsets* are `StaticallyInferred` from one image; treat the
//! parser as a strong default and confirm against a second real image (roadmap M0 "image
//! B") before trusting `declared_size`/ENTY for a write decision.
//!
//! This is intentionally read-only and allocation-light: it never mutates the buffer and
//! only validates structure. Cryptographic identity (SHA-256) and the manifest match are
//! layered on top in M2; this module gives them a trustworthy parse to build on.

/// The smallest buffer that can contain a full header (through the ENTY size field).
pub const HEADER_LEN: usize = 0x4C;

const MAGIC_PREFIX: &[u8] = b"KT_Helios";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KtHelios {
    /// The full magic string as stored (e.g. `"KT_Helios_v1b"`).
    pub magic: String,
    /// Target chip string, e.g. `"KT02H20B"`.
    pub chip: String,
    /// Length the header claims the whole image is, in bytes (`u32` @ `0x1C`).
    pub declared_size: u32,
    /// Build git hash string (may be empty).
    pub build_hash: String,
    /// Build date / version string (may be empty).
    pub build_date: String,
    /// ENTY load address (`u32` @ `0x44`).
    pub enty_load_addr: u32,
    /// ENTY payload size (`u32` @ `0x48`).
    pub enty_size: u32,
    /// Whether the literal `"Size"` label was present at `0x18`.
    pub size_label_ok: bool,
    /// Whether the literal `"ENTY"` label was present at `0x40`.
    pub enty_label_ok: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Buffer is shorter than a `KT_Helios` header.
    TooShort { len: usize, need: usize },
    /// Magic bytes at offset 0 are not `KT_Helios`.
    BadMagic,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::TooShort { len, need } => {
                write!(f, "file is {len} bytes; a KT_Helios header needs at least {need}")
            }
            ImageError::BadMagic => write!(
                f,
                "not a KT_Helios image (magic at offset 0 is not \"KT_Helios\") — refusing to treat as firmware"
            ),
        }
    }
}

impl std::error::Error for ImageError {}

/// Severity of a [`KtHelios::findings`] entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Error,
    Warn,
    Info,
}

impl Level {
    pub fn tag(self) -> &'static str {
        match self {
            Level::Error => "✗",
            Level::Warn => "⚠",
            Level::Info => "ℹ",
        }
    }
}

/// Cheap sniff without a full parse — useful before committing to treat bytes as firmware.
pub fn looks_like_kt_helios(bytes: &[u8]) -> bool {
    bytes.len() >= MAGIC_PREFIX.len() && &bytes[..MAGIC_PREFIX.len()] == MAGIC_PREFIX
}

fn read_u32_le(bytes: &[u8], off: usize) -> u32 {
    // callers only reach this after the length check in `parse`.
    u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

/// Read a fixed-width field as text: stop at the first NUL, then trim ASCII padding
/// (spaces and the underscore padding the magic field uses).
fn read_cstr(bytes: &[u8], range: std::ops::Range<usize>) -> String {
    let slice = &bytes[range];
    let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..end])
        .trim_matches(|c: char| c == ' ' || c == '_')
        .to_string()
}

impl KtHelios {
    /// Parse the header. Hard errors only for the two things that make the buffer
    /// definitely-not-firmware (too short / bad magic); everything softer is reported by
    /// [`findings`](Self::findings) so a caller can decide how strict to be.
    pub fn parse(bytes: &[u8]) -> Result<KtHelios, ImageError> {
        if bytes.len() < HEADER_LEN {
            return Err(ImageError::TooShort { len: bytes.len(), need: HEADER_LEN });
        }
        if !looks_like_kt_helios(bytes) {
            return Err(ImageError::BadMagic);
        }
        Ok(KtHelios {
            magic: read_cstr(bytes, 0x00..0x10),
            chip: read_cstr(bytes, 0x10..0x18),
            declared_size: read_u32_le(bytes, 0x1C),
            build_hash: read_cstr(bytes, 0x20..0x30),
            build_date: read_cstr(bytes, 0x30..0x40),
            enty_load_addr: read_u32_le(bytes, 0x44),
            enty_size: read_u32_le(bytes, 0x48),
            size_label_ok: &bytes[0x18..0x1C] == b"Size",
            enty_label_ok: &bytes[0x40..0x44] == b"ENTY",
        })
    }

    /// Structural checks against the *actual* file length. This is the raw material the M2
    /// image gate consumes; it does not itself gate anything.
    pub fn findings(&self, actual_len: usize) -> Vec<(Level, String)> {
        let mut out = Vec::new();

        if self.declared_size as usize != actual_len {
            out.push((
                Level::Error,
                format!(
                    "declared size {} (0x{:X}) != actual file length {} (0x{:X}) — truncated, padded, or wrong offset",
                    self.declared_size, self.declared_size, actual_len, actual_len
                ),
            ));
        } else {
            out.push((Level::Info, format!("declared size matches file length ({actual_len} bytes)")));
        }

        if !self.size_label_ok {
            out.push((Level::Warn, "missing \"Size\" label at 0x18 — header layout may differ for this image".into()));
        }
        if !self.enty_label_ok {
            out.push((Level::Warn, "missing \"ENTY\" label at 0x40 — entry table may differ for this image".into()));
        }
        if self.chip.is_empty() {
            out.push((Level::Warn, "empty chip string at 0x10".into()));
        }
        if self.enty_size == 0 {
            out.push((Level::Warn, "ENTY payload size is 0".into()));
        }
        if self.enty_size as usize > actual_len {
            out.push((
                Level::Warn,
                format!("ENTY size 0x{:X} is larger than the whole file (0x{:X})", self.enty_size, actual_len),
            ));
        }

        out
    }

    /// True when nothing in [`findings`](Self::findings) is an error, for a given file length.
    pub fn is_structurally_valid(&self, actual_len: usize) -> bool {
        !self.findings(actual_len).iter().any(|(l, _)| *l == Level::Error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal but well-formed header for a `total`-byte image.
    fn synth(total: usize, chip: &[u8], enty_addr: u32, enty_size: u32) -> Vec<u8> {
        let mut b = vec![0u8; total.max(HEADER_LEN)];
        b[0x00..0x0D].copy_from_slice(b"KT_Helios_v1b");
        b[0x10..0x10 + chip.len()].copy_from_slice(chip);
        b[0x18..0x1C].copy_from_slice(b"Size");
        b[0x1C..0x20].copy_from_slice(&(total as u32).to_le_bytes());
        b[0x20..0x27].copy_from_slice(b"75503ea");
        b[0x30..0x40].copy_from_slice(b"2025-06-30 V:1.0");
        b[0x40..0x44].copy_from_slice(b"ENTY");
        b[0x44..0x48].copy_from_slice(&enty_addr.to_le_bytes());
        b[0x48..0x4C].copy_from_slice(&enty_size.to_le_bytes());
        b
    }

    #[test]
    fn parses_the_documented_ja11_header() {
        // Values straight from docs/EXTRACTION.md for JadeAudio JA11_V2.2.bin.
        let img = synth(0x000106F0, b"KT02H20B", 0x00083000, 0x0001D188);
        let h = KtHelios::parse(&img).expect("valid header");
        assert_eq!(h.magic, "KT_Helios_v1b");
        assert_eq!(h.chip, "KT02H20B");
        assert_eq!(h.declared_size, 0x000106F0);
        assert_eq!(h.build_hash, "75503ea");
        assert_eq!(h.build_date, "2025-06-30 V:1.0");
        assert_eq!(h.enty_load_addr, 0x00083000);
        assert_eq!(h.enty_size, 0x0001D188);
        assert!(h.size_label_ok && h.enty_label_ok);
        assert!(h.is_structurally_valid(img.len()));
    }

    #[test]
    fn rejects_short_and_foreign_buffers() {
        assert!(matches!(KtHelios::parse(&[0u8; 8]), Err(ImageError::TooShort { .. })));
        let mut junk = vec![0u8; HEADER_LEN];
        junk[..4].copy_from_slice(b"ELF\0");
        assert_eq!(KtHelios::parse(&junk), Err(ImageError::BadMagic));
    }

    #[test]
    fn size_mismatch_is_an_error_finding() {
        let img = synth(1024, b"KT02H20B", 0x1000, 0x200);
        let h = KtHelios::parse(&img).unwrap();
        // Pretend the file was truncated: report against a wrong length.
        let findings = h.findings(512);
        assert!(findings.iter().any(|(l, _)| *l == Level::Error));
        assert!(!h.is_structurally_valid(512));
    }

    #[test]
    fn sniff_matches_prefix_only() {
        assert!(looks_like_kt_helios(b"KT_Helios_v1b......"));
        assert!(!looks_like_kt_helios(b"KT_Heli"));
    }
}
