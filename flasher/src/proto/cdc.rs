//! The CDC bootloader **download** protocol (roadmap M1.5) — the transport-independent core.
//!
//! ## What is and isn't known
//!
//! The vendor drives the `0x8888:0xCDC0` bootloader over a Qt `QSerialPort` state machine
//! (`Shake hand` → `Erase` → `Program` → `UPGRADE FIRMWARE SUCCESS`). The **exact on-the-wire
//! byte framing is now fully reversed** and lives in [`crate::proto::ktcdc`]; the shipped
//! native write (`ktflash flash-cdc`) drives it directly. This module keeps the
//! transport-independent, `Message`-based `Session` + `FakeBootloader` used for **tests and
//! replay fixtures**; [`PendingCodec`] is the placeholder for wiring the real framing into
//! *this* `Session` (not needed by the shipped writer — see `proto::ktcdc`).
//!
//! So this module splits cleanly along that fault line:
//!
//! - [`Message`] — the *semantic* commands/responses. These are real and stable.
//! - [`Session`] — the state machine: ordering, retries, timeouts, progress, verify. Real
//!   and **fully unit-tested today** against [`FakeBootloader`].
//! - [`FrameCodec`] — the seam where semantics meet bytes. [`ReferenceCodec`] is a simple,
//!   self-consistent encoding used for tests and the emulator; [`PendingCodec`] is the
//!   honest placeholder for the *real* KTMicro framing. When M0/M1 capture lands, implement
//!   `KtCdcCodec` and drop it in — nothing else in this module changes.
//!
//! Nothing here does I/O; a real run wires a [`Transport`] over the OrbStack/Linux serial
//! path in `main.rs`.

use std::collections::VecDeque;
use std::time::Duration;

