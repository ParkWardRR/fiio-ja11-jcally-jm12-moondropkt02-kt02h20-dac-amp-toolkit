// SelfTest.swift — the hardware-free checks, as a plain runnable function.
//
// ## Why not XCTest or swift-testing
//
// Neither is available with only the Command Line Tools installed, and that is the common case
// on a machine that is not doing iOS/macOS app development. Requiring a full Xcode.app to run
// the checks would mean they mostly don't get run, which defeats the point. So they are a
// function in the shipping library, invoked by `ktmac selftest`, using nothing but Foundation.
//
// The trade-off is real — no parallelism, no per-test isolation, no XML output — but these are
// pure assertions over constants and read-only IOKit queries, so none of that buys much. If a
// full Xcode is available later, porting them is mechanical.
//
// None of this touches hardware: no reports are sent and nothing is opened for writing.

import Foundation

public struct SelfTestResults: Sendable {
    public var passed: [String] = []
    public var failed: [(name: String, detail: String)] = []
    public var skipped: [(name: String, reason: String)] = []

    public var ok: Bool { failed.isEmpty }
    public var summary: String {
        "\(passed.count) passed, \(failed.count) failed, \(skipped.count) skipped"
    }
}

public enum SelfTest {
    /// Run every hardware-free check. `log` receives one line per check as it runs.
    public static func run(log: ((String) -> Void)? = nil) -> SelfTestResults {
        var r = SelfTestResults()

        func check(_ name: String, _ body: () -> String?) {
            if let failure = body() {
                r.failed.append((name, failure))
                log?("  ✗ \(name): \(failure)")
            } else {
                r.passed.append(name)
                log?("  ✓ \(name)")
            }
        }
        func skip(_ name: String, _ reason: String) {
            r.skipped.append((name, reason))
            log?("  – \(name) (skipped: \(reason))")
        }

        log?("protocol constants")

        check("unlock wire bytes match docs/PROTOCOL.md") {
            // Report 0x54 + "12345678" + 0x00. Getting this wrong is the difference between an
            // unlock and a silent no-op, so it is pinned to the documented bytes.
            let want: [UInt8] = [0x54, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x00]
            guard UnlockCommand.wireBytes == want else {
                return "got \(UnlockCommand.wireBytes.map { String(format: "%02x", $0) })"
            }
            guard String(bytes: UnlockCommand.payload.dropLast(), encoding: .ascii) == "12345678"
            else { return "payload is not the ASCII digits" }
            guard UnlockCommand.payload.last == 0x00 else { return "missing the trailing NUL" }
            return nil
        }

        check("both report-ID framings differ only in the prefix") {
            let withID = UnlockCommand.reportBuffer(includeIDPrefix: true)
            let without = UnlockCommand.reportBuffer(includeIDPrefix: false)
            guard withID.count == 10, without.count == 9 else { return "wrong lengths" }
            guard withID.first == 0x54, without.first == 0x31 else { return "wrong first byte" }
            guard Array(withID.dropFirst()) == without else { return "framings disagree" }
            return nil
        }

        log?("identity")

        check("bootloader identity requires BOTH vid and pid") {
            // 0x8888 is a squatted vendor ID: a VID-only match could catch an unrelated device
            // and aim a firmware write at it.
            guard KTDevice(vendorID: 0x8888, productID: 0xcdc0).isBootloader else {
                return "the real bootloader was not recognised"
            }
            guard !KTDevice(vendorID: 0x8888, productID: 0x1234).isBootloader else {
                return "matched 0x8888 on the VID alone — dangerous"
            }
            guard !KTDevice(vendorID: 0x31b2, productID: 0xcdc0).isBootloader else {
                return "matched the PID alone"
            }
            return nil
        }

        check("short() matches ktflash's DeviceFingerprint format") {
            // Must agree with Rust's DeviceFingerprint::short() so records line up across tools.
            guard KTDevice(vendorID: 0x31b2, productID: 0x0111).short == "31b2:0111",
                KTDevice(vendorID: 0x0001, productID: 0x0002).short == "0001:0002"
            else { return "format drifted from vvvv:pppp" }
            return nil
        }

        check("runtime dongle recognition is neither too narrow nor too wide") {
            guard KTDevice(vendorID: 0x31b2, productID: 0x0111).isKnownRuntimeDongle,
                KTDevice(vendorID: 0x2972, productID: 0x0102).isKnownRuntimeDongle
            else { return "a known dongle was not recognised" }
            guard !KTDevice(vendorID: 0x05ac, productID: 0x0001).isKnownRuntimeDongle,
                !KTDevice(vendorID: 0x8888, productID: 0xcdc0).isKnownRuntimeDongle
            else { return "recognised something that is not a runtime dongle" }
            return nil
        }

        check("an unresolved serial port never looks like the flash target") {
            let unresolved = KTSerialPort(calloutPath: "/dev/cu.usbmodemXYZ")
            guard !unresolved.isBootloader else {
                return "a port with no resolved VID/PID claimed to be the bootloader"
            }
            let boot = KTSerialPort(
                calloutPath: "/dev/cu.usbmodem1101", vendorID: 0x8888, productID: 0xcdc0)
            guard boot.isBootloader else { return "the real bootloader port was not recognised" }
            return nil
        }

        log?("ktflash invocation")

        check("a dry run never carries --execute or --yes") {
            // The single most important assertion in this file: `flow` runs a dry run first,
            // and if that invocation could ever write, the "nothing is touched" step would be
            // a lie.
            let dry = FlashCdcInvocation(imagePath: "/tmp/fw.bin")
            let args = dry.arguments
            guard !args.contains("--execute") else { return "dry run contained --execute" }
            guard !args.contains("--yes") else { return "dry run contained --yes" }
            guard args.first == "flash-cdc", args.contains("--image") else {
                return "malformed argv: \(args)"
            }
            return nil
        }

        check("--yes only ever appears alongside --execute") {
            // --yes is the "I have a known-good image saved" acknowledgement. It must never
            // ride along on a command that was not already going to write.
            let write = FlashCdcInvocation(imagePath: "/tmp/fw.bin", execute: true)
            let args = write.arguments
            guard args.contains("--execute"), args.contains("--yes") else {
                return "an executing invocation is missing --execute/--yes"
            }
            guard let e = args.firstIndex(of: "--execute"), let y = args.firstIndex(of: "--yes"),
                y == e + 1
            else { return "--yes is not adjacent to --execute" }
            return nil
        }

        check("a port implies the serial transport") {
            let inv = FlashCdcInvocation(imagePath: "/tmp/fw.bin", port: "/dev/cu.usbmodem1101")
            let args = inv.arguments
            guard let t = args.firstIndex(of: "--transport"), args[t + 1] == "serial" else {
                return "passing a port did not select the serial transport: \(args)"
            }
            guard let p = args.firstIndex(of: "--port"), args[p + 1] == "/dev/cu.usbmodem1101"
            else { return "port not passed through: \(args)" }
            return nil
        }

        check("--expect is passed through verbatim") {
            let inv = FlashCdcInvocation(imagePath: "/tmp/fw.bin", expect: "2972:0102")
            let args = inv.arguments
            guard let i = args.firstIndex(of: "--expect"), args[i + 1] == "2972:0102" else {
                return "expectation not passed through: \(args)"
            }
            return nil
        }

        check("the printed command line is the command actually run") {
            // `flow` shows the operator what it ran; if the two diverged, the audit trail for a
            // destructive operation would be wrong.
            let inv = FlashCdcInvocation(
                imagePath: "/tmp/my firmware.bin", port: "/dev/cu.usbmodem1101", execute: true)
            let line = inv.commandLine(binary: "/usr/local/bin/ktflash")
            for arg in inv.arguments where !arg.contains(" ") {
                guard line.contains(arg) else { return "\(arg) missing from: \(line)" }
            }
            guard line.contains("'/tmp/my firmware.bin'") else {
                return "a path with a space was not quoted: \(line)"
            }
            return nil
        }

        check("`which` finds a binary that exists and not one that does not") {
            guard ProcessRunner.which("ls") != nil else { return "could not find ls" }
            guard ProcessRunner.which("definitely-not-a-real-binary-xyz") == nil else {
                return "found a binary that does not exist"
            }
            return nil
        }

        check("ProcessRunner captures output and exit codes") {
            guard let r = try? ProcessRunner.run("/bin/echo", ["hello"]) else {
                return "running /bin/echo threw"
            }
            guard r.ok, r.stdout.contains("hello") else { return "bad result: \(r)" }
            guard let fail = try? ProcessRunner.run("/bin/sh", ["-c", "exit 3"]), fail.exitCode == 3
            else { return "non-zero exit code not propagated" }
            return nil
        }

        log?("diagnostics")

        check("IOReturn descriptions name the codes we actually hit") {
            guard describeIOReturn(kIOReturnNotPermitted).contains("Input Monitoring"),
                describeIOReturn(kIOReturnExclusiveAccess).contains("exclusive"),
                describeIOReturn(kIOReturnNotPrivileged).contains("sudo")
            else { return "a diagnostic lost its explanation" }
            return nil
        }

        check("the TCC error explains itself instead of printing a hex code") {
            // This is the first wall anyone hits. "0xe00002e2" alone sends people down a USB
            // rabbit hole when the actual problem is a privacy setting.
            let text = String(describing: KTError.notPermitted("IOHIDManagerOpen"))
            guard text.contains("Input Monitoring"), text.contains("System Settings"),
                text.contains("TCC")
            else { return "the TCC error is not actionable" }
            return nil
        }

        log?("live IOKit (read-only)")

        check("USB enumeration works and returns plausible records") {
            // Every Mac has USB devices in the IORegistry. This asserts the IOKit plumbing
            // works, not that a dongle is attached.
            let all = USBEnumerator.allDevices()
            guard !all.isEmpty else { return "no USB devices found at all — IOKit plumbing broken?" }
            if let bad = all.first(where: { $0.short.count != 9 }) {
                return "malformed short id: \(bad.short)"
            }
            return nil
        }

        check("the KT device filter is a strict subset of all devices") {
            let all = Set(USBEnumerator.allDevices().map(\.short))
            for d in USBEnumerator.ktDevices() where !all.contains(d.short) {
                return "\(d.short) is in ktDevices() but not allDevices()"
            }
            return nil
        }

        check("bootloaderPresent() agrees with the device list") {
            // Two independent code paths — a targeted matching dictionary vs. filtering the
            // full list — must agree. Passes whether or not a dongle is attached.
            let viaList = USBEnumerator.ktDevices().contains(where: \.isBootloader)
            guard USBEnumerator.bootloaderPresent() == viaList else {
                return "matching dictionary and list filter disagree"
            }
            return nil
        }

        check("serial ports resolve to callout nodes, never /dev/tty.*") {
            for p in SerialPortFinder.allPorts() {
                guard p.calloutPath.hasPrefix("/dev/") else {
                    return "path escaped /dev: \(p.calloutPath)"
                }
                // /dev/tty.* blocks on carrier detect; opening it is a classic hang.
                guard !p.calloutPath.hasPrefix("/dev/tty.") else {
                    return "\(p.calloutPath) is a dial-in node"
                }
            }
            return nil
        }

        check("bootloader ports are a subset of all ports") {
            let all = Set(SerialPortFinder.allPorts().map(\.calloutPath))
            for p in SerialPortFinder.bootloaderPorts() where !all.contains(p.calloutPath) {
                return "\(p.calloutPath) is in bootloaderPorts() but not allPorts()"
            }
            return nil
        }

        // The whole point of SerialPortFinder is the IORegistryEntrySearchCFProperty parent
        // walk. If it silently returned nil for everything, every other check here would still
        // pass while the module was useless — so assert it resolves something when there is
        // anything to resolve.
        let usbish = SerialPortFinder.allPorts().filter {
            $0.calloutPath.contains("usbmodem") || $0.calloutPath.contains("usbserial")
        }
        if usbish.isEmpty {
            skip("USB parent walk resolves VID/PID", "no USB serial devices attached")
        } else {
            check("USB parent walk resolves VID/PID") {
                guard usbish.contains(where: { $0.vendorID != nil }) else {
                    return "resolved no VID for any of \(usbish.map(\.calloutPath)) — "
                        + "IORegistryEntrySearchCFProperty is not finding idVendor"
                }
                return nil
            }
        }

        return r
    }
}
