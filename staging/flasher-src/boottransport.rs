//! Picks how to talk to the `8888:cdc0` bootloader: the CDC tty, or libusb bulk endpoints.
//!
//! Two transports implement [`crate::proto::cdc::Transport`]:
//!
//! | | reaches the device via | works on |
//! |---|---|---|
//! | [`crate::serialtransport::SerialTransport`] | the kernel's CDC-ACM tty | macOS **and** Linux |
//! | [`crate::usbtransport::RusbBootloaderTransport`] | claiming the USB interface | Linux only |
//!
//! Serial is preferred by default on both platforms: on macOS it is the only thing that works
//! without OrbStack, and on Linux it avoids the interface claim, the `cdc_acm` detach, and the
//! ModemManager race entirely (`docs/MACOS-NATIVE.md` §3.4).
//!
//! > **Status: untested.** The preference order below is a proposal. If hardware testing shows
//! > the libusb path is more reliable on Linux, flip [`Preference::default`] — that is the only
//! > line that needs to change.

use std::time::Duration;

use crate::proto::cdc::Transport;
use crate::serialtransport::SerialTransport;
use crate::usbtransport::RusbBootloaderTransport;

/// Which transport to use, from `--transport`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preference {
    /// Try serial, fall back to libusb. Flip this default if hardware testing shows libusb is
    /// more reliable on Linux.
    #[default]
    Auto,
    /// Serial only; fail if the tty is not usable.
    Serial,
    /// libusb only; fail if the interface cannot be claimed.
    Usb,
}

impl std::str::FromStr for Preference {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "auto" => Ok(Preference::Auto),
            "serial" | "tty" | "cdc" => Ok(Preference::Serial),
            "usb" | "libusb" | "bulk" => Ok(Preference::Usb),
            other => Err(format!("unknown --transport {other} (want: auto | serial | usb)")),
        }
    }
}

/// How long to wait for the bootloader's tty to appear. `unlock` re-enumerates the device, so
/// the node does not exist the instant the command returns.
const PORT_WAIT: Duration = Duration::from_millis(3000);

/// Open a transport to the bootloader.
///
/// `port` forces a specific tty (e.g. `/dev/cu.usbmodem1101`) and implies serial.
pub fn open(pref: Preference, port: Option<&str>) -> Result<(Box<dyn Transport>, String), String> {
    if let Some(p) = port {
        let t = SerialTransport::open(std::path::Path::new(p))?;
        let label = format!("serial {p}");
        return Ok((Box::new(t), label));
    }

    match pref {
        Preference::Serial => {
            let t = SerialTransport::wait_for_bootloader(PORT_WAIT)?;
            let label = format!("serial {}", t.path().display());
            Ok((Box::new(t), label))
        }
        Preference::Usb => {
            let t = RusbBootloaderTransport::open()?;
            Ok((Box::new(t), "libusb bulk".to_string()))
        }
        Preference::Auto => {
            let serial_err = match SerialTransport::wait_for_bootloader(PORT_WAIT) {
                Ok(t) => {
                    let label = format!("serial {}", t.path().display());
                    return Ok((Box::new(t), label));
                }
                Err(e) => e,
            };
            match RusbBootloaderTransport::open() {
                Ok(t) => Ok((Box::new(t), "libusb bulk".to_string())),
                Err(usb_err) => Err(format!(
                    "no usable transport to the bootloader.\n  serial: {serial_err}\n  libusb: {usb_err}\n\
                     Hint: run `ktflash unlock` immediately before flashing — the bootloader is one-shot."
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_names_parse() {
        assert_eq!("auto".parse::<Preference>().unwrap(), Preference::Auto);
        assert_eq!("serial".parse::<Preference>().unwrap(), Preference::Serial);
        assert_eq!("tty".parse::<Preference>().unwrap(), Preference::Serial);
        assert_eq!("usb".parse::<Preference>().unwrap(), Preference::Usb);
        assert_eq!(Preference::default(), Preference::Auto);
    }

    #[test]
    fn an_unknown_transport_name_lists_the_valid_ones() {
        let e = "smoke-signals".parse::<Preference>().unwrap_err();
        assert!(e.contains("auto") && e.contains("serial") && e.contains("usb"), "{e}");
    }
}
