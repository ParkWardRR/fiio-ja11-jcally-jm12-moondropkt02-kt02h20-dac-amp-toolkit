//! Serial (CDC-ACM tty) transport for the `8888:cdc0` bootloader.
//!
//! ## Why
//!
//! [`docs/CDC-PROTOCOL.md`] describes the bootloader as *"USB **CDC**: bulk `0x03` OUT / `0x83`
//! IN (interface 1); appears as `/dev/ttyACM0`"* — i.e. a plain serial device. The kernel CDC
//! driver already owns those endpoints and publishes them as a tty, so we do not need to claim
//! the USB interface at all.
//!
//! That matters most on **macOS**, where claiming is impossible and OrbStack exists solely to
//! work around it (`docs/MACOS-NATIVE.md` §3). It also helps on Linux: no `cdc_acm` detach, no
//! interface claim, and no race with ModemManager for the same interface.
//!
//! ## Status
//!
//! > **Untested.** Written from the protocol docs; never run against a device. Every assumption
//! > that could plausibly be wrong is marked `UNVERIFIED:` below.
//!
//! ## Deliberately no `serialport` crate
//!
//! `libc` is already in `Cargo.lock` transitively (MIT/Apache-2.0), so using it directly adds no
//! new `cargo deny` question and no new supply-chain surface for a tool that erases hardware.

use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::proto::cdc::{CdcError, Transport};

// The `8888:cdc0` bootloader IDs come from the crate root (`main.rs` `BOOT_VID`/`BOOT_PID`),
// imported inside `matches_bootloader` below — only the Linux path can actually check them.

/// Terminal-control ioctls, defined here rather than taken from `libc` so the build does not
/// depend on which constants a given `libc` version exposes per-platform.
///
/// Linux: `TIOCMBIS = 0x5416`. macOS/BSD: `TIOCMBIS = _IOW('t', 108, int) = 0x8004746c`.
/// `TIOCM_DTR`/`TIOCM_RTS` are `0x002`/`0x004` on both.
mod tio {
    #[cfg(target_os = "linux")]
    pub const TIOCMBIS: libc::c_ulong = 0x5416;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    pub const TIOCMBIS: libc::c_ulong = 0x8004_746c;

    pub const TIOCM_DTR: libc::c_int = 0x002;
    pub const TIOCM_RTS: libc::c_int = 0x004;
}

fn last_os_error() -> io::Error {
    io::Error::last_os_error()
}

/// An open, raw-mode, DTR/RTS-asserted serial port.
#[derive(Debug)]
pub struct SerialTransport {
    fd: libc::c_int,
    path: PathBuf,
}