/// A semantic bootloader message (either direction). Unknown payloads are preserved
/// losslessly via [`Message::Unknown`] so a decoder never silently drops information.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// Host → device: "Shake hand".
    Handshake,
    /// Host → device: erase a region before programming.
    Erase { addr: u32, len: u32 },
    /// Host → device: program a chunk of image bytes at `addr`.
    Program { addr: u32, data: Vec<u8> },
    /// Host → device: verify the written image.
    Verify,
    /// Host → device: read `len` bytes of flash at `addr` (roadmap M4a — readback). Whether
    /// the bootloader actually supports this is unproven; the type exists so the writer/dumper
    /// can be built and tested against the emulator before hardware confirms feasibility.
    ReadFlash { addr: u32, len: u32 },
    /// Device → host: a chunk of read-back data (response to [`Message::ReadFlash`]).
    Data(Vec<u8>),
    /// Host → device: reboot out of the bootloader (typically no reply).
    Reset,
    /// Device → host: positive acknowledgement.
    Ack,
    /// Device → host: negative acknowledgement, with an optional error code.
    Nack { code: Option<u8> },
    /// Device → host: a status word (e.g. progress / state).
    Status(u32),
    /// Any frame we can carry but not yet classify.
    Unknown(Vec<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlashStage {
    Idle,
    Handshake,
    Erase,
    Program,
    Verify,
    Reset,
    Done,
    Failed,
}

impl FlashStage {
    pub fn name(self) -> &'static str {
        match self {
            FlashStage::Idle => "idle",
            FlashStage::Handshake => "handshake",
            FlashStage::Erase => "erase",
            FlashStage::Program => "program",
            FlashStage::Verify => "verify",
            FlashStage::Reset => "reset",
            FlashStage::Done => "done",
            FlashStage::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CdcError {
    /// This `Message`-based `Session` isn't wired to the CDC framing — [`PendingCodec`]
    /// returns this. The real, reversed framing lives in [`crate::proto::ktcdc`]; use the
    /// `flash-cdc` command for the native write.
    Unreversed,
    /// No reply within the deadline.
    Timeout,
    /// Device sent a NACK.
    Nack(Option<u8>),
    /// The device dropped off the bus (unplug / re-enumerate / serial handle died).
    Disconnected,
    /// A reply was received but did not decode into a valid frame.
    BadFrame(String),
    /// The verify step reported the written image does not match.
    VerifyMismatch,
    /// Anything else, with context.
    Protocol(String),
}

impl std::fmt::Display for CdcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CdcError::Unreversed => write!(
                f,
                "this Message-based Session isn't wired to the CDC framing — use `ktflash flash-cdc` for the native write (framing lives in proto::ktcdc)"
            ),
            CdcError::Timeout => write!(f, "timed out waiting for the bootloader"),
            CdcError::Nack(c) => match c {
                Some(c) => write!(f, "device NACK (code 0x{c:02x})"),
                None => write!(f, "device NACK"),
            },
            CdcError::Disconnected => write!(f, "device disconnected mid-operation"),
            CdcError::BadFrame(m) => write!(f, "undecodable frame: {m}"),
            CdcError::VerifyMismatch => write!(f, "post-write verify failed (image mismatch)"),
            CdcError::Protocol(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for CdcError {}

/// A byte pipe to the bootloader. Implemented over real serial in `main.rs`, and by
/// [`FakeBootloader`] for tests.
pub trait Transport {
    fn send(&mut self, bytes: &[u8]) -> Result<(), CdcError>;
    fn recv(&mut self, timeout: Duration) -> Result<Vec<u8>, CdcError>;
}

/// The seam between [`Message`] semantics and on-the-wire bytes.
pub trait FrameCodec {
    fn encode(&self, msg: &Message) -> Result<Vec<u8>, CdcError>;
    fn decode(&self, bytes: &[u8]) -> Result<Message, CdcError>;
}

/// Placeholder codec for the `Message`-based [`Session`]: every call errors with
/// [`CdcError::Unreversed`]. The real, reversed CDC framing lives in
/// [`crate::proto::ktcdc`] and is driven directly by `ktflash flash-cdc`, so this `Session`
/// path stays emulator/test-only.
pub struct PendingCodec;

impl FrameCodec for PendingCodec {
    fn encode(&self, _msg: &Message) -> Result<Vec<u8>, CdcError> {
        Err(CdcError::Unreversed)
    }
    fn decode(&self, _bytes: &[u8]) -> Result<Message, CdcError> {
        Err(CdcError::Unreversed)
    }
}

/// A simple, self-consistent encoding used for tests, the emulator, and `--replay` fixtures.
///
/// **This is NOT the KTMicro wire format.** It is a stand-in so the sequencing logic can be
/// exercised end-to-end without hardware. Format: a 1-byte tag then little-endian fields.
pub struct ReferenceCodec;

impl ReferenceCodec {
    const T_HANDSHAKE: u8 = 0x01;
    const T_ERASE: u8 = 0x02;
    const T_PROGRAM: u8 = 0x03;
    const T_VERIFY: u8 = 0x04;
    const T_RESET: u8 = 0x05;
    const T_ACK: u8 = 0x06;
    const T_NACK: u8 = 0x07;
    const T_STATUS: u8 = 0x08;
    const T_READ: u8 = 0x09;
    const T_DATA: u8 = 0x0a;
    const T_UNKNOWN: u8 = 0x00;
}

fn le32(b: &[u8], off: usize) -> Result<u32, CdcError> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| CdcError::BadFrame(format!("need 4 bytes at offset {off}, have {}", b.len())))
}

impl FrameCodec for ReferenceCodec {
    fn encode(&self, msg: &Message) -> Result<Vec<u8>, CdcError> {
        let mut v = Vec::new();
        match msg {
            Message::Handshake => v.push(Self::T_HANDSHAKE),
            Message::Erase { addr, len } => {
                v.push(Self::T_ERASE);
                v.extend_from_slice(&addr.to_le_bytes());
                v.extend_from_slice(&len.to_le_bytes());
            }
            Message::Program { addr, data } => {
                v.push(Self::T_PROGRAM);
                v.extend_from_slice(&addr.to_le_bytes());
                v.extend_from_slice(&(data.len() as u16).to_le_bytes());
                v.extend_from_slice(data);
            }
            Message::Verify => v.push(Self::T_VERIFY),
            Message::ReadFlash { addr, len } => {
                v.push(Self::T_READ);
                v.extend_from_slice(&addr.to_le_bytes());
                v.extend_from_slice(&len.to_le_bytes());
            }
            Message::Data(d) => {
                v.push(Self::T_DATA);
                v.extend_from_slice(&(d.len() as u16).to_le_bytes());
                v.extend_from_slice(d);
            }
            Message::Reset => v.push(Self::T_RESET),
            Message::Ack => v.push(Self::T_ACK),
            Message::Nack { code } => {
                v.push(Self::T_NACK);
                v.push(code.unwrap_or(0));
            }
            Message::Status(w) => {
                v.push(Self::T_STATUS);
                v.extend_from_slice(&w.to_le_bytes());
            }
            Message::Unknown(raw) => {
                v.push(Self::T_UNKNOWN);
                v.extend_from_slice(raw);
            }
        }
        Ok(v)
    }

    fn decode(&self, bytes: &[u8]) -> Result<Message, CdcError> {
        let (&tag, rest) = bytes
            .split_first()
            .ok_or_else(|| CdcError::BadFrame("empty frame".into()))?;
        Ok(match tag {
            Self::T_HANDSHAKE => Message::Handshake,
            Self::T_ERASE => Message::Erase { addr: le32(rest, 0)?, len: le32(rest, 4)? },
            Self::T_PROGRAM => {
                let addr = le32(rest, 0)?;
                let n = rest
                    .get(4..6)
                    .map(|s| u16::from_le_bytes([s[0], s[1]]) as usize)
                    .ok_or_else(|| CdcError::BadFrame("program: missing length".into()))?;
                let data = rest
                    .get(6..6 + n)
                    .ok_or_else(|| CdcError::BadFrame("program: truncated payload".into()))?
                    .to_vec();
                Message::Program { addr, data }
            }
            Self::T_VERIFY => Message::Verify,
            Self::T_READ => Message::ReadFlash { addr: le32(rest, 0)?, len: le32(rest, 4)? },
            Self::T_DATA => {
                let n = rest
                    .get(0..2)
                    .map(|s| u16::from_le_bytes([s[0], s[1]]) as usize)
                    .ok_or_else(|| CdcError::BadFrame("data: missing length".into()))?;
                let d = rest
                    .get(2..2 + n)
                    .ok_or_else(|| CdcError::BadFrame("data: truncated payload".into()))?
                    .to_vec();
                Message::Data(d)
            }
            Self::T_RESET => Message::Reset,
            Self::T_ACK => Message::Ack,
            Self::T_NACK => Message::Nack { code: rest.first().copied() },
            Self::T_STATUS => Message::Status(le32(rest, 0)?),
            Self::T_UNKNOWN => Message::Unknown(rest.to_vec()),
            other => return Err(CdcError::BadFrame(format!("unknown tag 0x{other:02x}"))),
        })
    }
}

/// Per-stage timeout and retry policy. Explicit, not a blanket "retry 3 times" — the
/// working agent should tune these from capture timing (roadmap M1 timing spec).
#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    pub handshake_timeout: Duration,
    pub erase_timeout: Duration,
    pub program_timeout: Duration,
    pub verify_timeout: Duration,
    /// Timeout for a single readback (roadmap M4a).
    pub read_timeout: Duration,
    /// How many times to re-send a command that *times out* (NACKs are terminal).
    pub max_retries: u8,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            handshake_timeout: Duration::from_millis(1000),
            erase_timeout: Duration::from_millis(5000),
            program_timeout: Duration::from_millis(2000),
            verify_timeout: Duration::from_millis(5000),
            read_timeout: Duration::from_millis(2000),
            max_retries: 3,
        }
    }
}

