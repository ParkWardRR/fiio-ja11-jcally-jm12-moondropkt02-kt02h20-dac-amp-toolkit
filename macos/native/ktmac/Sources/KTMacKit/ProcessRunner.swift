// ProcessRunner.swift — run `ktflash` (or anything else) and stream its output.
//
// `ktmac` orchestrates the macOS-native flash flow but does not reimplement the protocol: the
// byte-exact framing, the journal and the safety gates all live in the Rust `ktflash`, and
// duplicating any of that in Swift would mean two implementations of the thing that erases
// firmware. So this drives the real binary and shows you exactly what it ran.

import Foundation

public struct ProcessResult: Sendable {
    public let exitCode: Int32
    public let stdout: String
    public let stderr: String
    public var ok: Bool { exitCode == 0 }
}

/// Accumulates a pipe's bytes and splits them into lines.
///
/// `readabilityHandler` closures are `@Sendable` and fire on Foundation's own queues, so the
/// shared state has to be behind a lock and the type has to vouch for itself. That is all the
/// `@unchecked` here means: every access below is inside `lock`.
private final class OutputCollector: @unchecked Sendable {
    private let lock = NSLock()
    private var data = Data()
    private var pending = ""
    private let onLine: (@Sendable (String) -> Void)?

    init(onLine: (@Sendable (String) -> Void)?) {
        self.onLine = onLine
    }

    func append(_ chunk: Data) {
        lock.lock()
        defer { lock.unlock() }
        data.append(chunk)
        guard let onLine, let text = String(data: chunk, encoding: .utf8) else { return }
        pending += text
        while let nl = pending.firstIndex(of: "\n") {
            onLine(String(pending[pending.startIndex..<nl]))
            pending = String(pending[pending.index(after: nl)...])
        }
    }

    /// Emit any trailing partial line and return everything collected.
    func finish() -> String {
        lock.lock()
        defer { lock.unlock() }
        if let onLine, !pending.isEmpty {
            onLine(pending)
            pending = ""
        }
        return String(data: data, encoding: .utf8) ?? ""
    }
}

public enum ProcessRunner {
    /// Run a command, streaming output to `onLine` as it arrives.
    ///
    /// Streaming matters here: a flash prints packet progress for a minute or more, and
    /// buffering it until exit would leave the operator staring at nothing during the one
    /// operation they most want to watch.
    @discardableResult
    public static func run(
        _ executable: String,
        _ arguments: [String],
        onLine: (@Sendable (String) -> Void)? = nil
    ) throws -> ProcessResult {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments

        let outPipe = Pipe()
        let errPipe = Pipe()
        process.standardOutput = outPipe
        process.standardError = errPipe

        let out = OutputCollector(onLine: onLine)
        // stderr is captured but not streamed: interleaving it with stdout would scramble the
        // packet-progress output, and it is printed in full on failure.
        let err = OutputCollector(onLine: nil)

        outPipe.fileHandleForReading.readabilityHandler = { handle in
            let chunk = handle.availableData
            if !chunk.isEmpty { out.append(chunk) }
        }
        errPipe.fileHandleForReading.readabilityHandler = { handle in
            let chunk = handle.availableData
            if !chunk.isEmpty { err.append(chunk) }
        }

        try process.run()
        process.waitUntilExit()

        outPipe.fileHandleForReading.readabilityHandler = nil
        errPipe.fileHandleForReading.readabilityHandler = nil
        // Drain whatever landed between the last handler call and exit.
        out.append(outPipe.fileHandleForReading.readDataToEndOfFile())
        err.append(errPipe.fileHandleForReading.readDataToEndOfFile())

        return ProcessResult(
            exitCode: process.terminationStatus, stdout: out.finish(), stderr: err.finish())
    }

    /// Find an executable on PATH. Returns nil rather than throwing so callers can report a
    /// missing tool as a preflight failure instead of a crash.
    public static func which(_ name: String) -> String? {
        if name.contains("/") {
            return FileManager.default.isExecutableFile(atPath: name) ? name : nil
        }
        let path = ProcessInfo.processInfo.environment["PATH"] ?? "/usr/bin:/bin"
        for dir in path.split(separator: ":") {
            let candidate = String(dir) + "/" + name
            if FileManager.default.isExecutableFile(atPath: candidate) { return candidate }
        }
        // Common install locations that a GUI-launched process may not have on PATH.
        for candidate in [
            NSHomeDirectory() + "/.local/bin/" + name,
            "/opt/homebrew/bin/" + name,
            "/usr/local/bin/" + name,
        ] where FileManager.default.isExecutableFile(atPath: candidate) {
            return candidate
        }
        return nil
    }
}
