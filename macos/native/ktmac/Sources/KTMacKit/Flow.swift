// Flow.swift — the whole macOS-native flash, end to end, with no OrbStack.
//
// ## What this replaces
//
// `orbstack/ktflash-orbstack.sh flash <fw.bin>` currently does: create an Ubuntu guest, install
// build deps, pass the dongle through, build ktflash inside the guest, unlock, re-attach the
// device because it re-enumerated under a new VID:PID, and flash over libusb. Every one of
// those steps exists to work around macOS refusing to let libusb claim the HID interface.
//
// The native path is:
//
//   1. preflight             — ktflash present, TCC granted, dongle attached
//   2. dry run               — ktflash prints the packet plan; nothing is touched
//   3. confirm               — the operator types FLASH, having saved a known-good image
//   4. unlock                — IOHIDManager SetReport, natively (docs/MACOS-NATIVE.md §4)
//   5. resolve the port      — wait for 8888:cdc0 and its /dev/cu.* node, matched by VID:PID
//   6. flash                 — ktflash flash-cdc --transport serial --port ... --execute --yes
//   7. confirm               — ktflash's own post-reset reprobe journals the outcome
//
// Steps 4 and 5 are the parts macOS used to make impossible. Step 4 is the one still genuinely
// in question — see docs/MACOS-NATIVE.md §4 — so `flow` reports honestly when it fails rather
// than falling back to something that looks like it worked.
//
// ## Why it shells out for the flash
//
// The framing, the journal and the safety gates live in the Rust `ktflash`. A second
// implementation of the thing that erases firmware is the last thing this project needs, so
// `flow` orchestrates and prints the exact command it runs.
//
// STATUS: UNTESTED against hardware.

import Foundation

public struct FlowOptions: Sendable {
    public var imagePath: String
    /// Post-reset identity to require, e.g. `2972:0102`.
    public var expect: String?
    /// Stop after the dry run. The default, so `flow` alone is never destructive.
    public var dryRunOnly: Bool
    /// Skip the interactive confirmation. For scripted use; still requires `--execute`.
    public var assumeYes: Bool
    public var unlockMethod: UnlockMethod
    public var includeIDPrefix: Bool
    public var portWaitSeconds: TimeInterval

    public init(
        imagePath: String, expect: String? = nil, dryRunOnly: Bool = true,
        assumeYes: Bool = false, unlockMethod: UnlockMethod = .hid,
        includeIDPrefix: Bool = true, portWaitSeconds: TimeInterval = 15
    ) {
        self.imagePath = imagePath
        self.expect = expect
        self.dryRunOnly = dryRunOnly
        self.assumeYes = assumeYes
        self.unlockMethod = unlockMethod
        self.includeIDPrefix = includeIDPrefix
        self.portWaitSeconds = portWaitSeconds
    }
}

public enum FlowError: Error, CustomStringConvertible {
    case preflightFailed([PreflightCheck])
    case imageMissing(String)
    case dryRunFailed(Int32)
    case aborted
    case unlockFailed(UnlockAttempt)
    case noBootloaderPort(String)
    case ambiguousPort([String])
    case flashFailed(Int32)

    public var description: String {
        switch self {
        case .preflightFailed(let checks):
            let bad = checks.map(\.name).joined(separator: ", ")
            return "preflight failed: \(bad)"
        case .imageMissing(let p):
            return "no such image: \(p)"
        case .dryRunFailed(let c):
            return "the dry run failed (exit \(c)) — refusing to flash an image ktflash rejected"
        case .aborted:
            return "aborted"
        case .unlockFailed(let a):
            return """
                unlock did not re-enumerate the dongle.
                \(a.summary)

                This is the open question in docs/MACOS-NATIVE.md §4. Try, in order:
                  ktmac unlock --send --no-id-prefix
                  sudo ktmac unlock --send --seize
                  cd macos/native && make e3 && sudo ./e3_interface_seize --send

                Until one of those works, the OrbStack path still flashes: orbstack/ktflash-orbstack.sh
                """
        case .noBootloaderPort(let d):
            return """
                the bootloader came up but no serial port resolved to 8888:cdc0 (\(d)).

                Check `ktmac ports`. If the port is listed as "unresolved", the IOKit parent
                walk failed and the flow will not guess — pointing a firmware write at the wrong
                device is worse than stopping.
                """
        case .ambiguousPort(let paths):
            return "several bootloader ports (\(paths.joined(separator: ", "))) — refusing to guess"
        case .flashFailed(let c):
            return "ktflash flash-cdc exited \(c) — read its output and the journal it printed"
        }
    }
}

public struct FlowResult: Sendable {
    public let dryRanOnly: Bool
    public let port: String?
    public let command: String?
}