/// The download state machine. Generic over the byte [`Transport`] and the [`FrameCodec`]
/// so the exact same logic runs against real serial, a replay, or the emulator.
pub struct Session<T: Transport, C: FrameCodec> {
    transport: T,
    codec: C,
    policy: RetryPolicy,
    stage: FlashStage,
}

impl<T: Transport, C: FrameCodec> Session<T, C> {
    pub fn new(transport: T, codec: C, policy: RetryPolicy) -> Self {
        Session { transport, codec, policy, stage: FlashStage::Idle }
    }

    pub fn stage(&self) -> FlashStage {
        self.stage
    }

    /// Encode → send → recv → decode a single request/response.
    fn exchange(&mut self, msg: &Message, timeout: Duration) -> Result<Message, CdcError> {
        let bytes = self.codec.encode(msg)?;
        self.transport.send(&bytes)?;
        let resp = self.transport.recv(timeout)?;
        self.codec.decode(&resp)
    }

    /// Send a request and require a non-NACK reply, retrying only on timeout.
    fn expect(&mut self, msg: &Message, timeout: Duration) -> Result<Message, CdcError> {
        let mut last = CdcError::Timeout;
        for _ in 0..=self.policy.max_retries {
            match self.exchange(msg, timeout) {
                Ok(Message::Nack { code }) => return Err(CdcError::Nack(code)),
                Ok(other) => return Ok(other),
                Err(CdcError::Timeout) => {
                    last = CdcError::Timeout;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    /// Run the whole `handshake → erase → program → verify → reset` sequence for `image`,
    /// programming `chunk`-byte payloads. `progress(stage, done_bytes, total_bytes)` is
    /// called as it advances. On any error the session is left in [`FlashStage::Failed`].
    ///
    /// With [`PendingCodec`] this returns [`CdcError::Unreversed`] immediately (at encode
    /// time) — a real flash is impossible until the wire codec exists.
    pub fn flash<F: FnMut(FlashStage, usize, usize)>(
        &mut self,
        image: &[u8],
        chunk: usize,
        mut progress: F,
    ) -> Result<(), CdcError> {
        match self.flash_steps(image, chunk, &mut progress) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.stage = FlashStage::Failed;
                Err(e)
            }
        }
    }

    /// Read `len` bytes of flash starting at `addr`, in `chunk`-byte requests (roadmap M4a
    /// readback / M4b backup). Returns [`CdcError::Unreversed`] with [`PendingCodec`], so a
    /// backup is impossible until the real read framing is proven — do **not** claim a flash
    /// is reversible until this returns real bytes on hardware and a full restore is verified.
    pub fn read_region(&mut self, addr: u32, len: u32, chunk: usize) -> Result<Vec<u8>, CdcError> {
        let step = chunk.max(1) as u32;
        let mut out = Vec::with_capacity(len as usize);
        let mut off = 0u32;
        while off < len {
            let want = step.min(len - off);
            let msg = Message::ReadFlash { addr: addr + off, len: want };
            match self.expect(&msg, self.policy.read_timeout)? {
                Message::Data(d) => out.extend_from_slice(&d),
                other => return Err(CdcError::Protocol(format!("expected Data, got {other:?}"))),
            }
            off += want;
        }
        Ok(out)
    }

    fn flash_steps(
        &mut self,
        image: &[u8],
        chunk: usize,
        progress: &mut dyn FnMut(FlashStage, usize, usize),
    ) -> Result<(), CdcError> {
        let total = image.len();

        self.stage = FlashStage::Handshake;
        progress(self.stage, 0, total);
        self.expect(&Message::Handshake, self.policy.handshake_timeout)?;

        self.stage = FlashStage::Erase;
        progress(self.stage, 0, total);
        self.expect(&Message::Erase { addr: 0, len: total as u32 }, self.policy.erase_timeout)?;

        self.stage = FlashStage::Program;
        let step = chunk.max(1);
        let mut off = 0;
        while off < total {
            let end = (off + step).min(total);
            let msg = Message::Program { addr: off as u32, data: image[off..end].to_vec() };
            self.expect(&msg, self.policy.program_timeout)?;
            off = end;
            progress(FlashStage::Program, off, total);
        }

        self.stage = FlashStage::Verify;
        progress(self.stage, total, total);
        self.expect(&Message::Verify, self.policy.verify_timeout).map_err(|e| match e {
            CdcError::Nack(_) => CdcError::VerifyMismatch,
            other => other,
        })?;

        self.stage = FlashStage::Reset;
        progress(self.stage, total, total);
        // Reset commonly has no reply; best-effort send only.
        if let Ok(bytes) = self.codec.encode(&Message::Reset) {
            let _ = self.transport.send(&bytes);
        }

        self.stage = FlashStage::Done;
        progress(self.stage, total, total);
        Ok(())
    }
}

/// How the [`FakeBootloader`] should behave — one knob per failure mode the roadmap's M0
/// corpus calls for, so recovery logic can be tested without ever risking a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Behavior {
    /// Happy path: ACK everything, verify passes.
    Success,
    /// NACK the erase command.
    NackOnErase,
    /// Never reply to program frames (exercises timeout + retry, then give-up).
    TimeoutOnProgram,
    /// ACK the first `n` program chunks, then NACK.
    PartialWrite(usize),
    /// Drop off the bus on the first program frame.
    DisconnectDuringProgram,
    /// ACK the write but fail verify.
    VerifyMismatch,
}

/// A deterministic in-memory bootloader used by tests and `--replay`. It decodes incoming
/// frames with its codec and enqueues encoded responses; `recv` drains that queue (or
/// yields [`CdcError::Timeout`] when empty, modelling a silent device).
pub struct FakeBootloader<C: FrameCodec> {
    codec: C,
    behavior: Behavior,
    outbox: VecDeque<Vec<u8>>,
    programmed: usize,
    disconnected: bool,
    /// Emulated flash contents, served for [`Message::ReadFlash`] (roadmap M4a tests).
    flash: Vec<u8>,
}

impl<C: FrameCodec> FakeBootloader<C> {
    pub fn new(codec: C, behavior: Behavior) -> Self {
        FakeBootloader {
            codec,
            behavior,
            outbox: VecDeque::new(),
            programmed: 0,
            disconnected: false,
            flash: Vec::new(),
        }
    }

