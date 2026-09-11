import Foundation
import Observation

enum CourseTab: String, CaseIterable, Identifiable, Hashable {
    case lessons, files

    var id: String { rawValue }
}

/// The selected course: its lesson recordings and files, the tracks to
/// download, and lesson sizes looked up while the rows are on screen.
@MainActor
@Observable
final class CourseModel {
    static let allTracks = ["slides", "teacher", "composite"]
    private static let sizeConcurrency = 2

    let course: Course
    @ObservationIgnored private weak var app: AppModel?

    var tab: CourseTab = .lessons
    var query = ""
    var lessonSelection: Set<String> = []
    var fileSelection: Set<String> = []
    /// Selected tracks in canonical order: slides, teacher, composite.
    private(set) var tracks: [String]

    private(set) var lessons: [Lesson] = []
    private(set) var files: [CourseFile] = []
    private(set) var isLoadingLessons = false
    private(set) var isLoadingFiles = false
    private(set) var lessonsError: String?
    /// The lesson message is information (no recordings scheduled), not a failure.
    private(set) var lessonsNotice = false
    private(set) var filesError: String?
    private(set) var retrySeconds = 0
    var historicalDismissed = false
    /// The last "已加入下载" message.
    var result: QueueResult?
    private(set) var isQueueing = false

    /// Known sizes by lesson and track.
    private(set) var sizes: [String: [String: TrackSize]] = [:]
    private(set) var fetching: Set<String> = []

    @ObservationIgnored private var visible: Set<String> = []
    @ObservationIgnored private var queue: [(id: String, refresh: Bool)] = []
    @ObservationIgnored private var running = 0
    @ObservationIgnored private var autoRetryUsed = false
    @ObservationIgnored private var countdown: Task<Void, Never>?
    @ObservationIgnored private var closed = false

    struct QueueResult: Equatable {
        var message: String
        var isWarning: Bool
        var isError: Bool
    }

    init(course: Course, app: AppModel) {
        self.course = course
        self.app = app
        let defaults = app.settings?.preferences.defaultTracks ?? ["slides", "teacher"]
        let tracks = Self.allTracks.filter(defaults.contains)
        self.tracks = tracks.isEmpty ? ["slides", "teacher"] : tracks
    }

    var subtitle: String {
        [course.courseCode, course.term ?? "", course.teacher ?? ""]
            .filter { !$0.trimmed.isEmpty }
            .joined(separator: " · ")
    }

    var historicalCount: Int { lessons.filter { $0.source == "historical" }.count }

    var visibleLessons: [Lesson] {
        let search = query.trimmed
        guard !search.isEmpty else { return lessons }
        return lessons.filter {
            $0.title.localizedCaseInsensitiveContains(search) || $0.classroom.localizedCaseInsensitiveContains(search)
        }
    }

    var visibleFiles: [CourseFile] {
        let search = query.trimmed
        guard !search.isEmpty else { return files }
        return files.filter {
            $0.name.localizedCaseInsensitiveContains(search) || $0.filename.localizedCaseInsensitiveContains(search)
        }
    }

    // MARK: - Loading

    func load() async {
        async let lessonsLoaded: Void = loadLessons()
        async let filesLoaded: Void = loadFiles()
        _ = await (lessonsLoaded, filesLoaded)
    }

    func loadLessons(automatic: Bool = false) async {
        guard let app else { return }
        cancelCountdown()
        if !automatic {
            autoRetryUsed = false
        }
        isLoadingLessons = true
        lessonsError = nil
        lessonsNotice = false
        do {
            let list: LessonList = try await app.call("courses.lessons", CourseParams(courseId: course.id))
            lessons = list.lessons
            let available = Set(lessons.filter(\.available).map(\.id))
            lessonSelection = lessonSelection.intersection(available)
        } catch let error as EngineError {
            if error.isUnauthorized {
                app.handleUnauthorized()
            } else {
                lessonsError = error.message
                lessonsNotice = error.code == "video_unavailable"
                if error.code == "upstream_unavailable", let seconds = error.retryAfterSeconds, seconds > 0, !autoRetryUsed {
                    startCountdown(min(seconds, 120))
                }
            }
        } catch {
            lessonsError = error.localizedDescription
        }
        isLoadingLessons = false
    }

