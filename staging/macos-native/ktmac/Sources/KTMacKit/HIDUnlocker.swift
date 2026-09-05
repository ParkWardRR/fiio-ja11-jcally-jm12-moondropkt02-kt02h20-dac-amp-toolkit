// HIDUnlocker.swift — experiments E1 and E2 from docs/MACOS-NATIVE.md §4.1, as a library.
//
// ## The question
//
// Two files in this repo disagree about macOS:
//
//   docs/PROTOCOL.md             — "IOHIDDeviceSetReport sends output reports down the *control*
//                                  pipe, which the firmware ignores (it reads interrupt-OUT)."
//   docs/dongle-investigation.md — "Open (shared), GET_REPORT, and SetReport all work natively
//                                  via IOHIDManager — no Windows needed for the transport."
//
// Both can be literally true — the call returns success, the device ignores it — but only one
// can be right about where the bytes land, and the whole native-macOS plan hinges on it. The
// investigation was also run against a KZ C04 (31b2:0313), not a JA11.
//
// ## The answer is binary
//
// The dongle re-enumerates as 8888:cdc0, or it does not. `IOReturn == kIOReturnSuccess` proves
// nothing on its own — that is precisely PROTOCOL.md's claim.
//
// ## Safety
//
// `unlock` is RECOVERABLE. It reboots the dongle into its CDC bootloader; a power-cycle returns
// it to normal mode. It erases nothing. This file sends one 10-byte report and never writes flash.

import Foundation
import IOKit
import IOKit.hid

public struct UnlockAttempt: Sendable {
    public let method: UnlockMethod
    public let includedIDPrefix: Bool
    public let device: KTDevice
    /// What `IOHIDDeviceSetReport` returned. Success here is necessary, not sufficient.
    public let setReportResult: IOReturn
    /// The only result that matters: did the bootloader appear?
    public let bootloaderAppeared: Bool

    public var succeeded: Bool { bootloaderAppeared }

    public var summary: String {
        var s = "method=\(method.rawValue) idPrefix=\(includedIDPrefix) "
        s += "device=\(device.short) setReport=\(describeIOReturn(setReportResult)) "
        s += bootloaderAppeared ? "=> PASS (bootloader appeared)" : "=> FAIL (no re-enumeration)"
        return s
    }
}

public enum HIDUnlocker {
    /// HID devices belonging to a known dongle VID, with the vendor collection preferred.
    ///
    /// - Throws: `KTError.notPermitted` when TCC has not granted Input Monitoring — which, on a
    ///   stock machine, is what happens *before any device is touched*.
    public static func candidateDevices(seize: Bool = false) throws -> [(IOHIDDevice, KTDevice)] {
        let manager = IOHIDManagerCreate(kCFAllocatorDefault, IOOptionBits(kIOHIDOptionsTypeNone))
        IOHIDManagerSetDeviceMatching(manager, nil)  // match all; filter ourselves

        let options = IOOptionBits(seize ? kIOHIDOptionsTypeSeizeDevice : kIOHIDOptionsTypeNone)
        let opened = IOHIDManagerOpen(manager, options)
        if opened == kIOReturnNotPermitted {
            throw KTError.notPermitted("IOHIDManagerOpen")
        }
        guard opened == kIOReturnSuccess else {
            throw KTError.ioKit("IOHIDManagerOpen", opened)
        }

        guard let set = IOHIDManagerCopyDevices(manager) as? Set<IOHIDDevice> else { return [] }

        var out: [(IOHIDDevice, KTDevice)] = []
        for dev in set {
            guard let vid = propertyAsUInt16(IOHIDDeviceGetProperty(dev, kIOHIDVendorIDKey as CFString)),
                KTIDs.knownRuntimeVIDs.contains(vid)
            else { continue }
            let pid =
                propertyAsUInt16(IOHIDDeviceGetProperty(dev, kIOHIDProductIDKey as CFString)) ?? 0
            out.append(
                (dev,
                    KTDevice(
                        vendorID: vid,
                        productID: pid,
                        vendorName: IOHIDDeviceGetProperty(dev, kIOHIDManufacturerKey as CFString)
                            as? String,
                        productName: IOHIDDeviceGetProperty(dev, kIOHIDProductKey as CFString)
                            as? String
                    )))
        }

        // Prefer the vendor collection (0xFF01) — macOS does not always surface it as the
        // *primary* usage page, so this is a sort, not a filter.
        return out.sorted { lhs, _ in
            propertyAsUInt32(IOHIDDeviceGetProperty(lhs.0, kIOHIDPrimaryUsagePageKey as CFString))
                == KTIDs.vendorUsagePage
        }
    }

