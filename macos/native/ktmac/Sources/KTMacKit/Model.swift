// Model.swift — the identities and value types the rest of KTMacKit works in.
//
// Everything here is pure data with no IOKit in sight, so it unit-tests without hardware.

import Foundation

/// USB identities from docs/PROTOCOL.md and docs/COMPATIBILITY.md.
public enum KTIDs {
    /// KTMicro — the stock dongles (JA11 runtime PID is 0x0111).
    public static let ktmicroVID: UInt16 = 0x31b2
    /// FiiO / JadeAudio — an already-cross-flashed JA11.
    public static let fiioVID: UInt16 = 0x2972
    public static let fiioJA11PID: UInt16 = 0x0102

    /// KT_USB_BOOT CDC bootloader, after `unlock`.
    ///
    /// 0x8888 is a squatted/placeholder vendor ID, so it is only ever matched together with
    /// its product ID — never on the VID alone.
    public static let bootVID: UInt16 = 0x8888
    public static let bootPID: UInt16 = 0xcdc0

    /// The vendor HID collection carrying the 0x4B/0x54 command reports.
    public static let vendorUsagePage: UInt32 = 0xff01

    public static let knownRuntimeVIDs: [UInt16] = [ktmicroVID, fiioVID]
}

/// The unlock command: report `0x54` + `"12345678"` + NUL.
///
/// On the wire: `54 31 32 33 34 35 36 37 38 00` (docs/PROTOCOL.md, from `FUN_006260b0`).
public enum UnlockCommand {
    public static let reportID: UInt8 = 0x54
    /// `"12345678\0"` — the payload after the report ID.
    public static let payload: [UInt8] = [0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x00]

    /// The full 10-byte wire form, report ID included.
    public static var wireBytes: [UInt8] { [reportID] + payload }

    /// The buffer to hand `IOHIDDeviceSetReport`.
    ///
    /// Whether IOKit wants the report ID repeated as byte 0 when it is also passed as the
    /// `reportID` argument is genuinely ambiguous across drivers — hidapi passes it, some
    /// stacks do not. Rather than argue, both forms are testable (`--no-id-prefix`).
    public static func reportBuffer(includeIDPrefix: Bool) -> [UInt8] {
        includeIDPrefix ? wireBytes : payload
    }
}

/// A USB device as seen in the IORegistry.
public struct KTDevice: Codable, Sendable, Equatable {
    public let vendorID: UInt16
    public let productID: UInt16
    public let vendorName: String?
    public let productName: String?
    public let serialNumber: String?
    public let locationID: UInt32?

    public init(
        vendorID: UInt16, productID: UInt16, vendorName: String? = nil,
        productName: String? = nil, serialNumber: String? = nil, locationID: UInt32? = nil
    ) {
        self.vendorID = vendorID
        self.productID = productID
        self.vendorName = vendorName
        self.productName = productName
        self.serialNumber = serialNumber
        self.locationID = locationID
    }

    /// `"31b2:0111"`, matching ktflash's own `DeviceFingerprint::short()`.
    public var short: String { String(format: "%04x:%04x", vendorID, productID) }

    public var isBootloader: Bool {
        vendorID == KTIDs.bootVID && productID == KTIDs.bootPID
    }

    public var isKnownRuntimeDongle: Bool {
        KTIDs.knownRuntimeVIDs.contains(vendorID)
    }
}

/// A serial port, resolved back to the USB device that owns it.
public struct KTSerialPort: Codable, Sendable, Equatable {
    /// `/dev/cu.usbmodemXXXX` — the callout device.
    public let calloutPath: String
    /// `/dev/tty.usbmodemXXXX` — present for completeness; **do not open it**, see `isSafeToOpen`.
    public let dialinPath: String?
    public let vendorID: UInt16?
    public let productID: UInt16?
    public let bsdName: String?

    public init(
        calloutPath: String, dialinPath: String? = nil, vendorID: UInt16? = nil,
        productID: UInt16? = nil, bsdName: String? = nil
    ) {
        self.calloutPath = calloutPath
        self.dialinPath = dialinPath
        self.vendorID = vendorID
        self.productID = productID
        self.bsdName = bsdName
    }

    /// Is this the `8888:cdc0` bootloader's port?
    ///
    /// This is the question `ktflash`'s `serialtransport.rs` currently cannot answer on macOS —
    /// it falls back to "any /dev/cu.usbmodem*". Matching on the resolved VID/PID is the fix.
    public var isBootloader: Bool {
        vendorID == KTIDs.bootVID && productID == KTIDs.bootPID
    }
}

/// Which mechanism to try for `unlock`. See docs/MACOS-NATIVE.md §4.1.
public enum UnlockMethod: String, Sendable, CaseIterable {
    /// E1 — `IOHIDDeviceSetReport` through IOHIDManager.
    case hid
    /// E2 — same, but opening with `kIOHIDOptionsTypeSeizeDevice`.
    case hidSeize = "hid-seize"
}

public enum KTError: Error, CustomStringConvertible {
    case noDeviceFound(String)
    case ioKit(String, IOReturn)
    case notPermitted(String)
    case alreadyInBootloader

    public var description: String {
        switch self {
        case .noDeviceFound(let what):
            return "no \(what) found"
        case .ioKit(let what, let code):
            return String(format: "%@: IOReturn 0x%08x", what, UInt32(bitPattern: code))
        case .notPermitted(let what):
            return """
                \(what): kIOReturnNotPermitted.

                This is macOS TCC, not a USB problem. Since macOS 10.15, opening an IOHIDManager
                requires Input Monitoring consent for the calling application — for a CLI, that
                means the terminal you launched it from.

                  System Settings > Privacy & Security > Input Monitoring > +  (add your terminal)

                Then restart the terminal and try again.
                """
        case .alreadyInBootloader:
            return """
                the dongle is already in bootloader mode (8888:cdc0).

                Power-cycle it (unplug/replug) before running an unlock experiment, or the
                result tells you nothing.
                """
        }
    }
}