impl SerialTransport {
    /// Open a specific port and put it into binary-safe raw mode.
    pub fn open(path: &Path) -> Result<Self, String> {
        // O_NOCTTY: never make this our controlling terminal.
        // O_NONBLOCK: on macOS, opening a modem device otherwise blocks waiting for carrier.
        //   We keep it set and use poll(2) for timeouts.
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| format!("{}: path contains a NUL byte", path.display()))?;
        let fd = unsafe {
            libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK)
        };
        if fd < 0 {
            let e = last_os_error();
            return Err(match e.kind() {
                io::ErrorKind::PermissionDenied => format!(
                    "{}: permission denied — install packaging/99-ktflash.rules (Linux), \
                     or check the node's owner (macOS /dev/cu.* is normally world-writable)",
                    path.display()
                ),
                _ => format!("open {}: {e}", path.display()),
            });
        }
        let me = SerialTransport { fd, path: path.to_path_buf() };
        me.configure()?;
        Ok(me)
    }

    /// Raw mode, 115200 8N1, no flow control, then assert DTR/RTS.
    ///
    /// Binary safety is not optional here: the protocol's tokens include `0x11`/`0x13`
    /// (XON/XOFF if `IXON` is left on) and `0x0d`/`0x0a` (mangled by `ICRNL`/`OPOST`), and the
    /// 1 KB data payloads contain arbitrary bytes. `cfmakeraw` clears all of these; the explicit
    /// clears below are belt-and-braces against platform differences in `cfmakeraw`.
    fn configure(&self) -> Result<(), String> {
        unsafe {
            let mut tio: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(self.fd, &mut tio) != 0 {
                return Err(format!("tcgetattr {}: {}", self.path.display(), last_os_error()));
            }
            libc::cfmakeraw(&mut tio);

            tio.c_cflag |= libc::CLOCAL | libc::CREAD; // ignore modem lines, enable receiver
            tio.c_cflag &= !libc::CRTSCTS; // no hardware flow control
            tio.c_cflag &= !libc::CSTOPB; // 1 stop bit
            tio.c_cflag &= !libc::PARENB; // no parity
            tio.c_iflag &= !(libc::IXON | libc::IXOFF | libc::IXANY); // no software flow control
            tio.c_iflag &= !(libc::ICRNL | libc::INLCR | libc::IGNCR); // no CR/LF translation
            tio.c_oflag &= !libc::OPOST; // no output post-processing

            // We poll(2) for readiness, so reads must never block in the tty layer.
            tio.c_cc[libc::VMIN] = 0;
            tio.c_cc[libc::VTIME] = 0;

            // The bootloader's SET_LINE_CODING is 115200 8N1 (main.rs:822, usbtransport.rs:95).
            // UNVERIFIED: for USB CDC the line rate is nominal — the bootloader almost certainly
            // ignores it — but match the vendor anyway rather than guess.
            if libc::cfsetispeed(&mut tio, libc::B115200) != 0
                || libc::cfsetospeed(&mut tio, libc::B115200) != 0
            {
                return Err(format!("cfsetspeed {}: {}", self.path.display(), last_os_error()));
            }
            if libc::tcsetattr(self.fd, libc::TCSANOW, &tio) != 0 {
                return Err(format!("tcsetattr {}: {}", self.path.display(), last_os_error()));
            }
            libc::tcflush(self.fd, libc::TCIOFLUSH);

            // Assert DTR + RTS explicitly. The protocol doc says the host "asserts DTR/RTS
            // (opens the port) then exchanges framed messages" — do not rely on open(2) doing
            // it, because /dev/cu.* on macOS deliberately does not.
            let bits: libc::c_int = tio::TIOCM_DTR | tio::TIOCM_RTS;
            // `as _` (not a fixed type) on purpose: `libc::ioctl`'s request parameter is
            // `c_ulong` on glibc targets but `c_int` on musl — found by cross-compiling to
            // `x86_64-unknown-linux-musl` via cargo-zigbuild (RELEASE-PLAN.md §3.2), which
            // native compilation on a real (glibc) Linux VM never exercised. `as _` lets the
            // cast target whichever type the platform's `ioctl` signature actually wants.
            if libc::ioctl(self.fd, tio::TIOCMBIS as _, &bits) != 0 {
                // UNVERIFIED: some CDC-ACM implementations reject this. Non-fatal on purpose —
                // if the bootloader turns out to need it, promote to a hard error.
                eprintln!(
                    "warning: could not assert DTR/RTS on {}: {}",
                    self.path.display(),
                    last_os_error()
                );
            }
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Open the bootloader's port, choosing it automatically. See [`discover`].
    pub fn open_bootloader() -> Result<Self, String> {
        let ports = discover()?;
        match ports.len() {
            0 => Err("no bootloader serial port found — run `ktflash unlock` first \
                      (expected /dev/cu.usbmodem* on macOS, /dev/ttyACM* on Linux)"
                .into()),
            1 => Self::open(&ports[0]),
            _ => Err(format!(
                "several candidate serial ports ({}); pass --port to choose one",
                ports.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
            )),
        }
    }

    /// Poll for the bootloader port to appear, then open it.
    ///
    /// `unlock` makes the device re-enumerate, so the node does not exist immediately after —
    /// callers should wait rather than failing on the first miss.
    pub fn wait_for_bootloader(timeout: Duration) -> Result<Self, String> {
        let deadline = Instant::now() + timeout;
        loop {
            match Self::open_bootloader() {
                Ok(t) => return Ok(t),
                Err(e) if Instant::now() >= deadline => {
                    return Err(format!("timed out waiting for the bootloader port: {e}"))
                }
                Err(_) => std::thread::sleep(Duration::from_millis(200)),
            }
        }
    }

    /// Wait for the fd to become readable/writable. Returns false on timeout.
    fn wait(&self, events: libc::c_short, timeout: Duration) -> Result<bool, CdcError> {
        let mut pfd = libc::pollfd { fd: self.fd, events, revents: 0 };
        // poll(2) takes milliseconds as an int; clamp rather than overflow.
        let ms = timeout.as_millis().min(i32::MAX as u128) as libc::c_int;
        loop {
            let n = unsafe { libc::poll(&mut pfd, 1, ms) };
            if n < 0 {
                let e = last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue; // EINTR: retry
                }
                return Err(CdcError::Protocol(format!("poll: {e}")));
            }
            if n == 0 {
                return Ok(false);
            }
            // The device vanishing mid-flash is the case worth naming precisely.
            if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                return Err(CdcError::Disconnected);
            }
            return Ok(true);
        }
    }
}

