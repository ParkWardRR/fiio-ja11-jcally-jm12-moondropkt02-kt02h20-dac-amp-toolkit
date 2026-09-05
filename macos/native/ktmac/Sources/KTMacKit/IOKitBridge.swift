// IOKitBridge.swift — the small, awkward part of talking to the IORegistry from Swift,
// isolated so the rest of KTMacKit reads like ordinary code.

import Foundation
import IOKit
import IOKit.serial
import IOKit.usb

/// Iterate an `io_iterator_t`, releasing every entry, and always releasing the iterator.
///
/// Forgetting one of those releases is the classic IOKit leak; doing it once here means the
/// call sites cannot get it wrong.
func forEachService(matching: CFMutableDictionary, _ body: (io_object_t) -> Void) -> IOReturn {
    var iterator: io_iterator_t = 0
    // IOServiceGetMatchingServices consumes a reference to `matching`.
    let kr = IOServiceGetMatchingServices(kIOMainPortDefault, matching, &iterator)
    guard kr == KERN_SUCCESS else { return kr }
    defer { IOObjectRelease(iterator) }

    while case let service = IOIteratorNext(iterator), service != 0 {
        body(service)
        IOObjectRelease(service)
    }
    return KERN_SUCCESS
}

/// Read a property directly off one registry entry.
func registryProperty(_ entry: io_registry_entry_t, _ key: String) -> Any? {
    IORegistryEntryCreateCFProperty(entry, key as CFString, kCFAllocatorDefault, 0)?
        .takeRetainedValue()
}

/// Read a property from an entry **or any of its ancestors**.
///
/// This is the whole trick behind resolving a serial port to its USB device: a
/// `/dev/cu.usbmodem*` node is published by `IOSerialBSDClient`, several levels below the
/// `IOUSBHostDevice` that actually carries `idVendor`/`idProduct`. Rather than walking parents
/// by hand, `IORegistryEntrySearchCFProperty` does it with `kIORegistryIterateParents`.
func inheritedProperty(_ entry: io_registry_entry_t, _ key: String) -> Any? {
    let options = IOOptionBits(kIORegistryIterateRecursively | kIORegistryIterateParents)
    return IORegistryEntrySearchCFProperty(
        entry, kIOServicePlane, key as CFString, kCFAllocatorDefault, options)
}

func propertyAsUInt16(_ value: Any?) -> UInt16? {
    guard let n = value as? NSNumber else { return nil }
    let i = n.intValue
    guard i >= 0, i <= Int(UInt16.max) else { return nil }
    return UInt16(i)
}

func propertyAsUInt32(_ value: Any?) -> UInt32? {
    guard let n = value as? NSNumber else { return nil }
    let i = n.int64Value
    guard i >= 0, i <= Int64(UInt32.max) else { return nil }
    return UInt32(i)
}

func propertyAsString(_ value: Any?) -> String? {
    if let s = value as? String { return s }
    if let d = value as? Data { return String(data: d, encoding: .utf8) }
    return nil
}

/// Build a matching dictionary for a USB device class, optionally pinned to a VID/PID.
func usbMatchingDictionary(vendorID: UInt16? = nil, productID: UInt16? = nil)
    -> CFMutableDictionary?
{
    // "IOUSBHostDevice" is the modern class; "IOUSBDevice" is the legacy name that still
    // matches on older systems. IOUSBHostDevice is correct for macOS 10.11+.
    guard let dict = IOServiceMatching("IOUSBHostDevice") else { return nil }
    let mutable = dict as NSMutableDictionary
    if let vid = vendorID { mutable["idVendor"] = NSNumber(value: vid) }
    if let pid = productID { mutable["idProduct"] = NSNumber(value: pid) }
    return mutable as CFMutableDictionary
}

/// Human-readable IOReturn, because `0xe00002e2` means nothing at a glance.
public func describeIOReturn(_ code: IOReturn) -> String {
    let hex = String(format: "0x%08x", UInt32(bitPattern: code))
    let name: String
    switch code {
    case kIOReturnSuccess: name = "success"
    case kIOReturnNotPermitted: name = "not permitted (TCC — grant Input Monitoring)"
    case kIOReturnNoDevice: name = "no device"
    case kIOReturnExclusiveAccess: name = "exclusive access (another driver owns it)"
    case kIOReturnNotOpen: name = "not open"
    case kIOReturnUnsupported: name = "unsupported"
    case kIOReturnNotPrivileged: name = "not privileged (try sudo)"
    case kIOReturnBadArgument: name = "bad argument"
    default: name = "unknown"
    }
    return "\(hex) (\(name))"
}
