//! Normal-mode HID command frames (the `0xFF01` vendor collection).
//!
//! **Confidence: `CaptureProven` / well-established.** These frames are fully reversed and
//! already driven successfully by the CLI (`handshake` / `unlock`), so their bytes are
//! exercised with exact-value tests here. This is the *app/EQ* channel and the unlock — the
//! firmware **download** is the separate CDC serial protocol in [`super::cdc`].
//!
//! Wire format (11-byte report, ID `0x4B`):
//! ```text
//!  byte:  0    1  2  3  4    5     6     7  8  9  10
//!         4B   <-- addr -->  cmd   00    <-- data -->
//!              (u32 LE)                  (u32 LE)
//! ```
//! The reply mirrors the frame; the 32-bit result lives at `resp[7..11]` (LE).

/// Report ID for the uniform command frame.
pub const REPORT_CMD: u8 = 0x4B;
/// Report ID for the unlock string.
pub const REPORT_UNLOCK: u8 = 0x54;

/// Known command bytes (frame byte 5). See `docs/PROTOCOL.md`.
pub mod cmd {
    /// Handshake / status — reply result word is `3` when the device is ready in ISP.
    pub const HANDSHAKE: u8 = 0x33;
    /// Status / handshake variant.
    pub const STATUS: u8 = 0x32;
    /// Read a 32-bit word at `addr`; result = the word.
    pub const READ: u8 = 0x08;
    /// Erase a region at `addr`; `data` field carries the size.
    pub const ERASE: u8 = 0x21;
    /// Write a 32-bit word at `addr` (no reply).
    pub const WRITE: u8 = 0x88;
}

/// The `"T12345678"` unlock report: `54 31 32 33 34 35 36 37 38 00` (10 bytes).
///
/// Sending this reboots the dongle into the `0x8888:0xCDC0` CDC bootloader.
pub fn unlock_frame() -> [u8; 10] {
    [REPORT_UNLOCK, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', 0x00]
}

/// Build the uniform 11-byte command frame.
pub fn command_frame(addr: u32, cmd: u8, data: u32) -> [u8; 11] {
    let a = addr.to_le_bytes();
    let d = data.to_le_bytes();
    [REPORT_CMD, a[0], a[1], a[2], a[3], cmd, 0x00, d[0], d[1], d[2], d[3]]
}

/// Convenience: the handshake/status frame at address 0.
pub fn handshake_frame() -> [u8; 11] {
    command_frame(0, cmd::HANDSHAKE, 0)
}

/// Extract the 32-bit result word from a reply (`resp[7..11]`, LE), if long enough.
pub fn result_word(resp: &[u8]) -> Option<u32> {
    if resp.len() >= 11 {
        Some(u32::from_le_bytes([resp[7], resp[8], resp[9], resp[10]]))
    } else {
        None
    }
}

/// A handshake reply of `3` means the device is ready in ISP mode.
pub fn is_ready(resp: &[u8]) -> bool {
    result_word(resp) == Some(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlock_is_the_documented_bytes() {
        assert_eq!(unlock_frame(), [0x54, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x00]);
    }

    #[test]
    fn handshake_frame_matches_docs() {
        assert_eq!(handshake_frame(), [0x4B, 0, 0, 0, 0, 0x33, 0x00, 0, 0, 0, 0]);
    }

    #[test]
    fn command_frame_places_addr_and_data_le() {
        // erase 0x200 bytes @ 0x00083000
        let f = command_frame(0x0008_3000, cmd::ERASE, 0x200);
        assert_eq!(f, [0x4B, 0x00, 0x30, 0x08, 0x00, 0x21, 0x00, 0x00, 0x02, 0x00, 0x00]);
    }

    #[test]
    fn result_word_reads_le_at_7() {
        let resp = [0x4B, 0, 0, 0, 0, 0x33, 0x00, 0x03, 0x00, 0x00, 0x00];
        assert_eq!(result_word(&resp), Some(3));
        assert!(is_ready(&resp));
        assert_eq!(result_word(&[0u8; 4]), None);
    }
}