    /// Attempt the unlock and wait to see whether the device re-enumerates.
    ///
    /// - Parameters:
    ///   - method: `.hid` (E1) or `.hidSeize` (E2).
    ///   - includeIDPrefix: repeat the report ID as byte 0 of the buffer. Report-ID framing is
    ///     the likeliest thing to get wrong, so it is a parameter rather than a decision.
    ///   - dryRun: find and describe the device, send nothing.
    public static func attemptUnlock(
        method: UnlockMethod = .hid,
        includeIDPrefix: Bool = true,
        dryRun: Bool = false,
        watchTimeout: TimeInterval = 5.0,
        log: ((String) -> Void)? = nil
    ) throws -> UnlockAttempt {
        // A device already in bootloader mode makes the result meaningless.
        if USBEnumerator.bootloaderPresent() { throw KTError.alreadyInBootloader }

        let seize = (method == .hidSeize)
        let candidates = try candidateDevices(seize: seize)
        guard let (hidDevice, info) = candidates.first else {
            throw KTError.noDeviceFound("KTMicro/FiiO HID device (is the dongle plugged in?)")
        }
        log?("target: \(info.short) \(info.productName ?? "")")

        var buffer = UnlockCommand.reportBuffer(includeIDPrefix: includeIDPrefix)
        log?(
            "report id=0x\(String(UnlockCommand.reportID, radix: 16)) "
                + "bytes=[\(buffer.map { String(format: "%02x", $0) }.joined(separator: " "))]")

        if dryRun {
            log?("[dry run] nothing sent.")
            return UnlockAttempt(
                method: method, includedIDPrefix: includeIDPrefix, device: info,
                setReportResult: kIOReturnSuccess, bootloaderAppeared: false)
        }

        let openResult = IOHIDDeviceOpen(
            hidDevice, IOOptionBits(seize ? kIOHIDOptionsTypeSeizeDevice : kIOHIDOptionsTypeNone))
        if openResult != kIOReturnSuccess {
            // Not fatal: SetReport sometimes works on a device we could not open exclusively.
            log?("IOHIDDeviceOpen: \(describeIOReturn(openResult)) — continuing anyway")
        }
        defer { IOHIDDeviceClose(hidDevice, IOOptionBits(kIOHIDOptionsTypeNone)) }

        let setResult = IOHIDDeviceSetReport(
            hidDevice, kIOHIDReportTypeOutput, CFIndex(UnlockCommand.reportID), &buffer,
            buffer.count)
        log?("IOHIDDeviceSetReport: \(describeIOReturn(setResult))")

        log?("watching for 8888:cdc0 (\(Int(watchTimeout))s)…")
        let appeared = USBEnumerator.waitForBootloader(timeout: watchTimeout)

        return UnlockAttempt(
            method: method, includedIDPrefix: includeIDPrefix, device: info,
            setReportResult: setResult, bootloaderAppeared: appeared)
    }

    /// Try the plausible framings in order and stop at the first that re-enumerates.
    ///
    /// Cheaper than reasoning about which one is right: each attempt is recoverable, and a pass
    /// is unambiguous. Only used when the caller explicitly asks (`--auto`).
    public static func attemptAll(
        watchTimeout: TimeInterval = 5.0, log: ((String) -> Void)? = nil
    ) -> [UnlockAttempt] {
        var results: [UnlockAttempt] = []
        for method in [UnlockMethod.hid, .hidSeize] {
            for prefix in [true, false] {
                log?("\n--- \(method.rawValue), idPrefix=\(prefix) ---")
                do {
                    let r = try attemptUnlock(
                        method: method, includeIDPrefix: prefix, watchTimeout: watchTimeout,
                        log: log)
                    results.append(r)
                    if r.succeeded { return results }
                } catch {
                    log?("skipped: \(error)")
                }
            }
        }
        return results
    }
}
