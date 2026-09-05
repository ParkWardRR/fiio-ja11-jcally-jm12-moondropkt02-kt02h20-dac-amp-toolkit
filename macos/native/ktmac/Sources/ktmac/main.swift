// ktmac — macOS-native helper for ktflash (docs/MACOS-NATIVE.md).
//
//   ktmac list                    USB devices this toolkit cares about
//   ktmac ports                   serial ports, resolved to their USB owner
//   ktmac port                    just the bootloader's /dev/cu.* path (for scripts)
//   ktmac unlock                  dry run: find the device, send nothing
//   ktmac unlock --send           attempt the unlock (RECOVERABLE — power-cycle undoes it)
//   ktmac watch                   wait for the bootloader to appear
//
//   --json      machine-readable output (for ktflash to consume)
//
// Hand-rolled argument parsing, on purpose: this package has no third-party dependencies.
//
// STATUS: UNTESTED against hardware.

import Foundation
import KTMacKit

let version = "0.1.0-untested"

func die(_ message: String, code: Int32 = 1) -> Never {
    FileHandle.standardError.write(Data(("ktmac: " + message + "\n").utf8))
    exit(code)
}

func emitJSON<T: Encodable>(_ value: T) {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    guard let data = try? encoder.encode(value), let s = String(data: data, encoding: .utf8) else {
        die("could not encode JSON")
    }
    print(s)
}

let usage = """
    ktmac \(version) — macOS-native helper for ktflash

    USAGE:
      ktmac list                     USB devices (KTMicro / FiiO / bootloader)
      ktmac ports                    serial ports with their resolved USB VID:PID
      ktmac port                     print the bootloader's /dev/cu.* path only
      ktmac watch [--timeout N]      wait for the 8888:cdc0 bootloader
      ktmac unlock [options]         attempt the HID unlock
      ktmac doctor                   check everything the native flow needs
      ktmac flow --image fw.bin      the whole native flash, no OrbStack
      ktmac selftest                 hardware-free checks (no dongle needed)

    FLOW OPTIONS:
      --image PATH        firmware image (required)
      --execute           actually unlock and write (default is a dry run)
      --yes               skip the interactive FLASH confirmation
      --expect VID:PID    require this identity after the reset, e.g. 2972:0102
      --seize             use kIOHIDOptionsTypeSeizeDevice for the unlock
      --no-id-prefix      omit the repeated report ID from the unlock buffer

    UNLOCK OPTIONS:
      --send              actually send (default is a dry run that sends nothing)
      --seize             open with kIOHIDOptionsTypeSeizeDevice (experiment E2)
      --no-id-prefix      omit the repeated report ID from the buffer
      --auto              try each framing until one works
      --timeout N         seconds to watch for re-enumeration (default 5)

    GLOBAL:
      --json              machine-readable output
      --help, --version

    NOTE: `unlock` is recoverable — it reboots the dongle into its CDC bootloader, and a
    power-cycle returns it to normal mode. It does not erase anything.

    If IOHIDManagerOpen fails with 'not permitted', that is macOS TCC: grant Input Monitoring
    to your terminal in System Settings > Privacy & Security, then restart it.
    """

var args = Array(CommandLine.arguments.dropFirst())
let json = args.contains("--json")
args.removeAll { $0 == "--json" }

// Swift 6 puts top-level code on the main actor, so helpers touching `args` must be isolated
// to it too. Nothing here is concurrent; this is bookkeeping, not a design decision.
@MainActor
func flag(_ name: String) -> Bool {
    if let i = args.firstIndex(of: name) {
        args.remove(at: i)
        return true
    }
    return false
}

@MainActor
func option(_ name: String) -> String? {
    guard let i = args.firstIndex(of: name), i + 1 < args.count else { return nil }
    let v = args[i + 1]
    args.removeSubrange(i...(i + 1))
    return v
}

if flag("--help") || flag("-h") { print(usage); exit(0) }
if flag("--version") { print(version); exit(0) }

let timeout = TimeInterval(option("--timeout") ?? "5") ?? 5