impl Transport for SerialTransport {
    fn send(&mut self, bytes: &[u8]) -> Result<(), CdcError> {
        let mut off = 0usize;
        while off < bytes.len() {
            if !self.wait(libc::POLLOUT, Duration::from_millis(2000))? {
                return Err(CdcError::Timeout);
            }
            let n = unsafe {
                libc::write(
                    self.fd,
                    bytes[off..].as_ptr() as *const libc::c_void,
                    bytes.len() - off,
                )
            };
            if n < 0 {
                let e = last_os_error();
                match e.kind() {
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock => continue,
                    _ => {
                        return Err(match e.raw_os_error() {
                            Some(libc::ENXIO) | Some(libc::EIO) | Some(libc::ENODEV) => {
                                CdcError::Disconnected
                            }
                            _ => CdcError::Protocol(format!("write: {e}")),
                        })
                    }
                }
            }
            off += n as usize;
        }
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> Result<Vec<u8>, CdcError> {
        if !self.wait(libc::POLLIN, timeout)? {
            return Err(CdcError::Timeout);
        }
        // One read of whatever is buffered. Frame accumulation is the driver's job
        // (proto::ktcdc_driver::Driver::exchange), matching the USB transport's semantics —
        // a tty gives a byte stream, so a single read is never guaranteed to be a whole frame.
        let mut buf = vec![0u8; 4096];
        let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            let e = last_os_error();
            return Err(match e.kind() {
                io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock => CdcError::Timeout,
                _ => match e.raw_os_error() {
                    Some(libc::ENXIO) | Some(libc::EIO) | Some(libc::ENODEV) => {
                        CdcError::Disconnected
                    }
                    _ => CdcError::Protocol(format!("read: {e}")),
                },
            });
        }
        if n == 0 {
            // EOF on a tty after poll said readable means the device went away.
            return Err(CdcError::Disconnected);
        }
        buf.truncate(n as usize);
        Ok(buf)
    }
}

impl Drop for SerialTransport {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

/// Serial ports that plausibly belong to the `8888:cdc0` bootloader.
///
/// On Linux this is exact (sysfs gives us the VID/PID). On macOS it is a heuristic — see
/// [`matches_bootloader`].
pub fn discover() -> Result<Vec<PathBuf>, String> {
    let mut out: Vec<PathBuf> = Vec::new();
    let dir = std::fs::read_dir("/dev").map_err(|e| format!("read /dev: {e}"))?;
    for entry in dir.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_candidate = if cfg!(target_os = "macos") {
            // Callout device, NOT /dev/tty.* — the tty.* node blocks on carrier detect.
            name.starts_with("cu.usbmodem")
        } else {
            name.starts_with("ttyACM")
        };
        if is_candidate && matches_bootloader(&entry.path()) {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

/// Linux: resolve the tty back to its USB device via sysfs and compare VID/PID exactly.
///
/// `/sys/class/tty/<name>/device` is a symlink to the USB *interface*; `..` from there is the
/// USB *device*, which carries `idVendor`/`idProduct`.
#[cfg(target_os = "linux")]
fn matches_bootloader(dev: &Path) -> bool {
    use crate::{BOOT_PID, BOOT_VID};
    fn read_hex(p: &Path) -> Option<u16> {
        let s = std::fs::read_to_string(p).ok()?;
        u16::from_str_radix(s.trim(), 16).ok()
    }
    let Some(name) = dev.file_name() else { return false };
    let base = Path::new("/sys/class/tty").join(name).join("device");
    let vid = read_hex(&base.join("../idVendor"));
    let pid = read_hex(&base.join("../idProduct"));
    vid == Some(BOOT_VID) && pid == Some(BOOT_PID)
}

/// macOS: **heuristic**, and the weakest point in this file.
///
/// UNVERIFIED / TODO for the implementing agent: match properly via IOKit rather than by name.
/// The correct approach is `IOServiceMatching("IOSerialBSDClient")`, read
/// `kIOCalloutDeviceKey` for the `/dev/cu.*` path, then walk `IORegistryEntryGetParentEntry`
/// up the `IOService` plane until a node carries `idVendor`/`idProduct`, and compare against
/// `BOOT_VID`/`BOOT_PID`. Until then this accepts any `/dev/cu.usbmodem*`, and
/// [`SerialTransport::open_bootloader`] refuses when there is more than one candidate rather
/// than guessing — a user with another USB-serial device attached must pass `--port`.
#[cfg(not(target_os = "linux"))]
fn matches_bootloader(_dev: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_never_panics_on_a_normal_dev_tree() {
        // No hardware assumption: /dev always exists, and an empty result is a valid answer.
        let found = discover().expect("reading /dev should succeed");
        for p in &found {
            assert!(p.starts_with("/dev/"), "{p:?} escaped /dev");
        }
    }

    #[test]
    fn opening_a_nonexistent_port_is_an_error_not_a_panic() {
        let e = SerialTransport::open(Path::new("/dev/definitely-not-a-port")).unwrap_err();
        assert!(e.contains("definitely-not-a-port"), "{e}");
    }

    #[test]
    fn open_bootloader_reports_the_unlock_hint_when_nothing_is_attached() {
        // On a dev machine with no dongle this is the message users will actually hit.
        if let Err(e) = SerialTransport::open_bootloader() {
            assert!(
                e.contains("unlock") || e.contains("--port") || e.contains("permission"),
                "unhelpful error: {e}"
            );
        }
    }
}
