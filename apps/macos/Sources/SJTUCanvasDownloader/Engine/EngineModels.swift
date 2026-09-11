import Foundation

// Wire types of the sjtu-canvas-engine JSON-RPC protocol (engine/src/rpc.rs,
// engine/src/models.rs, engine/src/downloads). EngineClient converts between
// snake_case and camelCase keys.

struct Profile: Decodable, Hashable, Sendable {
    var id: String
    var name: String
    var shortName: String?
    var avatarUrl: String?
}

struct AccountInfo: Decodable, Hashable, Sendable {
    var authenticated = false
    var profile: Profile?
    /// False when the login could not be protected with a keychain key; it
    /// then lasts only until the app quits.
    var persisted = false
    /// Set by account.get with verify: the login was confirmed with Canvas.
    var verified: Bool?
}

struct LoginStatus: Decodable, Hashable, Sendable {
    var attemptId: String
    /// preparing | waiting | reconnecting | authorizing | authorized | expired | cancelled | error
    var state: String
    var message: String
    var generation: Int
    var revision: Int64
    /// The QR code as a base64 PNG while waiting for the scan.
    var qrPng: String?
    var expiresAt: String?
}

struct Course: Decodable, Identifiable, Hashable, Sendable {
    var id: String
    var name: String
    var courseCode: String
    var term: String?
    var teacher: String?
    /// active | invited_or_pending | completed
    var enrollmentState: String

    var isCurrent: Bool { enrollmentState != "completed" }
}

struct CourseList: Decodable, Sendable {
    var courses: [Course]
}

struct Lesson: Decodable, Identifiable, Hashable, Sendable {
    var videoId: String
    var title: String
    var beginTime: String
    var endTime: String
    var classroom: String
    var available: Bool
    /// resource | canvas-lti | historical (课堂视频旧版)
    var source: String?

    var id: String { videoId }
}

struct LessonList: Decodable, Sendable {
    var lessons: [Lesson]
}

struct CourseFile: Decodable, Identifiable, Hashable, Sendable {
    var id: String
    var displayName: String
    var filename: String
    var size: Int64
    var contentType: String?
    var updatedAt: String?

    var name: String { displayName.trimmed.isEmpty ? filename : displayName }
}

struct FileList: Decodable, Sendable {
    var files: [CourseFile]
}

struct TrackSize: Decodable, Hashable, Sendable {
    /// ready | missing | unavailable
    var status: String
    var size: Int64?

    static let unavailable = TrackSize(status: "unavailable", size: nil)
}

struct LessonSizes: Decodable, Sendable {
    var videoId: String
    var tracks: [String: TrackSize]
}

struct DownloadInfo: Decodable, Identifiable, Hashable, Sendable {
    var id: String
    /// video | file
    var kind: String
    var courseId: String
    var courseName: String
    var resourceId: String
    var track: String?
    var title: String
    /// "第 01 讲 · 电脑屏幕" for videos, the file name for course files.
    var displayName: String
    var beginTime: String?
    var destination: String
    /// The final location once known (the file is `<path>.part` until done).
    var filePath: String?
    /// queued | downloading | paused | completed | failed | cancelled
    var status: String
    var received: Int64
    var total: Int64?
    /// Bytes per second while downloading.
    var speed: Double
    var error: String?
    var createdAt: Date
    var updatedAt: Date
    var completedAt: Date?

    var isUnfinished: Bool { status == "queued" || status == "downloading" || status == "paused" }
    var isRunning: Bool { status == "queued" || status == "downloading" }
    var isCompleted: Bool { status == "completed" }
    var isStopped: Bool { status == "failed" || status == "cancelled" }
}

struct DownloadProgress: Decodable, Sendable {
    var id: String
    var received: Int64
    var total: Int64?
    var speed: Double
}

struct DownloadCounts: Decodable, Hashable, Sendable {
    var all = 0
    /// Not finished: queued, downloading or paused.
    var active = 0
    /// Queued or downloading.
    var running = 0
    var completed = 0
    /// Failed or cancelled.
    var failed = 0
}

struct DownloadList: Decodable, Sendable {
    var items: [DownloadInfo]
    var counts: DownloadCounts
}

struct CreateResult: Decodable, Sendable {
    var created: [DownloadInfo]
    var skipped: [SkippedItem]
}

struct SkippedItem: Decodable, Sendable {
    var index: Int
    /// invalid | queued | downloaded
    var reason: String
    var message: String
}

/// One item of downloads.create. Absent optionals stay off the wire: the
/// engine refuses unknown fields.
struct NewDownload: Encodable, Sendable {
    var kind: String
    var courseId: String
    var courseName: String
    var lessonId: String?
    var fileId: String?
    var title: String
    var beginTime: String?
    var track: String?
    var size: Int64?

    static func video(course: Course, lesson: Lesson, track: String, size: Int64?) -> NewDownload {
        NewDownload(
            kind: "video",
            courseId: course.id,
            courseName: course.name,
            lessonId: lesson.videoId,
            title: lesson.title,
            beginTime: lesson.beginTime.trimmed.isEmpty ? nil : lesson.beginTime,
            track: track,
            size: size
        )
    }

    static func file(course: Course, file: CourseFile) -> NewDownload {
        NewDownload(
            kind: "file",
            courseId: course.id,
            courseName: course.name,
            fileId: file.id,
            title: file.name,
            size: file.size > 0 ? file.size : nil
        )
    }
}

struct ProxySettings: Codable, Hashable, Sendable {
    /// system | direct | custom
    var mode = "system"
    var url: String?
}

struct Preferences: Codable, Hashable, Sendable {
    var downloadDir = ""
    var askDestination = false
    var concurrency = 3
    var defaultTracks = ["slides", "teacher"]
    var proxy = ProxySettings()
}

struct SettingsInfo: Decodable, Hashable, Sendable {
    var preferences: Preferences
    var defaultDownloadDir: String
    var concurrencyMax: Int
    var dataDir: String
    var engineVersion: String
    var testMode: Bool
    var fakeSchool: Bool
}

struct InitializeResult: Decodable, Sendable {
    var `protocol`: Int
    var version: String
    var settings: SettingsInfo
    var account: AccountInfo
}

// MARK: - Request parameters

struct NoParams: Encodable, Sendable {}

struct InitializeParams: Encodable, Sendable {
    var sessionKey: String?
    var downloadsFolder: String?
}

struct VerifyParams: Encodable, Sendable {
    var verify: Bool
}

struct CourseParams: Encodable, Sendable {
    var courseId: String
}

struct SizesParams: Encodable, Sendable {
    var courseId: String
    var lessonId: String
    var tracks: [String]
    var refresh: Bool
}

struct CreateParams: Encodable, Sendable {
    var items: [NewDownload]
    var destination: String?
}

struct ListParams: Encodable, Sendable {
    var filter: String?
    var query: String?
    var limit: Int?
}

struct IDParams: Encodable, Sendable {
    var id: String
}

struct SettingsUpdateParams: Encodable, Sendable {
    var preferences: Preferences
}

struct Empty: Decodable, Sendable {}

struct CountResult: Decodable, Sendable {
    var count: Int
}
