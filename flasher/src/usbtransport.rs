//! Real USB (libusb) transport for the CDC bootloader — the OrbStack/Linux path.
//!
//! This is the concrete [`proto::cdc::Transport`] that carries protocol bytes over the
//! bootloader's bulk endpoints (`0x03` OUT / `0x83` IN on the CDC-data interface). It lives
//! *outside* `proto/` because it depends on `rusb`/libusb — `proto/` stays dependency-free so
//! its logic can be tested with no hardware. This module is the opposite: it is thin, all
//! I/O, and can only be exercised with a real dongle in the bootloader state.
//!
//! [`crate::serialtransport::SerialTransport`] is the other implementation of the same trait,
//! driving the bootloader's CDC‑ACM tty instead of claiming the USB interface directly — that
//! one works natively on macOS too (confirmed on hardware, [`docs/MACOS-NATIVE.md`]); this one
//! remains the OrbStack/Linux route. [`crate::boottransport`] picks between them.
//!
//! ## Working-agent notes
//! - Hardware‑confirmed on Linux/OrbStack: after `ktflash unlock`, the device re-enumerates as
//!   `8888:cdc0`; this transport drives it over the raw bulk endpoints.
//! - On macOS, prefer [`crate::serialtransport::SerialTransport`] — this module still can't
//!   claim the interface there (`IOHIDFamily`/kernel CDC driver owns it), which is a real,
//!   unresolved limitation of the *libusb* route specifically, not of native macOS in general.
//! - `recv` reads one bulk packet (up to 64 B, the usual CDC bulk max-packet). If the real
//!   framing spans multiple packets you'll want to accumulate until a full frame is decodable
//!   — the `FrameCodec` should tell you where a frame ends. Adjust once the wire codec exists.
//! - The `write_control` line-coding calls mirror `cmd_bootdiag`; keep or drop them once a
//!   capture shows whether the bootloader cares about CDC line coding.
//! - Pair this with `proto::cdc::Session` + the real `KtCdcCodec` and a
//!   `proto::journal::Journal` to get a recoverable native flash.

use std::time::Duration;

use rusb::{Context, Direction, TransferType, UsbContext};

use crate::proto::cdc::{CdcError, Transport};
use crate::{BOOT_PID, BOOT_VID};

/// A claimed bootloader CDC-data interface, ready for bulk I/O.
pub struct RusbBootloaderTransport {
    handle: rusb::DeviceHandle<Context>,
    iface: u8,
    out_ep: u8,
    in_ep: u8,
    timeout: Duration,
}

fn map_err(e: rusb::Error) -> CdcError {
    match e {
        rusb::Error::Timeout => CdcError::Timeout,
        rusb::Error::NoDevice | rusb::Error::Io => CdcError::Disconnected,
        other => CdcError::Protocol(format!("usb: {other}")),
    }
}

impl RusbBootloaderTransport {
    /// Locate the `8888:cdc0` bootloader, claim its CDC-data (class 10) bulk interface, and
    /// return a ready transport. Errors (as a human string) if the bootloader isn't present —
    /// run `ktflash unlock` first — or if the interface can't be claimed (macOS: use OrbStack).
    pub fn open() -> Result<Self, String> {
        Self::open_with_timeout(Duration::from_millis(2000))
    }

    pub fn open_with_timeout(timeout: Duration) -> Result<Self, String> {
        let ctx = Context::new().map_err(|e| e.to_string())?;
        let dev = ctx
            .devices()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|d| {
                d.device_descriptor()
                    .map(|dd| dd.vendor_id() == BOOT_VID && dd.product_id() == BOOT_PID)
                    .unwrap_or(false)
            })
            .ok_or("bootloader not present — run `ktflash unlock` first")?;

        let cfg = dev.active_config_descriptor().map_err(|e| e.to_string())?;
        let mut target = None;
        for iface in cfg.interfaces() {
            for id in iface.descriptors() {
                if id.class_code() != 10 {
                    continue;
                }
                let (mut o, mut i) = (None, None);
                for e in id.endpoint_descriptors() {
                    if e.transfer_type() == TransferType::Bulk {
                        match e.direction() {
                            Direction::Out => o = Some(e.address()),
                            Direction::In => i = Some(e.address()),
                        }
                    }
                }
                if let (Some(o), Some(i)) = (o, i) {
                    target = Some((id.interface_number(), o, i));
                }
            }
        }
        let (iface, out_ep, in_ep) = target.ok_or("no CDC-data bulk interface on the bootloader")?;

        let handle = dev.open().map_err(|e| format!("open: {e}"))?;
        let _ = handle.set_auto_detach_kernel_driver(true);
        handle
            .claim_interface(iface)
            .map_err(|e| format!("claim iface {iface}: {e} — macOS blocks this; run inside OrbStack"))?;
        // Best-effort CDC line coding (mirrors `cmd_bootdiag`); harmless if ignored.
        let _ = handle.write_control(0x21, 0x20, 0, 0, &[0x00, 0xc2, 0x01, 0x00, 0x00, 0x00, 0x08], timeout);
        let _ = handle.write_control(0x21, 0x22, 0x0003, 0, &[], timeout);

        Ok(RusbBootloaderTransport { handle, iface, out_ep, in_ep, timeout })
    }
}

impl Transport for RusbBootloaderTransport {
    fn send(&mut self, bytes: &[u8]) -> Result<(), CdcError> {
        self.handle.write_bulk(self.out_ep, bytes, self.timeout).map(|_| ()).map_err(map_err)
    }

    fn recv(&mut self, timeout: Duration) -> Result<Vec<u8>, CdcError> {
        let mut buf = [0u8; 64];
        let n = self.handle.read_bulk(self.in_ep, &mut buf, timeout).map_err(map_err)?;
        Ok(buf[..n].to_vec())
    }
}

impl Drop for RusbBootloaderTransport {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(self.iface);
    }
}
