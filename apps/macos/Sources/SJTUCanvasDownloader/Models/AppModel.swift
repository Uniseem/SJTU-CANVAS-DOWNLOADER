import AppKit
import Observation
import SwiftUI

enum EngineState: Equatable {
    case stopped
    case starting
    case ready
    case failed(String)
}

/// What the sidebar shows in the detail column.
enum SidebarItem: Hashable {
    case downloads
    case course(String)
}

struct AppAlert: Identifiable {
    let id = UUID()
    var title: String
    var message: String
}

/// App-wide state: the engine connection, the Canvas account, the course
/// list, the download counts and the dialogs. There is one main window.
@MainActor
@Observable
final class AppModel {
    static let shared = AppModel()

    // MARK: Engine

    private(set) var engineState: EngineState = .stopped
    private(set) var settings: SettingsInfo?
    private(set) var engineVersion = ""
    private(set) var account = AccountInfo()
    var preferencesError: String?

    @ObservationIgnored private var engine: EngineClient?
    @ObservationIgnored private var listener: Task<Void, Never>?

    // MARK: Content

    let login = LoginModel()
    let downloads = DownloadsModel()
    private(set) var courses: [Course] = []
    private(set) var isLoadingCourses = false
    private(set) var coursesError: String?
    private(set) var counts = DownloadCounts()
    private(set) var current: CourseModel?
    private var selectionValue: SidebarItem?
    var showsFinishedCourses = false

    @ObservationIgnored private var countsTask: Task<Void, Never>?
    @ObservationIgnored private var activity: NSObjectProtocol?
    @ObservationIgnored private var wasRunning = false
    @ObservationIgnored private var completedInBatch = 0
    @ObservationIgnored private var failedInBatch = 0

    // MARK: Dialogs and window

    var alert: AppAlert?
    var isSignOutPresented = false
    @ObservationIgnored var openMainWindow: (@MainActor () -> Void)?

    private init() {
        login.app = self
        downloads.app = self
    }

    var isReady: Bool { engineState == .ready }

    var isSignedIn: Bool { isReady && account.authenticated }

    var needsShutdown: Bool { engine != nil }

    var isDemo: Bool { settings?.fakeSchool == true }

    var downloadDirectory: String {
        settings?.preferences.downloadDir ?? ""
    }

    // MARK: - Engine lifecycle

    func start() {
        guard engineState != .starting else { return }
        Task { await startEngine() }
    }

    func startEngine() async {
        engineState = .starting
        if let previous = engine {
            engine = nil
            listener?.cancel()
            await previous.stop()
        }
        let client = EngineClient()
        do {
            let location = try EngineLocator.locate()
            let data = DataLocation.current
            try FileManager.default.createDirectory(at: data, withIntermediateDirectories: true)
            try client.start(engine: location, dataDirectory: data)
            engine = client
            listen(to: client)
            let result: InitializeResult = try await client.call(
                "engine.initialize",
                params: InitializeParams(sessionKey: KeychainStore.sessionKey(), downloadsFolder: DataLocation.downloadsFolder?.path)
            )
            settings = result.settings
            engineVersion = result.version
            account = result.account
            engineState = .ready
        } catch {
            if engine === client {
                engine = nil
            }
            await client.stop()
            // An engine that exited during start-up explains why on stderr.
            engineState = .failed(client.failureDescription ?? error.localizedDescription)
            return
        }
        await refreshCounts()
        if account.authenticated {
            await accountDidChange()
            await verifyAccount()
        } else {
            await login.start()
        }
    }

    func shutdown() async {
        guard let engine else { return }
        self.engine = nil
        listener?.cancel()
        await engine.stop()
        engineState = .stopped
        endActivity()
    }

    private func listen(to client: EngineClient) {
        listener?.cancel()
        listener = Task { [weak self] in
            for await notification in client.notifications {
                guard let self, self.engine === client else { return }
                self.handle(notification)
            }
        }
    }

    private func handle(_ notification: EngineNotification) {
        switch notification {
        case .loginStatus(let status):
            login.apply(status)
        case .accountChanged(let info):
            Task { await updateAccount(info) }
        case .downloadChanged(let info):
            downloads.apply(info)
            if info.status == "completed" {
                completedInBatch += 1
            } else if info.status == "failed" {
                failedInBatch += 1
            }
            refreshCountsSoon()
        case .downloadProgress(let progress):
            downloads.apply(progress)
        case .downloadRemoved(let id):
            downloads.remove(id)
            refreshCountsSoon()
        case .exited(let message):
            engine = nil
            engineState = .failed(message)
            endActivity()
        }
    }

