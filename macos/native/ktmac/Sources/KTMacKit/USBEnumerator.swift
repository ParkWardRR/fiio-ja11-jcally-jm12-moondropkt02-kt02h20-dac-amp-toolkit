// USBEnumerator.swift — what USB devices are attached, and is the bootloader up?
//
// Enumeration is the one USB thing macOS has never blocked (only *claiming* is), so this is
// reliable and needs no entitlement, no TCC consent, and no root.

import Foundation
import IOKit
import IOKit.usb

public enum USBEnumerator {
    /// Every USB device currently attached.
    public static func allDevices() -> [KTDevice] {
        guard let matching = usbMatchingDictionary() else { return [] }
        var out: [KTDevice] = []
        _ = forEachService(matching: matching) { service in
            guard let vid = propertyAsUInt16(registryProperty(service, "idVendor")),
                let pid = propertyAsUInt16(registryProperty(service, "idProduct"))
            else { return }
            out.append(
                KTDevice(
                    vendorID: vid,
                    productID: pid,
                    vendorName: propertyAsString(registryProperty(service, "USB Vendor Name")),
                    productName: propertyAsString(registryProperty(service, "USB Product Name")),
                    serialNumber: propertyAsString(
                        registryProperty(service, "USB Serial Number")),
                    locationID: propertyAsUInt32(registryProperty(service, "locationID"))
                ))
        }
        return out
    }

    /// KTMicro / FiiO dongles and the bootloader — i.e. anything this toolkit cares about.
    public static func ktDevices() -> [KTDevice] {
        allDevices().filter { $0.isKnownRuntimeDongle || $0.isBootloader }
    }

    /// Is the `8888:cdc0` bootloader present?
    ///
    /// This is the pass/fail signal for every unlock experiment: it either re-enumerates or it
    /// does not. Matched on VID **and** PID together, because 0x8888 is a squatted vendor ID.
    public static func bootloaderPresent() -> Bool {
        guard
            let matching = usbMatchingDictionary(
                vendorID: KTIDs.bootVID, productID: KTIDs.bootPID)
        else { return false }
        var found = false
        _ = forEachService(matching: matching) { _ in found = true }
        return found
    }

    /// Poll for the bootloader to appear.
    ///
    /// Used after an unlock attempt: the device drops off the bus and comes back under a new
    /// VID:PID, which takes a moment.
    ///
    /// - Returns: `true` if it showed up before the deadline.
    public static func waitForBootloader(
        timeout: TimeInterval, pollInterval: TimeInterval = 0.2,
        onPoll: ((TimeInterval) -> Void)? = nil
    ) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if bootloaderPresent() { return true }
            onPoll?(deadline.timeIntervalSinceNow)
            Thread.sleep(forTimeInterval: pollInterval)
        }
        return bootloaderPresent()
    }

    /// Wait for the dongle to come back in *normal* mode — the post-flash confirmation step.
    ///
    /// `ktflash`'s journal stays at `reset-issued` until something observes this, so it is
    /// worth having natively.
    public static func waitForRuntimeDongle(timeout: TimeInterval) -> KTDevice? {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            if let d = ktDevices().first(where: { $0.isKnownRuntimeDongle }) { return d }
            Thread.sleep(forTimeInterval: 0.2)
        } while Date() < deadline
        return nil
    }
}
