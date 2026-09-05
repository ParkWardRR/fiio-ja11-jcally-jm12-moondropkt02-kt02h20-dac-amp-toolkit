// SerialPortFinder.swift — resolve /dev/cu.* back to the USB device that owns it.
//
// ## Why this file is the most useful thing in the package
//
// `ktflash`'s serial transport (staging/flasher-src/serialtransport.rs) can match a tty to the
// bootloader exactly on Linux, via sysfs:
//
//     /sys/class/tty/ttyACM0/device/../idVendor
//
// On macOS it currently cannot, so it falls back to "accept any /dev/cu.usbmodem*" and refuses
// when there is more than one candidate. That is a real limitation: a user with any other
// USB-serial device attached has to pass --port by hand, and worse, a wrong guess would aim a
// firmware write at the wrong device.
//
// This is the fix, and the reference implementation for porting it into Rust:
// enumerate `IOSerialBSDClient` services, read the callout path from each, and use
// `IORegistryEntrySearchCFProperty` with `kIORegistryIterateParents` to pull `idVendor` /
// `idProduct` down from the owning `IOUSBHostDevice` several levels up the IOService plane.

import Foundation
import IOKit
import IOKit.serial

public enum SerialPortFinder {
    /// Every serial port on the system, with its owning USB device's IDs where they exist.
    ///
    /// Ports that are not USB-backed (Bluetooth, virtual) come back with nil IDs rather than
    /// being dropped, so `ktmac ports` can show the user everything and explain the mismatch.
    public static func allPorts() -> [KTSerialPort] {
        guard let base = IOServiceMatching(kIOSerialBSDServiceValue) else { return [] }
        let dict = base as NSMutableDictionary
        // Without this the match returns nothing on some systems.
        dict[kIOSerialBSDTypeKey] = kIOSerialBSDAllTypes

        var out: [KTSerialPort] = []
        _ = forEachService(matching: dict as CFMutableDictionary) { service in
            guard
                let callout = propertyAsString(registryProperty(service, kIOCalloutDeviceKey))
            else { return }
            out.append(
                KTSerialPort(
                    calloutPath: callout,
                    dialinPath: propertyAsString(registryProperty(service, kIODialinDeviceKey)),
                    // The parent walk: these live on the USB device, not on the serial client.
                    vendorID: propertyAsUInt16(inheritedProperty(service, "idVendor")),
                    productID: propertyAsUInt16(inheritedProperty(service, "idProduct")),
                    bsdName: propertyAsString(registryProperty(service, "IOTTYDevice"))
                ))
        }
        return out.sorted { $0.calloutPath < $1.calloutPath }
    }

    /// The bootloader's port, matched on VID **and** PID — never on the name.
    ///
    /// Returns more than one only if two bootloaders are genuinely attached, in which case the
    /// caller must ask rather than guess: picking the wrong one aims a firmware write at the
    /// wrong device.
    public static func bootloaderPorts() -> [KTSerialPort] {
        allPorts().filter(\.isBootloader)
    }

    /// Wait for the bootloader's serial port to appear and return its callout path.
    ///
    /// `unlock` makes the device re-enumerate, and the tty node is published a moment after the
    /// USB device itself, so callers must wait rather than failing on the first miss.
    public static func waitForBootloaderPort(timeout: TimeInterval) -> KTSerialPort? {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            let ports = bootloaderPorts()
            if ports.count == 1 { return ports[0] }
            if ports.count > 1 { return nil }  // ambiguous — let the caller decide
            Thread.sleep(forTimeInterval: 0.2)
        } while Date() < deadline
        return nil
    }

    /// Ports that *look* like USB modems but could not be resolved to a USB device.
    ///
    /// Diagnostic: if the bootloader's port shows up here, the parent walk failed and the Rust
    /// port of this logic would fall back to guessing — worth knowing before that happens
    /// during a flash.
    public static func unresolvedUSBModemPorts() -> [KTSerialPort] {
        allPorts().filter {
            $0.vendorID == nil && $0.calloutPath.contains("usbmodem")
        }
    }
}