    /// Calls the engine; fails with "下载引擎未运行" while it is not running.
    func call<Value: Decodable>(_ method: String, _ params: some Encodable) async throws -> Value {
        guard let engine else { throw EngineError.stopped }
        return try await engine.call(method, params: params)
    }

    func call<Value: Decodable>(_ method: String) async throws -> Value {
        try await call(method, NoParams())
    }

    // MARK: - Account

    /// Takes a new account state; a login that started or ended switches views.
    private func updateAccount(_ info: AccountInfo) async {
        let changed = info.authenticated != account.authenticated || info.profile?.id != account.profile?.id
        account = info
        if changed {
            await accountDidChange()
        }
    }

    /// Signed in: the courses; signed out: the login view with a fresh QR code.
    private func accountDidChange() async {
        if account.authenticated {
            await loadCourses()
            if selection == nil {
                selection = courses.first(where: \.isCurrent).map { .course($0.id) } ?? .downloads
            }
            await downloads.load()
        } else {
            selection = nil
            courses = []
            coursesError = nil
            await login.start()
        }
    }

    /// Confirms a restored login with Canvas; an expired one ends here.
    private func verifyAccount() async {
        guard let checked: AccountInfo = try? await call("account.get", VerifyParams(verify: true)) else {
            // Offline: keep the saved login; requests report their own errors.
            return
        }
        await updateAccount(checked)
    }

    /// Shows the sign-out confirmation in the main window.
    func confirmSignOut() {
        showMainWindow()
        isSignOutPresented = true
    }

    func signOut() {
        Task {
            do {
                let info: AccountInfo = try await call("account.logout")
                await updateAccount(info)
            } catch {
                alert = AppAlert(title: "无法退出登录", message: error.localizedDescription)
            }
        }
    }

    /// A request found the login gone (it expired or was revoked).
    func handleUnauthorized() {
        Task {
            if let info: AccountInfo = try? await call("account.get") {
                await updateAccount(info)
            }
        }
    }

    // MARK: - Courses

    var selection: SidebarItem? {
        get { selectionValue }
        set {
            guard newValue != selectionValue else { return }
            selectionValue = newValue
            if case .course(let id) = newValue, let course = courses.first(where: { $0.id == id }) {
                if current?.course.id != id {
                    current?.close()
                    let model = CourseModel(course: course, app: self)
                    current = model
                    Task { await model.load() }
                }
            } else {
                current?.close()
                current = nil
            }
        }
    }

    /// The sidebar search: course name, code or teacher.
    var courseQuery = ""

    private var matchingCourses: [Course] {
        let search = courseQuery.trimmed
        guard !search.isEmpty else { return courses }
        return courses.filter {
            $0.name.localizedCaseInsensitiveContains(search)
                || $0.courseCode.localizedCaseInsensitiveContains(search)
                || ($0.teacher ?? "").localizedCaseInsensitiveContains(search)
        }
    }

    var currentCourses: [Course] { matchingCourses.filter(\.isCurrent) }

    var finishedCourses: [Course] { matchingCourses.filter { !$0.isCurrent } }

    func loadCourses() async {
        guard isSignedIn else { return }
        isLoadingCourses = true
        coursesError = nil
        do {
            let list: CourseList = try await call("courses.list")
            courses = list.courses
        } catch let error as EngineError where error.isUnauthorized {
            handleUnauthorized()
        } catch {
            coursesError = error.localizedDescription
        }
        isLoadingCourses = false
        if case .course(let id) = selection, !courses.contains(where: { $0.id == id }) {
            selection = nil
        }
    }

    func refresh() {
        Task {
            await loadCourses()
            if let current {
                await current.load()
            }
            await downloads.load()
        }
    }

    // MARK: - Downloads

    /// Queues downloads, asking for a folder first when the user wants that.
    /// Returns the result, or nil when the folder picker was cancelled.
    func createDownloads(_ items: [NewDownload]) async throws -> CreateResult? {
        var destination: String?
        if settings?.preferences.askDestination == true {
            guard let folder = await FileActions.chooseFolder(
                message: "选择这次下载的保存位置，课程和讲次会自动分文件夹保存。",
                prompt: "下载到这里",
                startingAt: downloadDirectory
            ) else {
                return nil
            }
            destination = folder.path
        }
        var result = CreateResult(created: [], skipped: [])
        // The engine accepts at most 500 items per request.
        var start = 0
        while start < items.count {
            let batch = Array(items[start..<min(start + 500, items.count)])
            let part: CreateResult = try await call("downloads.create", CreateParams(items: batch, destination: destination))
            result.created += part.created
            result.skipped += part.skipped.map { item in
                var shifted = item
                shifted.index += start
                return shifted
            }
            start += 500
        }
        refreshCountsSoon()
        return result
    }

