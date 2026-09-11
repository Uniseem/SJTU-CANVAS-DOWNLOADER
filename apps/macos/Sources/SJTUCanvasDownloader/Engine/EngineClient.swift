import Foundation

/// An error reported by the engine for one request, or a lost connection.
struct EngineError: LocalizedError, Sendable {
    var code: String
    var message: String
    /// Seconds after which the school suggests trying again.
    var retryAfterSeconds: Int?

    var errorDescription: String? { message }

    /// The Canvas login is gone; the window switches to the login view.
    var isUnauthorized: Bool { code == "unauthorized" }

    static let stopped = EngineError(code: "engine_stopped", message: "下载引擎未运行")
}

enum EngineNotification: Sendable {
    case loginStatus(LoginStatus)
    case accountChanged(AccountInfo)
    case downloadChanged(DownloadInfo)
    case downloadProgress(DownloadProgress)
    case downloadRemoved(String)
    /// The engine stopped on its own; the message explains why.
    case exited(String)
}

enum EngineCoding {
    static func encoder() -> JSONEncoder {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        return encoder
    }

    static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        decoder.dateDecodingStrategy = .custom { decoder in
            let container = try decoder.singleValueContainer()
            let text = try container.decode(String.self)
            guard let date = EngineDate.parse(text) else {
                throw DecodingError.dataCorruptedError(in: container, debugDescription: "无效的时间：\(text)")
            }
            return date
        }
        return decoder
    }
}

/// RFC 3339 timestamps ("2026-09-11T08:30:12.123Z", Canvas's "+08:00" too).
enum EngineDate {
    // ISO8601DateFormatter is thread-safe; it is only configured here.
    nonisolated(unsafe) private static let formatter: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter
    }()

    static func parse(_ text: String) -> Date? {
        // The fraction has 0–9 digits; ISO8601DateFormatter only reads fixed
        // shapes, so it is split off and added back.
        var whole = text
        var fraction = 0.0
        if let dot = text.firstIndex(of: ".") {
            let digits = text[text.index(after: dot)...].prefix { $0.isASCII && $0.isNumber }
            fraction = Double("0." + digits) ?? 0
            whole = String(text[..<dot]) + text[digits.endIndex...]
        }
        return formatter.date(from: whole)?.addingTimeInterval(fraction)
    }
}

/// Runs sjtu-canvas-engine as a child process and talks newline-delimited
/// JSON-RPC over its stdin/stdout. The engine exits when its stdin closes,
/// so it cannot outlive this app, not even after a crash.
final class EngineClient: @unchecked Sendable {
    let notifications: AsyncStream<EngineNotification>

    private let stream: AsyncStream<EngineNotification>.Continuation
    private let process = Process()
    private let input = Pipe()
    private let output = Pipe()
    private let errors = Pipe()
    private let exitSignal = DispatchSemaphore(value: 0)
    private let stderrClosed = DispatchSemaphore(value: 0)

    // Guarded by `lock`.
    private let lock = NSLock()
    private var pending: [Int64: CheckedContinuation<Data, Error>] = [:]
    private var nextID: Int64 = 0
    private var stderrTail = Data()
    private var stopping = false
    private var finished = false
    private var exitDescription: String?

    // Serialises writes so concurrent requests never interleave on the pipe.
    private let writeLock = NSLock()

    init() {
        let (notifications, stream) = AsyncStream.makeStream(of: EngineNotification.self)
        self.notifications = notifications
        self.stream = stream
    }

    var isRunning: Bool {
        locked { !finished } && process.isRunning
    }

    /// Why the engine stopped on its own (its last stderr lines), if it did.
    var failureDescription: String? {
        locked { exitDescription }
    }

    func start(engine: URL, dataDirectory: URL) throws {
        process.executableURL = engine
        process.arguments = ["--data-dir", dataDirectory.path]
        process.currentDirectoryURL = engine.deletingLastPathComponent()
        process.standardInput = input
        process.standardOutput = output
        process.standardError = errors
        process.terminationHandler = { [weak self] _ in
            self?.exitSignal.signal()
        }
        // Process closes the child's ends of the pipes in this process, so
        // stdout reaches EOF when the engine exits.
        try process.run()
        readOutput()
        readErrors()
    }

    // MARK: Requests

    func call<Value: Decodable>(_ method: String, params: some Encodable) async throws -> Value {
        let body = try EngineCoding.encoder().encode(params)
        let line = try await send(method, body: body)
        let response = try EngineCoding.decoder().decode(Response<Value>.self, from: line)
        if let error = response.error {
            throw EngineError(code: error.code, message: error.message, retryAfterSeconds: error.retryAfterSeconds)
        }
        guard let result = response.result else {
            throw EngineError(code: "failed", message: "\(method) 返回了空结果")
        }
        return result
    }

    private struct Response<Value: Decodable>: Decodable {
        var result: Value?
        var error: RemoteError?
    }

    private struct RemoteError: Decodable {
        var code: String
        var message: String
        var retryAfterSeconds: Int?
    }