    func loadFiles() async {
        guard let app else { return }
        isLoadingFiles = true
        filesError = nil
        do {
            let list: FileList = try await app.call("courses.files", CourseParams(courseId: course.id))
            files = list.files
            fileSelection = fileSelection.intersection(Set(files.map(\.id)))
        } catch let error as EngineError where error.isUnauthorized {
            app.handleUnauthorized()
        } catch {
            filesError = error.localizedDescription
        }
        isLoadingFiles = false
    }

    private func startCountdown(_ seconds: Int) {
        autoRetryUsed = true
        retrySeconds = seconds
        countdown = Task { [weak self] in
            while let self, self.retrySeconds > 0 {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                guard !Task.isCancelled else { return }
                self.retrySeconds -= 1
            }
            guard !Task.isCancelled, let self else { return }
            await self.loadLessons(automatic: true)
        }
    }

    private func cancelCountdown() {
        countdown?.cancel()
        countdown = nil
        retrySeconds = 0
    }

    func close() {
        closed = true
        cancelCountdown()
        queue.removeAll()
    }

    // MARK: - Tracks

    func hasTrack(_ track: String) -> Bool { tracks.contains(track) }

    /// Adds or removes a track; at least one stays selected.
    func setTrack(_ track: String, _ selected: Bool) {
        var chosen = Set(tracks)
        if selected {
            chosen.insert(track)
        } else if chosen.count > 1 {
            chosen.remove(track)
        }
        tracks = Self.allTracks.filter(chosen.contains)
        for id in visible {
            enqueue(id, refresh: false)
        }
        pump()
    }

    var tracksText: String {
        tracks.map(Format.trackLabel).joined(separator: "、")
    }

    // MARK: - Sizes: looked up for rows on screen, two lessons at a time

    func rowAppeared(_ lesson: Lesson) {
        guard lesson.available else { return }
        visible.insert(lesson.id)
        enqueue(lesson.id, refresh: false)
        pump()
    }

    func rowDisappeared(_ lesson: Lesson) {
        visible.remove(lesson.id)
        queue.removeAll { $0.id == lesson.id && !$0.refresh }
    }

    func retrySize(_ lesson: Lesson) {
        if var known = sizes[lesson.id] {
            for track in tracks where known[track]?.status == "unavailable" {
                known[track] = nil
            }
            sizes[lesson.id] = known
        }
        enqueue(lesson.id, refresh: true)
        pump()
    }

    private func missingTracks(_ id: String) -> [String] {
        tracks.filter { sizes[id]?[$0] == nil }
    }

    private func enqueue(_ id: String, refresh: Bool) {
        guard !fetching.contains(id), !queue.contains(where: { $0.id == id }), !missingTracks(id).isEmpty else { return }
        queue.append((id, refresh))
    }

    private func pump() {
        while !closed, running < Self.sizeConcurrency, !queue.isEmpty {
            let (id, refresh) = queue.removeFirst()
            let missing = missingTracks(id)
            guard !missing.isEmpty else { continue }
            running += 1
            fetching.insert(id)
            Task { await fetchSizes(id, tracks: missing, refresh: refresh) }
        }
    }

    private func fetchSizes(_ id: String, tracks: [String], refresh: Bool) async {
        var result: [String: TrackSize] = [:]
        do {
            guard let app else { throw EngineError.stopped }
            let found: LessonSizes = try await app.call(
                "lessons.sizes",
                SizesParams(courseId: course.id, lessonId: id, tracks: tracks, refresh: refresh)
            )
            for track in tracks {
                result[track] = found.tracks[track] ?? .unavailable
            }
        } catch {
            for track in tracks {
                result[track] = .unavailable
            }
        }
        running -= 1
        fetching.remove(id)
        guard !closed else { return }
        var known = sizes[id] ?? [:]
        for (track, size) in result {
            known[track] = size
        }
        sizes[id] = known
        pump()
    }

    /// The size column: the sum of the selected tracks.
    func sizeText(_ lesson: Lesson) -> String {
        guard lesson.available else { return "" }
        let values = tracks.map { sizes[lesson.id]?[$0] }
        if values.contains(where: { $0 == nil }) {
            return fetching.contains(lesson.id) || queue.contains(where: { $0.id == lesson.id }) ? "读取中…" : "—"
        }
        let known = values.compactMap { $0 }
        if known.contains(where: { $0.status == "missing" }) {
            return "无所选画面"
        }
        if known.contains(where: { $0.status != "ready" || $0.size == nil }) {
            return "大小未知"
        }
        return Format.size(known.reduce(0) { $0 + ($1.size ?? 0) })
    }