switch args.first {
case "list", nil:
    let devices = USBEnumerator.ktDevices()
    if json {
        emitJSON(devices)
    } else if devices.isEmpty {
        print("no KTMicro / FiiO / bootloader devices attached.")
        print("(plug in the dongle; `ktmac list --json` for machine-readable output)")
    } else {
        for d in devices {
            let tag = d.isBootloader ? "  [BOOTLOADER]" : ""
            print("\(d.short)  \(d.productName ?? "?")  \(d.vendorName ?? "")\(tag)")
        }
    }

case "ports":
    let ports = SerialPortFinder.allPorts()
    if json {
        emitJSON(ports)
    } else if ports.isEmpty {
        print("no serial ports.")
    } else {
        for p in ports {
            let ids =
                (p.vendorID != nil && p.productID != nil)
                ? String(format: "%04x:%04x", p.vendorID!, p.productID!) : "unresolved"
            let tag = p.isBootloader ? "  [BOOTLOADER]" : ""
            print("\(p.calloutPath)  \(ids)\(tag)")
        }
        // Diagnostic that matters: if the bootloader's port lands here, the parent walk failed
        // and any Rust port of this logic would be back to guessing.
        let unresolved = SerialPortFinder.unresolvedUSBModemPorts()
        if !unresolved.isEmpty {
            print("\nUSB-modem ports whose USB parent could not be resolved:")
            for p in unresolved { print("  \(p.calloutPath)") }
        }
    }

case "port":
    // Scriptable single-value output — this is what ktflash would shell out for, or better,
    // what SerialPortFinder gets ported into Rust to replace.
    let ports = SerialPortFinder.bootloaderPorts()
    switch ports.count {
    case 0: die("no bootloader serial port — run an unlock first", code: 2)
    case 1: print(ports[0].calloutPath)
    default:
        die(
            "several bootloader ports (\(ports.map(\.calloutPath).joined(separator: ", "))) "
                + "— refusing to guess which device to flash", code: 3)
    }

case "watch":
    if USBEnumerator.bootloaderPresent() {
        print("bootloader already present.")
        exit(0)
    }
    print("waiting up to \(Int(timeout))s for 8888:cdc0…")
    if USBEnumerator.waitForBootloader(timeout: timeout) {
        print("bootloader appeared.")
        if let p = SerialPortFinder.bootloaderPorts().first {
            print("serial port: \(p.calloutPath)")
        }
    } else {
        die("bootloader did not appear", code: 4)
    }

case "unlock":
    let send = flag("--send")
    let seize = flag("--seize")
    let noPrefix = flag("--no-id-prefix")
    let auto = flag("--auto")

    if !send && !auto {
        print("[dry run] pass --send to actually attempt the unlock.\n")
    }

    let logger: (String) -> Void = { if !json { print($0) } }

    if auto {
        let results = HIDUnlocker.attemptAll(watchTimeout: timeout, log: logger)
        if json { emitJSON(results.map(\.summary)) } else {
            print("\n--- results ---")
            results.forEach { print($0.summary) }
        }
        exit(results.contains(where: \.succeeded) ? 0 : 1)
    }

    do {
        let r = try HIDUnlocker.attemptUnlock(
            method: seize ? .hidSeize : .hid,
            includeIDPrefix: !noPrefix,
            dryRun: !send,
            watchTimeout: timeout,
            log: logger)
        if json { emitJSON(r.summary) } else { print("\n" + r.summary) }

        guard send else { exit(0) }
        if r.succeeded {
            print(
                """

                *** PASS — native macOS unlock WORKS. ***
                docs/PROTOCOL.md's macOS caveat is wrong for this device and must be corrected.
                Next: flash over the CDC tty (docs/MACOS-NATIVE.md §3):
                  ktflash flash-cdc --transport serial --image fw.bin
                """)
            exit(0)
        }
        print(
            """

            *** FAIL — no re-enumeration. ***
            \(r.setReportResult == kIOReturnSuccess
                ? "The call succeeded but the device ignored it — consistent with docs/PROTOCOL.md."
                : "The call itself failed.")
            Try next, in order:
              ktmac unlock --send --no-id-prefix     (report-ID framing is the likeliest mistake)
              sudo ktmac unlock --send --seize       (experiment E2)
              cd .. && make e3 && sudo ./e3_interface_seize --send   (experiment E3)
            """)
        exit(1)
    } catch {
        die("\(error)")
    }

case "doctor":
    // Everything the native flow depends on, checked before anything is touched.
    let pre = Doctor.run()
    for c in pre.checks {
        let mark = c.status == .ok ? "\u{001B}[32m✓\u{001B}[0m"
            : (c.status == .warn ? "\u{001B}[33m!\u{001B}[0m" : "\u{001B}[31m✗\u{001B}[0m")
        print("\(mark) \(c.name): \(c.detail)")
        if let r = c.remedy, c.status != .ok { print("    → \(r)") }
    }
    print("")
    // Report both gates: planning is useful on a machine with no dongle attached.
    if pre.canProceed(executing: false) {
        print("Dry runs work here:  ktmac flow --image fw.bin")
    } else {
        print("Not even dry runs work — fix the ✗ items above.")
        exit(1)
    }
    if pre.canProceed(executing: true) {
        print("Ready to flash:      ktmac flow --image fw.bin --execute")
    } else {
        let names = pre.blockers(executing: true).map(\.name).joined(separator: ", ")
        print("Not ready to flash — still blocked on: \(names)")
    }

case "flow":
    guard let image = option("--image") else {
        die("flow needs --image <fw.bin>\n\n\(usage)")
    }
    let execute = flag("--execute")
    let assumeYes = flag("--yes")
    let seize = flag("--seize")
    let noPrefix = flag("--no-id-prefix")
    let expect = option("--expect")

    let opts = FlowOptions(
        imagePath: image,
        expect: expect,
        dryRunOnly: !execute,
        assumeYes: assumeYes,
        unlockMethod: seize ? .hidSeize : .hid,
        includeIDPrefix: !noPrefix)

    do {
        _ = try Flow.run(
            opts,
            log: { print($0) },
            confirm: { prompt in
                // Read from the terminal, not from a pipe: a confirmation that can be
                // satisfied by stdin redirection is not a confirmation.
                guard isatty(FileHandle.standardInput.fileDescriptor) == 1 else {
                    print("\(prompt)(stdin is not a terminal — pass --yes to skip this prompt)")
                    return false
                }
                print(prompt, terminator: "")
                return readLine()?.trimmingCharacters(in: .whitespaces) == "FLASH"
            })
    } catch {
        die("\(error)")
    }

case "selftest":
    // See KTMacKit/SelfTest.swift for why this is a command rather than a test target.
    print("ktmac selftest — hardware-free checks\n")
    let results = SelfTest.run(log: { print($0) })
    print("\n\(results.summary)")
    if !results.ok {
        for f in results.failed { print("FAILED: \(f.name) — \(f.detail)") }
        exit(1)
    }

case let other?:
    die("unknown command '\(other)'\n\n\(usage)")
}