    private func send(_ method: String, body: Data) async throws -> Data {
        let id: Int64 = locked {
            nextID += 1
            return nextID
        }
        var line = Data("{\"id\":\(id),\"method\":\"\(method)\",\"params\":".utf8)
        line.append(body)
        line.append(contentsOf: Array("}\n".utf8))

        return try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Data, Error>) in
            let accepted: Bool = locked {
                guard !finished, !stopping else { return false }
                pending[id] = continuation
                return true
            }
            guard accepted else {
                continuation.resume(throwing: EngineError.stopped)
                return
            }
            do {
                try write(line)
            } catch {
                let waiting: CheckedContinuation<Data, Error>? = locked { pending.removeValue(forKey: id) }
                waiting?.resume(throwing: EngineError(code: "engine_stopped", message: "下载引擎连接已断开"))
            }
        }
    }

    private func write(_ data: Data) throws {
        writeLock.lock()
        defer { writeLock.unlock() }
        try input.fileHandleForWriting.write(contentsOf: data)
    }

    private func closeInput() {
        writeLock.lock()
        defer { writeLock.unlock() }
        try? input.fileHandleForWriting.close()
    }

    // MARK: Shutdown

    /// Closes stdin (the engine then stops its transfers, keeping the partial
    /// files) and waits for the process; downloads resume on the next launch.
    func stop() async {
        let alreadyStopping: Bool = locked {
            let value = stopping
            stopping = true
            return value
        }
        guard !alreadyStopping, process.isRunning else { return }
        closeInput()
        if await waitForExit(seconds: 8) { return }
        process.terminate()
        if await waitForExit(seconds: 2) { return }
        kill(process.processIdentifier, SIGKILL)
    }

    private func waitForExit(seconds: Double) async -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        while process.isRunning {
            if Date() > deadline { return false }
            try? await Task.sleep(nanoseconds: 50_000_000)
        }
        return true
    }

    // MARK: Reading

    private struct Header: Decodable {
        var id: Int64?
        var method: String?
    }

    private struct Message<Params: Decodable>: Decodable {
        var params: Params
    }

    private struct IDPayload: Decodable {
        var id: String
    }

    private func readOutput() {
        let descriptor = output.fileHandleForReading.fileDescriptor
        let thread = Thread { [self] in
            var buffer = [UInt8](repeating: 0, count: 64 * 1024)
            var line = Data()
            while true {
                let count = buffer.withUnsafeMutableBytes { read(descriptor, $0.baseAddress, $0.count) }
                if count < 0 && errno == EINTR { continue }
                if count <= 0 { break }
                line.append(contentsOf: buffer[0..<count])
                // Split on "\n" bytes only: JSON strings may contain U+2028.
                while let newline = line.firstIndex(of: 0x0A) {
                    let message = line[line.startIndex..<newline]
                    if !message.isEmpty {
                        handle(Data(message))
                    }
                    line.removeSubrange(line.startIndex...newline)
                }
            }
            // EOF: wait for the exit status and the last stderr lines, then
            // fail what is still pending.
            _ = exitSignal.wait(timeout: .now() + 10)
            _ = stderrClosed.wait(timeout: .now() + 2)
            finish()
        }
        thread.name = "sjtu-canvas-engine stdout"
        thread.start()
    }

    private func readErrors() {
        let descriptor = errors.fileHandleForReading.fileDescriptor
        let thread = Thread { [self] in
            var buffer = [UInt8](repeating: 0, count: 8 * 1024)
            while true {
                let count = buffer.withUnsafeMutableBytes { read(descriptor, $0.baseAddress, $0.count) }
                if count < 0 && errno == EINTR { continue }
                if count <= 0 { break }
                locked {
                    stderrTail.append(contentsOf: buffer[0..<count])
                    if stderrTail.count > 8_000 {
                        stderrTail = Data(stderrTail.suffix(8_000))
                    }
                }
            }
            stderrClosed.signal()
        }
        thread.name = "sjtu-canvas-engine stderr"
        thread.start()
    }

    private func handle(_ line: Data) {
        let decoder = EngineCoding.decoder()
        guard let header = try? decoder.decode(Header.self, from: line) else { return }
        if let id = header.id {
            let waiting: CheckedContinuation<Data, Error>? = locked { pending.removeValue(forKey: id) }
            waiting?.resume(returning: line)
            return
        }
        switch header.method {
        case "login.status":
            if let message = try? decoder.decode(Message<LoginStatus>.self, from: line) {
                stream.yield(.loginStatus(message.params))
            }
        case "account.changed":
            if let message = try? decoder.decode(Message<AccountInfo>.self, from: line) {
                stream.yield(.accountChanged(message.params))
            }
        case "download.changed":
            if let message = try? decoder.decode(Message<DownloadInfo>.self, from: line) {
                stream.yield(.downloadChanged(message.params))
            }
        case "download.progress":
            if let message = try? decoder.decode(Message<DownloadProgress>.self, from: line) {
                stream.yield(.downloadProgress(message.params))
            }
        case "download.removed":
            if let message = try? decoder.decode(Message<IDPayload>.self, from: line) {
                stream.yield(.downloadRemoved(message.params.id))
            }
        default:
            break
        }
    }

    private func finish() {
        let status = process.isRunning ? -1 : process.terminationStatus
        let (waiting, intentional, description) = locked { () -> ([CheckedContinuation<Data, Error>], Bool, String) in
            finished = true
            let waiting = Array(pending.values)
            pending.removeAll()
            let detail = String(decoding: stderrTail, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            let description = detail.isEmpty ? "下载引擎意外退出（代码 \(status)）" : detail
            if !stopping {
                exitDescription = description
            }
            return (waiting, stopping, description)
        }
        for continuation in waiting {
            continuation.resume(throwing: EngineError(code: "engine_stopped", message: "下载引擎已退出"))
        }
        if !intentional {
            stream.yield(.exited(description))
        }
        stream.finish()
    }

    private func locked<T>(_ body: () throws -> T) rethrows -> T {
        lock.lock()
        defer { lock.unlock() }
        return try body()
    }
}