    func canRetrySize(_ lesson: Lesson) -> Bool {
        guard lesson.available else { return false }
        let values = tracks.compactMap { sizes[lesson.id]?[$0] }
        return values.count == tracks.count && !values.contains { $0.status == "missing" }
            && values.contains { $0.status != "ready" || $0.size == nil }
    }

    /// "电脑屏幕：32.1 MB" per track, for the size column's help tag.
    func sizeDetail(_ lesson: Lesson) -> String {
        tracks.map { track in
            let text: String
            switch sizes[lesson.id]?[track] {
            case .some(let size) where size.status == "ready" && size.size != nil:
                text = Format.size(size.size ?? 0)
            case .some(let size) where size.status == "missing":
                text = "没有这个画面"
            case .some:
                text = "暂时无法获取"
            case .none:
                text = "尚未获取"
            }
            return "\(Format.trackLabel(track))：\(text)"
        }
        .joined(separator: "\n")
    }

    // MARK: - Downloads

    var selectedLessons: [Lesson] {
        lessons.filter { $0.available && lessonSelection.contains($0.id) }
    }

    var selectedFiles: [CourseFile] {
        files.filter { fileSelection.contains($0.id) }
    }

    var selectionSummary: String {
        switch tab {
        case .lessons:
            let count = selectedLessons.count
            return count == 0 ? "" : "已选择 \(count) 讲 · 将下载 \(count * tracks.count) 个视频文件"
        case .files:
            let chosen = selectedFiles
            return chosen.isEmpty ? "" : "已选择 \(chosen.count) 个文件 · \(Format.size(chosen.reduce(0) { $0 + $1.size }))"
        }
    }

    var downloadTitle: String {
        switch tab {
        case .lessons:
            let count = selectedLessons.count
            return count == 0 ? "下载所选" : "下载 \(count) 讲"
        case .files:
            let count = selectedFiles.count
            return count == 0 ? "下载所选" : "下载 \(count) 个文件"
        }
    }

    var canDownloadSelection: Bool {
        !isQueueing && (tab == .lessons ? !selectedLessons.isEmpty : !selectedFiles.isEmpty)
    }

    var isAllSelected: Bool {
        switch tab {
        case .lessons:
            let available = lessons.filter(\.available)
            return !available.isEmpty && available.allSatisfy { lessonSelection.contains($0.id) }
        case .files:
            return !files.isEmpty && files.allSatisfy { fileSelection.contains($0.id) }
        }
    }

    func toggleSelectAll() {
        switch tab {
        case .lessons:
            lessonSelection = isAllSelected ? [] : Set(lessons.filter(\.available).map(\.id))
        case .files:
            fileSelection = isAllSelected ? [] : Set(files.map(\.id))
        }
    }

    func items(for lessons: [Lesson]) -> [NewDownload] {
        lessons.filter(\.available).flatMap { lesson in
            tracks.map { track in
                let known = sizes[lesson.id]?[track]
                return NewDownload.video(course: course, lesson: lesson, track: track, size: known?.status == "ready" ? known?.size : nil)
            }
        }
    }

    func items(for files: [CourseFile]) -> [NewDownload] {
        files.map { NewDownload.file(course: course, file: $0) }
    }

    func downloadSelection() {
        let chosen = tab == .lessons ? items(for: selectedLessons) : items(for: selectedFiles)
        queueDownloads(chosen, clearsSelection: true)
    }

    func download(_ lesson: Lesson) {
        queueDownloads(items(for: [lesson]), clearsSelection: false)
    }

    func download(_ file: CourseFile) {
        queueDownloads(items(for: [file]), clearsSelection: false)
    }

    private func queueDownloads(_ items: [NewDownload], clearsSelection: Bool) {
        guard let app, !items.isEmpty else { return }
        isQueueing = true
        Task {
            defer { isQueueing = false }
            do {
                guard let created = try await app.createDownloads(items) else { return }
                result = QueueResult(
                    message: AppModel.summary(of: created),
                    isWarning: created.created.isEmpty || !created.skipped.isEmpty,
                    isError: false
                )
                if clearsSelection && !created.created.isEmpty {
                    if tab == .lessons {
                        lessonSelection = []
                    } else {
                        fileSelection = []
                    }
                }
            } catch let error as EngineError where error.isUnauthorized {
                app.handleUnauthorized()
            } catch {
                result = QueueResult(message: error.localizedDescription, isWarning: false, isError: true)
            }
        }
    }
}
