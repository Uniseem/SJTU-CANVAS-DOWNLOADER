import Foundation

enum EngineLocator {
    /// SJTU Canvas Downloader.app/Contents/MacOS/sjtu-canvas-engine. During
    /// development (`swift run`) the engine comes from engine/target — the
    /// debug build first for a debug app, since only it has the test mode.
    /// SJTU_CANVAS_ENGINE overrides both.
    static func locate() throws -> URL {
        let environment = ProcessInfo.processInfo.environment
        var candidates: [URL] = []
        if let path = environment["SJTU_CANVAS_ENGINE"], !path.isEmpty {
            candidates.append(URL(fileURLWithPath: path))
        }
        if let bundled = Bundle.main.url(forAuxiliaryExecutable: "sjtu-canvas-engine") {
            candidates.append(bundled)
        }
        if let repository = repositoryRoot() {
            #if DEBUG
            let profiles = ["debug", "release"]
            #else
            let profiles = ["release", "debug"]
            #endif
            for profile in profiles {
                candidates.append(repository.appendingPathComponent("engine/target/\(profile)/sjtu-canvas-engine"))
            }
        }
        guard let engine = candidates.first(where: { FileManager.default.isExecutableFile(atPath: $0.path) }) else {
            throw EngineError(code: "engine_missing", message: "找不到下载引擎 sjtu-canvas-engine，请重新安装应用。")
        }
        return engine
    }

    /// The source checkout when running from apps/macos/.build.
    private static func repositoryRoot() -> URL? {
        var directory = Bundle.main.executableURL?.deletingLastPathComponent()
        for _ in 0..<8 {
            guard let current = directory else { return nil }
            if FileManager.default.fileExists(atPath: current.appendingPathComponent("engine/Cargo.toml").path) {
                return current
            }
            directory = current.deletingLastPathComponent()
        }
        return nil
    }
}

/// Where the app keeps its data (download list, settings, the encrypted
/// login): ~/Library/Application Support/SJTU Canvas Downloader, or
/// SJTU_CANVAS_DATA_DIR for development and tests.
enum DataLocation {
    static var current: URL {
        if let path = ProcessInfo.processInfo.environment["SJTU_CANVAS_DATA_DIR"], !path.isEmpty {
            return URL(fileURLWithPath: path, isDirectory: true)
        }
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return support.appendingPathComponent("SJTU Canvas Downloader", isDirectory: true)
    }

    /// The user's Downloads folder; new downloads go to "SJTU Canvas" inside it.
    static var downloadsFolder: URL? {
        if let path = ProcessInfo.processInfo.environment["SJTU_CANVAS_DOWNLOADS_FOLDER"], !path.isEmpty {
            return URL(fileURLWithPath: path, isDirectory: true)
        }
        return FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
    }
}