    /// Like [`new`](Self::new) but with emulated flash contents to serve reads from.
    pub fn new_with_flash(codec: C, behavior: Behavior, flash: Vec<u8>) -> Self {
        FakeBootloader { flash, ..Self::new(codec, behavior) }
    }

    /// Number of program chunks accepted so far (for assertions).
    pub fn programmed_chunks(&self) -> usize {
        self.programmed
    }

    fn reply(&mut self, msg: Message) {
        if let Ok(bytes) = self.codec.encode(&msg) {
            self.outbox.push_back(bytes);
        }
    }
}

impl<C: FrameCodec> Transport for FakeBootloader<C> {
    fn send(&mut self, bytes: &[u8]) -> Result<(), CdcError> {
        if self.disconnected {
            return Err(CdcError::Disconnected);
        }
        let msg = self.codec.decode(bytes)?;
        let behavior = self.behavior;
        match (behavior, &msg) {
            (_, Message::Handshake) => self.reply(Message::Ack),

            (Behavior::NackOnErase, Message::Erase { .. }) => self.reply(Message::Nack { code: Some(0x01) }),
            (_, Message::Erase { .. }) => self.reply(Message::Ack),

            (Behavior::TimeoutOnProgram, Message::Program { .. }) => { /* no reply → recv times out */ }
            (Behavior::DisconnectDuringProgram, Message::Program { .. }) => {
                self.disconnected = true;
                return Err(CdcError::Disconnected);
            }
            (Behavior::PartialWrite(n), Message::Program { .. }) => {
                if self.programmed >= n {
                    self.reply(Message::Nack { code: Some(0x02) });
                } else {
                    self.programmed += 1;
                    self.reply(Message::Ack);
                }
            }
            (_, Message::Program { .. }) => {
                self.programmed += 1;
                self.reply(Message::Ack);
            }

            (Behavior::VerifyMismatch, Message::Verify) => self.reply(Message::Nack { code: Some(0x03) }),
            (_, Message::Verify) => self.reply(Message::Ack),

            (_, Message::ReadFlash { addr, len }) => {
                let a = *addr as usize;
                let end = a.saturating_add(*len as usize).min(self.flash.len());
                let data = if a < self.flash.len() { self.flash[a..end].to_vec() } else { Vec::new() };
                self.reply(Message::Data(data));
            }

            (_, Message::Reset) => { /* bootloaders typically don't answer a reset */ }

            _ => self.reply(Message::Nack { code: Some(0xff) }),
        }
        Ok(())
    }

