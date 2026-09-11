import Foundation
import Observation

enum DownloadFilter: String, CaseIterable, Identifiable, Hashable {
    case all, active, completed, failed

    var id: String { rawValue }

    var title: String {
        switch self {
        case .all: return "全部"
        case .active: return "进行中"
        case .completed: return "已完成"
        case .failed: return "失败与取消"
        }
    }

    var symbol: String {
        switch self {
        case .all: return "tray.full"
        case .active: return "arrow.down.circle"
        case .completed: return "checkmark.circle"
        case .failed: return "exclamationmark.triangle"
        }
    }

    func count(in counts: DownloadCounts) -> Int {
        switch self {
        case .all: return counts.all
        case .active: return counts.active
        case .completed: return counts.completed
        case .failed: return counts.failed
        }
    }

    func matches(_ download: DownloadInfo) -> Bool {
        switch self {
        case .all: return true
        case .active: return download.isUnfinished
        case .completed: return download.isCompleted
        case .failed: return download.isStopped
        }
    }
}

/// The download list, kept in step with engine notifications.
@MainActor
@Observable
final class DownloadsModel {
    @ObservationIgnored weak var app: AppModel?

    private(set) var items: [DownloadInfo] = []
    var counts = DownloadCounts()
    private(set) var isLoading = false
    private(set) var error: String?
    var cancelCandidate: DownloadInfo?
    private var filterValue: DownloadFilter = .all
    private var queryValue = ""

    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var queryTask: Task<Void, Never>?

    var filter: DownloadFilter {
        get { filterValue }
        set {
            guard newValue != filterValue else { return }
            filterValue = newValue
            Task { await load() }
        }
    }

    var query: String {
        get { queryValue }
        set {
            guard newValue != queryValue else { return }
            queryValue = newValue
            queryTask?.cancel()
            queryTask = Task {
                try? await Task.sleep(nanoseconds: 250_000_000)
                guard !Task.isCancelled else { return }
                await load()
            }
        }
    }

    func load() async {
        guard let app, app.isReady else { return }
        generation += 1
        let current = generation
        isLoading = true
        do {
            let list: DownloadList = try await app.call(
                "downloads.list",
                ListParams(filter: filter.rawValue, query: query.trimmed, limit: 2000)
            )
            guard current == generation else { return }
            items = list.items
            counts = list.counts
            error = nil
        } catch {
            guard current == generation else { return }
            self.error = error.localizedDescription
        }
        isLoading = false
    }

    /// Inserts, updates or drops one row according to the filter and search.
    func apply(_ download: DownloadInfo) {
        let search = query.trimmed
        let visible = filter.matches(download)
            && (search.isEmpty
                || download.displayName.localizedCaseInsensitiveContains(search)
                || download.courseName.localizedCaseInsensitiveContains(search))
        if let index = items.firstIndex(where: { $0.id == download.id }) {
            // Responses to overlapping requests may arrive out of order.
            if items[index].updatedAt > download.updatedAt {
                return
            }
            if visible {
                items[index] = download
            } else {
                items.remove(at: index)
            }
        } else if visible {
            // The newest batch first, in queue order within a batch.
            let position = items.firstIndex { $0.createdAt < download.createdAt } ?? items.endIndex
            items.insert(download, at: position)
        }
    }

    func apply(_ progress: DownloadProgress) {
        guard let index = items.firstIndex(where: { $0.id == progress.id }), items[index].status == "downloading" else { return }
        items[index].received = progress.received
        items[index].total = progress.total ?? items[index].total
        items[index].speed = progress.speed
    }

    func remove(_ id: String) {
        items.removeAll { $0.id == id }
    }

    // MARK: - Actions

    func pause(_ download: DownloadInfo) { perform("downloads.pause", download, failure: "无法暂停") }

    func resume(_ download: DownloadInfo) { perform("downloads.resume", download, failure: "无法继续") }

    func retry(_ download: DownloadInfo) { perform("downloads.retry", download, failure: "无法重新下载") }

    func cancel(_ download: DownloadInfo) { perform("downloads.cancel", download, failure: "无法取消") }

    func remove(_ download: DownloadInfo) {
        guard let app else { return }
        Task {
            do {
                let _: Empty = try await app.call("downloads.remove", IDParams(id: download.id))
            } catch {
                app.alert = AppAlert(title: "无法移除", message: error.localizedDescription)
            }
        }
    }

    private func perform(_ method: String, _ download: DownloadInfo, failure: String) {
        guard let app else { return }
        Task {
            do {
                let updated: DownloadInfo = try await app.call(method, IDParams(id: download.id))
                apply(updated)
            } catch {
                app.alert = AppAlert(title: failure, message: error.localizedDescription)
            }
        }
    }

    func pauseAll() { bulk("downloads.pauseAll", failure: "无法全部暂停") }

    func resumeAll() { bulk("downloads.resumeAll", failure: "无法全部继续") }

    func clearCompleted() { bulk("downloads.clearCompleted", failure: "无法清除已完成的任务") }

    private func bulk(_ method: String, failure: String) {
        guard let app else { return }
        Task {
            do {
                let _: CountResult = try await app.call(method)
            } catch {
                app.alert = AppAlert(title: failure, message: error.localizedDescription)
            }
        }
    }

    var canPauseAll: Bool { counts.running > 0 }

    var canResumeAll: Bool { counts.active > counts.running }

    var canClearCompleted: Bool { counts.completed > 0 }

    var isCancelPresented: Bool {
        get { cancelCandidate != nil }
        set { if !newValue { cancelCandidate = nil } }
    }

    // MARK: - Files

    func open(_ download: DownloadInfo) {
        if let path = download.filePath, FileActions.exists(path) {
            FileActions.open(path)
        }
    }

    func reveal(_ download: DownloadInfo) {
        if let path = download.filePath, FileActions.exists(path) {
            FileActions.reveal(path)
        } else if let path = download.filePath, FileActions.exists(path + ".part") {
            FileActions.reveal(path + ".part")
        } else {
            FileActions.openFolder(download.destination)
        }
    }
}