    /// "已添加 4 个下载任务，2 个已经下载过" plus what else was skipped and why.
    static func summary(of result: CreateResult) -> String {
        var parts = [result.created.isEmpty ? "没有添加新的下载任务" : "已添加 \(result.created.count) 个下载任务"]
        let downloaded = result.skipped.filter { $0.reason == "downloaded" }.count
        let queued = result.skipped.filter { $0.reason == "queued" }.count
        if downloaded > 0 {
            parts.append("\(downloaded) 个已经下载过")
        }
        if queued > 0 {
            parts.append("\(queued) 个已在下载队列中")
        }
        var text = parts.joined(separator: "，")
        if let other = result.skipped.first(where: { $0.reason != "downloaded" && $0.reason != "queued" }) {
            text += "。" + other.message
        }
        return text
    }

    func showDownloads() {
        showMainWindow()
        if isSignedIn {
            selection = .downloads
        }
    }

    private func refreshCountsSoon() {
        countsTask?.cancel()
        countsTask = Task {
            try? await Task.sleep(nanoseconds: 300_000_000)
            guard !Task.isCancelled else { return }
            await refreshCounts()
        }
    }

    private func refreshCounts() async {
        guard let list: DownloadList = try? await call("downloads.list", ListParams(limit: 1)) else { return }
        counts = list.counts
        downloads.counts = list.counts
        countsChanged()
    }

    /// Dock badge, a notification when the queue empties in the background,
    /// and no App Nap or idle sleep while files are downloading.
    private func countsChanged() {
        NSApp.dockTile.badgeLabel = counts.running > 0 ? "\(counts.running)" : nil
        if counts.running > 0 {
            wasRunning = true
            if activity == nil {
                activity = ProcessInfo.processInfo.beginActivity(options: .userInitiated, reason: "正在下载课程文件")
            }
        } else {
            endActivity()
            if wasRunning {
                wasRunning = false
                notifyBatchFinished()
            }
        }
    }

    private func notifyBatchFinished() {
        let (completed, failed) = (completedInBatch, failedInBatch)
        completedInBatch = 0
        failedInBatch = 0
        guard completed + failed > 0, !NSApp.isActive else { return }
        Notifier.shared.post(
            title: failed > 0 ? "下载结束" : "下载完成",
            body: failed > 0 ? "已完成 \(completed) 个，\(failed) 个失败" : "已完成 \(completed) 个文件"
        )
    }

    private func endActivity() {
        if let activity {
            ProcessInfo.processInfo.endActivity(activity)
            self.activity = nil
        }
    }

    // MARK: - Window

    func showMainWindow() {
        openMainWindow?()
        NSApp.activate()
    }

    var isAlertPresented: Bool {
        get { alert != nil }
        set { if !newValue { alert = nil } }
    }

    // MARK: - Settings

    func setPreference<Value: Equatable>(_ keyPath: WritableKeyPath<Preferences, Value>, _ value: Value) {
        guard var preferences = settings?.preferences, preferences[keyPath: keyPath] != value else { return }
        preferences[keyPath: keyPath] = value
        settings?.preferences = preferences
        Task { await savePreferences(preferences) }
    }

    func savePreferences(_ preferences: Preferences) async {
        do {
            let updated: SettingsInfo = try await call("settings.update", SettingsUpdateParams(preferences: preferences))
            settings = updated
            preferencesError = nil
        } catch {
            preferencesError = error.localizedDescription
            if let current: SettingsInfo = try? await call("settings.get") {
                settings = current
            }
        }
    }

    func showAbout() {
        let credits = NSMutableAttributedString(
            string: "下载上海交通大学 Canvas 课程的课堂录像与课程文件。以 MIT 许可证发布，第三方组件的许可见应用包内的 THIRD_PARTY_NOTICES.md。本应用与上海交通大学无关。\n\n",
            attributes: [.font: NSFont.systemFont(ofSize: NSFont.smallSystemFontSize), .foregroundColor: NSColor.secondaryLabelColor]
        )
        credits.append(NSAttributedString(
            string: "源代码与说明",
            attributes: [
                .font: NSFont.systemFont(ofSize: NSFont.smallSystemFontSize),
                .link: URL(string: "https://github.com/Uniseem/SJTU-CANVAS-DOWNLOADER")!,
            ]
        ))
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        credits.addAttribute(.paragraphStyle, value: paragraph, range: NSRange(location: 0, length: credits.length))
        var options: [NSApplication.AboutPanelOptionKey: Any] = [
            .applicationName: "SJTU Canvas Downloader",
            .credits: credits,
        ]
        if !engineVersion.isEmpty {
            options[.version] = "引擎 \(engineVersion)"
        }
        NSApp.orderFrontStandardAboutPanel(options: options)
        NSApp.activate()
    }
}
