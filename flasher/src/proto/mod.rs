//! `ktflash-proto` — the **transport-independent** protocol core (roadmap M1.5).
//!
//! Everything here is deliberately free of `rusb`, serial ports, OrbStack, and any OS
//! I/O, so it compiles and unit-tests on any machine **with no dongle attached**. The
//! hardware plumbing lives in `main.rs`; this module owns the *logic*:
//!
//! - [`image`]      — parse/validate the `KT_Helios` firmware container.
//! - [`frame`]      — the fully-known normal-mode HID command frames (`0x4B` / `0x54`).
//! - [`transcript`] — the M0 normalized capture-transcript format + a `tshark` decoder.
//! - [`ktcdc`] — the **byte-exact CDC download framing** (header/CRC/packet planner),
//!   reversed from the vendor tool and unit-tested. This is what `ktflash flash-cdc` uses.
//! - [`cdc`] — the transport-independent `Message`-based `Session` + `FakeBootloader`
//!   emulator used for tests/replay. Its `FrameCodec` seam ([`cdc::PendingCodec`]) is a
//!   placeholder; the shipped native write drives [`ktcdc`] directly.
//!
//! **Confidence discipline (roadmap M1):** every protocol fact is tagged with a
//! [`Confidence`] so a reader can tell capture-proven ground truth from an educated guess.
//!
//! > Working-agent note: this is scaffolding meant to be filled in against real USBPcap
//! > captures. Some items are unused until the wire codec lands, hence the module-wide
//! > `dead_code` allowance below.
#![allow(dead_code)]

pub mod cdc;
pub mod compat;
pub mod fingerprint;
pub mod frame;
pub mod image;
pub mod journal;
pub mod ktcdc;
pub mod ktcdc_driver;
pub mod ktcdc_journal;
pub mod manifest;
pub mod plan;
pub mod transcript;

/// How well we actually *know* a given protocol fact.
///
/// The whole point of M0/M1 is to move fields up this ladder from `Hypothetical` to
/// `CaptureProven`. Anything that drives a destructive write must be `CaptureProven`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    /// Seen on the wire in a real USBPcap capture.
    CaptureProven,
    /// Inferred from the vendor binary (Ghidra/rizin) but not yet confirmed live.
    StaticallyInferred,
    /// An educated guess, consistent with what we know but unverified.
    Hypothetical,
}

impl Confidence {
    pub fn tag(self) -> &'static str {
        match self {
            Confidence::CaptureProven => "capture-proven",
            Confidence::StaticallyInferred => "static-inferred",
            Confidence::Hypothetical => "hypothetical",
        }
    }
}