    fn recv(&mut self, _timeout: Duration) -> Result<Vec<u8>, CdcError> {
        if self.disconnected {
            return Err(CdcError::Disconnected);
        }
        self.outbox.pop_front().ok_or(CdcError::Timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(behavior: Behavior) -> Session<FakeBootloader<ReferenceCodec>, ReferenceCodec> {
        Session::new(
            FakeBootloader::new(ReferenceCodec, behavior),
            ReferenceCodec,
            RetryPolicy::default(),
        )
    }

    #[test]
    fn reference_codec_round_trips_every_message() {
        let codec = ReferenceCodec;
        let msgs = [
            Message::Handshake,
            Message::Erase { addr: 0x1000, len: 0x200 },
            Message::Program { addr: 0x40, data: vec![1, 2, 3, 4] },
            Message::Verify,
            Message::ReadFlash { addr: 0x8000, len: 64 },
            Message::Data(vec![9, 8, 7, 6, 5]),
            Message::Reset,
            Message::Ack,
            Message::Nack { code: Some(7) },
            Message::Status(0xdead_beef),
            Message::Unknown(vec![0xaa, 0xbb]),
        ];
        for m in msgs {
            let bytes = codec.encode(&m).unwrap();
            assert_eq!(codec.decode(&bytes).unwrap(), m, "round-trip failed for {m:?}");
        }
    }

    #[test]
    fn pending_codec_refuses_to_encode() {
        assert_eq!(PendingCodec.encode(&Message::Handshake), Err(CdcError::Unreversed));
        assert_eq!(PendingCodec.decode(&[0x01]), Err(CdcError::Unreversed));
    }

    #[test]
    fn happy_path_flashes_and_reaches_done() {
        let image = vec![0xABu8; 1000];
        let mut s = session(Behavior::Success);
        let mut last = FlashStage::Idle;
        s.flash(&image, 256, |stage, _done, _total| last = stage).unwrap();
        assert_eq!(s.stage(), FlashStage::Done);
        assert_eq!(last, FlashStage::Done);
    }

    #[test]
    fn chunking_covers_the_whole_image() {
        // 1000 bytes / 256 = 4 chunks (256,256,256,232)
        let image = vec![0u8; 1000];
        let mut s = session(Behavior::Success);
        let mut max_done = 0;
        s.flash(&image, 256, |stage, done, _| {
            if stage == FlashStage::Program {
                max_done = max_done.max(done);
            }
        })
        .unwrap();
        assert_eq!(max_done, 1000);
    }

    #[test]
    fn nack_on_erase_is_terminal() {
        let mut s = session(Behavior::NackOnErase);
        let err = s.flash(&[0u8; 64], 32, |_, _, _| {}).unwrap_err();
        assert_eq!(err, CdcError::Nack(Some(0x01)));
        assert_eq!(s.stage(), FlashStage::Failed);
    }

    #[test]
    fn program_timeout_gives_up_after_retries() {
        let mut s = session(Behavior::TimeoutOnProgram);
        let err = s.flash(&[0u8; 64], 32, |_, _, _| {}).unwrap_err();
        assert_eq!(err, CdcError::Timeout);
        assert_eq!(s.stage(), FlashStage::Failed);
    }

    #[test]
    fn verify_mismatch_is_reported_distinctly() {
        let mut s = session(Behavior::VerifyMismatch);
        let err = s.flash(&[0u8; 64], 32, |_, _, _| {}).unwrap_err();
        assert_eq!(err, CdcError::VerifyMismatch);
    }

    #[test]
    fn disconnect_mid_program_surfaces_as_disconnected() {
        let mut s = session(Behavior::DisconnectDuringProgram);
        let err = s.flash(&[0u8; 128], 32, |_, _, _| {}).unwrap_err();
        assert_eq!(err, CdcError::Disconnected);
    }

    #[test]
    fn partial_write_stops_at_the_configured_chunk() {
        // Accept 2 chunks then NACK. 128 bytes / 32 = 4 chunks total.
        let mut s = session(Behavior::PartialWrite(2));
        let err = s.flash(&[0u8; 128], 32, |_, _, _| {}).unwrap_err();
        assert_eq!(err, CdcError::Nack(Some(0x02)));
    }

    #[test]
    fn read_region_returns_emulated_flash() {
        let flash: Vec<u8> = (0..=255u8).collect();
        let mut s = Session::new(
            FakeBootloader::new_with_flash(ReferenceCodec, Behavior::Success, flash.clone()),
            ReferenceCodec,
            RetryPolicy::default(),
        );
        let got = s.read_region(0, 100, 32).unwrap();
        assert_eq!(got, flash[..100].to_vec());
        // reading from an offset works too
        let mid = s.read_region(64, 32, 16).unwrap();
        assert_eq!(mid, flash[64..96].to_vec());
    }

    #[test]
    fn read_region_with_pending_codec_errors() {
        let mut s = Session::new(
            FakeBootloader::new_with_flash(ReferenceCodec, Behavior::Success, vec![0u8; 16]),
            PendingCodec,
            RetryPolicy::default(),
        );
        assert_eq!(s.read_region(0, 4, 4), Err(CdcError::Unreversed));
    }

    #[test]
    fn session_with_pending_codec_cannot_flash() {
        let mut s = Session::new(
            FakeBootloader::new(ReferenceCodec, Behavior::Success),
            PendingCodec,
            RetryPolicy::default(),
        );
        assert_eq!(s.flash(&[0u8; 16], 8, |_, _, _| {}), Err(CdcError::Unreversed));
    }
}
