// KtflashCLI.swift — how `ktmac` invokes the Rust `ktflash`.
//
// The argv construction is a pure function on purpose. It is the part that decides whether a
// command writes flash, and it is far easier to assert on than to eyeball inside an
// orchestrator — see the `selftest` checks. Nothing here executes anything; see `Flow.swift`.

import Foundation

/// A `ktflash flash-cdc` invocation.
public struct FlashCdcInvocation: Sendable, Equatable {
    public var imagePath: String
    /// nil = let ktflash choose the transport. `.serial` is the macOS-native path.
    public var port: String?
    /// Post-reset identity to require, e.g. `2972:0102`. Absent means "record what came back".
    public var expect: String?
    /// **The dangerous one.** false = dry run that prints the packet plan and touches nothing.
    public var execute: Bool
    public var flagOverride: UInt32?

    public init(
        imagePath: String, port: String? = nil, expect: String? = nil, execute: Bool = false,
        flagOverride: UInt32? = nil
    ) {
        self.imagePath = imagePath
        self.port = port
        self.expect = expect
        self.execute = execute
        self.flagOverride = flagOverride
    }

    /// The argv, minus the binary itself.
    ///
    /// `--yes` is emitted only alongside `--execute`, and never independently: it is the
    /// acknowledgement that the operator has a known-good image saved, and it should never
    /// appear on a command that was not already going to write.
    public var arguments: [String] {
        var args = ["flash-cdc", "--image", imagePath]
        if let flagOverride { args += ["--flag", String(flagOverride)] }
        if let port { args += ["--transport", "serial", "--port", port] }
        if let expect { args += ["--expect", expect] }
        if execute { args += ["--execute", "--yes"] }
        return args
    }

    /// Copy-pasteable, so the operator can see and re-run exactly what was done.
    public func commandLine(binary: String) -> String {
        ([binary] + arguments).map { $0.contains(" ") ? "'\($0)'" : $0 }.joined(separator: " ")
    }
}

/// One preflight check and how it went.
public struct PreflightCheck: Sendable {
    public enum Status: String, Sendable {
        case ok
        case warn
        case fail
    }

    /// When a failure of this check actually blocks.
    ///
    /// A dry run builds a packet plan from a file and touches no hardware, so demanding TCC
    /// consent and an attached dongle before letting someone *look at the plan* would block the
    /// exact inspect-before-you-commit workflow the dry run exists for.
    public enum Requirement: Sendable {
        /// Needed even to plan.
        case always
        /// Only needed once we are going to unlock and write.
        case execute
    }

    public let name: String
    public let status: Status
    public let detail: String
    /// What to do about it, when there is something to do.
    public let remedy: String?
    public let requiredFor: Requirement

    public init(
        name: String, status: Status, detail: String, remedy: String? = nil,
        requiredFor: Requirement = .execute
    ) {
        self.name = name
        self.status = status
        self.detail = detail
        self.remedy = remedy
        self.requiredFor = requiredFor
    }
}

public struct Preflight: Sendable {
    public let checks: [PreflightCheck]
    public var ktflashPath: String?

    /// Blocking failures for the operation being attempted.
    public func blockers(executing: Bool) -> [PreflightCheck] {
        checks.filter {
            $0.status == .fail && (executing || $0.requiredFor == .always)
        }
    }

    public func canProceed(executing: Bool) -> Bool { blockers(executing: executing).isEmpty }
}

public enum Doctor {
    /// Everything the native flow depends on, checked before anything is touched.
    ///
    /// Read-only: enumerates, looks for binaries, and probes TCC by opening an IOHIDManager.
    /// It sends no reports and opens no serial ports.
    public static func run() -> Preflight {
        var checks: [PreflightCheck] = []

        // --- the Rust tool does the actual work ---
        let ktflash = ProcessRunner.which("ktflash")
        if let ktflash {
            var detail = ktflash
            if let r = try? ProcessRunner.run(ktflash, ["--help"]), r.ok {
                detail += "  (responds to --help)"
            }
            checks.append(
                .init(
                    name: "ktflash on PATH", status: .ok, detail: detail, requiredFor: .always))
        } else {
            checks.append(
                .init(
                    name: "ktflash on PATH", status: .fail, detail: "not found",
                    remedy:
                        "build it (cd flasher && cargo build --release) or install a release "
                        + "into ~/.local/bin",
                    requiredFor: .always))
        }

        // --- TCC: the first wall, and invisible unless you name it ---
        do {
            _ = try HIDUnlocker.candidateDevices()
            checks.append(
                .init(
                    name: "Input Monitoring (TCC)", status: .ok,
                    detail: "IOHIDManager opens — HID unlock is reachable"))
        } catch KTError.notPermitted {
            checks.append(
                .init(
                    name: "Input Monitoring (TCC)", status: .fail,
                    detail: "IOHIDManagerOpen returned kIOReturnNotPermitted",
                    remedy:
                        "System Settings > Privacy & Security > Input Monitoring > + (add your "
                        + "terminal), then restart the terminal. Nothing about the dongle can be "
                        + "learned until this is granted."))
        } catch {
            checks.append(
                .init(
                    name: "Input Monitoring (TCC)", status: .warn,
                    detail: "\(error)",
                    remedy: "not necessarily fatal — `ktmac unlock` will report the real error"))
        }

        // --- the device itself ---
        let devices = USBEnumerator.ktDevices()
        if let boot = devices.first(where: \.isBootloader) {
            checks.append(
                .init(
                    name: "dongle", status: .warn,
                    detail: "already in bootloader mode (\(boot.short))",
                    remedy:
                        "power-cycle it before running `flow`, which expects to do the unlock "
                        + "itself (the bootloader state machine is one-shot)"))
        } else if let dongle = devices.first {
            checks.append(
                .init(
                    name: "dongle", status: .ok,
                    detail: "\(dongle.short) \(dongle.productName ?? "")"))
        } else {
            checks.append(
                .init(
                    name: "dongle", status: .fail, detail: "no KTMicro/FiiO device attached",
                    remedy: "plug the dongle into this Mac"))
        }

        // --- can we resolve a serial port to its USB owner at all? ---
        let ports = SerialPortFinder.allPorts()
        let resolvable = ports.contains { $0.vendorID != nil }
        if ports.isEmpty {
            checks.append(
                .init(
                    name: "serial port resolution", status: .warn,
                    detail: "no serial ports present to test the USB parent walk against"))
        } else if resolvable {
            checks.append(
                .init(
                    name: "serial port resolution", status: .ok,
                    detail: "\(ports.count) port(s); the IOKit parent walk resolves VID/PID"))
        } else {
            checks.append(
                .init(
                    name: "serial port resolution", status: .warn,
                    detail: "\(ports.count) port(s), none resolved to a USB device",
                    remedy:
                        "if the bootloader's port also fails to resolve, `flow` cannot safely "
                        + "pick a device and will stop rather than guess"))
        }

        // --- explicitly note that OrbStack is not required ---
        let orb = ProcessRunner.which("orb") != nil
        checks.append(
            .init(
                name: "OrbStack", status: .ok,
                detail: orb
                    ? "installed, but not needed for this flow"
                    : "not installed, and not needed for this flow"))

        return Preflight(checks: checks, ktflashPath: ktflash)
    }
}