public enum Flow {
    /// Run the native flash flow.
    ///
    /// - Parameter confirm: asked once, immediately before anything destructive. Returning
    ///   false aborts. Passing nil means "no confirmation available" and, unless
    ///   `assumeYes` is set, aborts rather than proceeding silently.
    public static func run(
        _ options: FlowOptions,
        log: @escaping @Sendable (String) -> Void,
        confirm: ((String) -> Bool)? = nil
    ) throws -> FlowResult {
        // --- 1. preflight ---
        // Gated on what we are actually about to do: a dry run reads a file and prints a packet
        // plan, so it needs `ktflash` and nothing else. Demanding TCC consent and an attached
        // dongle before letting someone look at the plan would block the inspect-before-you-
        // commit workflow the dry run exists for.
        let executing = !options.dryRunOnly
        log("── preflight (\(executing ? "execute" : "dry run")) ──")
        let pre = Doctor.run()
        for c in pre.checks {
            let blocking = c.status == .fail && (executing || c.requiredFor == .always)
            let mark = c.status == .ok ? "✓" : (blocking ? "✗" : "!")
            var line = "  \(mark) \(c.name): \(c.detail)"
            if c.status == .fail && !blocking { line += "  (not needed for a dry run)" }
            log(line)
            if let r = c.remedy, c.status != .ok, blocking { log("      → \(r)") }
        }
        guard pre.canProceed(executing: executing), let ktflash = pre.ktflashPath else {
            throw FlowError.preflightFailed(pre.blockers(executing: executing))
        }
        guard FileManager.default.fileExists(atPath: options.imagePath) else {
            throw FlowError.imageMissing(options.imagePath)
        }

        // --- 2. dry run ---
        // Do this BEFORE the unlock. The bootloader state machine is one-shot, so discovering a
        // bad image after unlocking would mean power-cycling and starting over.
        log("\n── dry run (nothing is written) ──")
        let dry = FlashCdcInvocation(imagePath: options.imagePath, expect: options.expect)
        log("  $ \(dry.commandLine(binary: ktflash))")
        let dryResult = try ProcessRunner.run(ktflash, dry.arguments) { log("  " + $0) }
        guard dryResult.ok else {
            log(dryResult.stderr)
            throw FlowError.dryRunFailed(dryResult.exitCode)
        }

        if options.dryRunOnly {
            log("\n[dry run] stopping here. Re-run with --execute to unlock and write.")
            return FlowResult(dryRanOnly: true, port: nil, command: nil)
        }

        // --- 3. confirm ---
        if !options.assumeYes {
            guard let confirm else { throw FlowError.aborted }
            log(
                """

                ⚠  THIS WILL ERASE AND REWRITE THE DONGLE'S FIRMWARE.
                   ktflash cannot read firmware off a KT02H20, so there is no backup. If this
                   goes wrong, recovery needs a compatible image you already have on disk.
                   The KT_USB_BOOT ROM survives an app-flash, so you can retry — but only with
                   an image in hand.
                """)
            guard confirm("Type FLASH to unlock and write: ") else { throw FlowError.aborted }
        }

        // --- 4. unlock, natively ---
        log("\n── unlock (native, no OrbStack) ──")
        let attempt = try HIDUnlocker.attemptUnlock(
            method: options.unlockMethod,
            includeIDPrefix: options.includeIDPrefix,
            dryRun: false,
            watchTimeout: 8,
            log: { log("  " + $0) })
        guard attempt.succeeded else { throw FlowError.unlockFailed(attempt) }
        log("  bootloader is up")

        // --- 5. resolve the bootloader's serial port ---
        // The tty node is published a moment after the USB device, so this waits.
        log("\n── resolving the bootloader's serial port ──")
        guard let port = SerialPortFinder.waitForBootloaderPort(timeout: options.portWaitSeconds)
        else {
            let ports = SerialPortFinder.bootloaderPorts().map(\.calloutPath)
            if ports.count > 1 { throw FlowError.ambiguousPort(ports) }
            let seen = SerialPortFinder.allPorts().map(\.calloutPath).joined(separator: ", ")
            throw FlowError.noBootloaderPort(seen.isEmpty ? "no ports at all" : seen)
        }
        log("  \(port.calloutPath)")

        // --- 6. flash ---
        log("\n── flashing ──")
        let write = FlashCdcInvocation(
            imagePath: options.imagePath, port: port.calloutPath, expect: options.expect,
            execute: true)
        let commandLine = write.commandLine(binary: ktflash)
        log("  $ \(commandLine)")
        let result = try ProcessRunner.run(ktflash, write.arguments) { log("  " + $0) }
        if !result.ok {
            log(result.stderr)
            throw FlowError.flashFailed(result.exitCode)
        }

        // --- 7. ktflash's own post-reset reprobe already journaled the outcome ---
        log("\n── done ──")
        log("  ktflash reprobed the bus and recorded the result in its journal (above).")
        log("  Play audio through the dongle to check it end to end.")
        return FlowResult(dryRanOnly: false, port: port.calloutPath, command: commandLine)
    }
}
