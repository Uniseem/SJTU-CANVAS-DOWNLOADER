import SwiftUI

/// Every download task, with progress, speed and the actions for each.
@MainActor
struct DownloadsView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var downloads = model.downloads
        List {
            ForEach(downloads.items) { download in
                DownloadRow(download: download)
                    .contextMenu { menu(for: download) }
            }
        }
        .overlay {
            if downloads.items.isEmpty {
                if downloads.isLoading && downloads.counts.all > 0 {
                    ProgressView()
                } else if let error = downloads.error {
                    ContentUnavailableView("无法读取下载列表", systemImage: "exclamationmark.triangle", description: Text(error))
                } else if !downloads.query.trimmed.isEmpty {
                    ContentUnavailableView.search(text: downloads.query)
                } else {
                    ContentUnavailableView {
                        Label(emptyTitle, systemImage: downloads.filter.symbol)
                    } description: {
                        Text("在课程中选择课堂录像或课程文件，点“下载”后会出现在这里。")
                    }
                }
            }
        }
        .navigationTitle("下载")
        .navigationSubtitle(subtitle)
        .searchable(text: $downloads.query, placement: .toolbar, prompt: "搜索标题或课程")
        .toolbar {
            ToolbarItem(placement: .principal) {
                Picker("筛选", selection: $downloads.filter) {
                    ForEach(DownloadFilter.allCases) { filter in
                        Text(title(of: filter)).tag(filter)
                    }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
            }
            ToolbarItemGroup(placement: .primaryAction) {
                Button {
                    downloads.pauseAll()
                } label: {
                    Label("全部暂停", systemImage: "pause.circle")
                }
                .help("全部暂停")
                .disabled(!downloads.canPauseAll)
                Button {
                    downloads.resumeAll()
                } label: {
                    Label("全部继续", systemImage: "play.circle")
                }
                .help("全部继续")
                .disabled(!downloads.canResumeAll)
                Menu {
                    Button("在访达中显示下载文件夹") { FileActions.openFolder(model.downloadDirectory) }
                        .disabled(model.downloadDirectory.isEmpty)
                    Divider()
                    Button("清除已完成的任务") { downloads.clearCompleted() }
                        .disabled(!downloads.canClearCompleted)
                } label: {
                    Label("更多", systemImage: "ellipsis.circle")
                }
                .help("更多")
            }
        }
        .confirmationDialog(
            "取消“\(downloads.cancelCandidate?.displayName ?? "")”？",
            isPresented: $downloads.isCancelPresented,
            titleVisibility: .visible
        ) {
            Button("取消下载", role: .destructive) {
                if let download = downloads.cancelCandidate {
                    downloads.cancel(download)
                }
            }
            Button("继续下载", role: .cancel) {}
        } message: {
            Text((downloads.cancelCandidate?.received ?? 0) > 0 ? "已下载的部分会被删除，之后可以重新下载。" : "之后可以重新下载。")
        }
        .task {
            await model.downloads.load()
        }
    }

    private var subtitle: String {
        let running = model.downloads.counts.running
        return running > 0 ? "\(running) 个任务正在进行" : ""
    }

    private var emptyTitle: String {
        switch model.downloads.filter {
        case .active: return "没有进行中的下载"
        case .completed: return "还没有完成的下载"
        case .failed: return "没有失败或取消的下载"
        case .all: return "还没有下载任务"
        }
    }

    private func title(of filter: DownloadFilter) -> String {
        let count = filter.count(in: model.downloads.counts)
        return count > 0 ? "\(filter.title) \(count)" : filter.title
    }

    @ViewBuilder
    private func menu(for download: DownloadInfo) -> some View {
        let downloads = model.downloads
        if download.isCompleted {
            Button("打开") { downloads.open(download) }
                .disabled(!FileActions.exists(download.filePath))
        }
        Button("在访达中显示") { downloads.reveal(download) }
        Divider()
        if download.isRunning {
            Button("暂停") { downloads.pause(download) }
        }
        if download.status == "paused" {
            Button("继续") { downloads.resume(download) }
        }
        if download.isStopped {
            Button("重新下载") { downloads.retry(download) }
        }
        if download.isUnfinished || download.status == "failed" {
            Button("取消下载…") { downloads.cancelCandidate = download }
        }
        if !download.isUnfinished {
            Divider()
            Button("从列表中移除") { downloads.remove(download) }
        }
    }
}

@MainActor
private struct DownloadRow: View {
    @Environment(AppModel.self) private var model
    let download: DownloadInfo

    var body: some View {
        let downloads = model.downloads
        HStack(spacing: 12) {
            Image(systemName: download.kind == "video" ? "play.rectangle" : Format.fileSymbol(contentType: nil, name: download.title))
                .font(.title2)
                .foregroundStyle(.secondary)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 4) {
                Text(download.displayName)
                    .font(.headline)
                    .lineLimit(1)
                Text("\(download.courseName) · \(download.kind == "video" ? "课堂录像" : "课程文件")")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                if showsProgress {
                    if download.status == "downloading" && download.total == nil {
                        ProgressView()
                            .progressViewStyle(.linear)
                            .controlSize(.small)
                    } else {
                        ProgressView(value: fraction)
                            .controlSize(.small)
                            .tint(download.status == "downloading" ? nil : Color.secondary)
                    }
                }
                Label {
                    Text(statusText)
                        .foregroundStyle(download.status == "failed" ? Color.red : Color.secondary)
                        .lineLimit(2)
                        .textSelection(.enabled)
                } icon: {
                    Image(systemName: Format.statusSymbol(download.status))
                        .foregroundStyle(Format.statusColor(download.status))
                }
                .font(.caption)
                .monospacedDigit()
            }
            Spacer(minLength: 12)
            HStack(spacing: 6) {
                if download.isRunning {
                    RowButton(title: "暂停", symbol: "pause.circle") { downloads.pause(download) }
                }
                if download.status == "paused" {
                    RowButton(title: "继续", symbol: "play.circle") { downloads.resume(download) }
                }
                if download.isStopped {
                    RowButton(title: "重新下载", symbol: "arrow.clockwise.circle") { downloads.retry(download) }
                }
                if download.isUnfinished || download.status == "failed" {
                    RowButton(title: "取消下载", symbol: "xmark.circle") { downloads.cancelCandidate = download }
                }
                if download.isCompleted {
                    RowButton(title: "打开", symbol: "arrow.up.forward.app") { downloads.open(download) }
                        .disabled(!FileActions.exists(download.filePath))
                }
                RowButton(title: "在访达中显示", symbol: "magnifyingglass.circle") { downloads.reveal(download) }
                if !download.isUnfinished {
                    RowButton(title: "从列表中移除", symbol: "minus.circle") { downloads.remove(download) }
                }
            }
        }
        .padding(.vertical, 4)
    }

    private var showsProgress: Bool {
        switch download.status {
        case "downloading": return true
        case "paused", "queued", "failed": return download.received > 0
        default: return false
        }
    }

    private var fraction: Double {
        guard let total = download.total, total > 0 else { return 0 }
        return min(1, Double(download.received) / Double(total))
    }

    private var statusText: String {
        let amount = download.total.map { "\(Format.size(download.received)) / \(Format.size($0))" } ?? Format.size(download.received)
        switch download.status {
        case "downloading":
            return ["下载中", amount, Format.speed(download.speed), Format.remaining(received: download.received, total: download.total, speed: download.speed)]
                .filter { !$0.isEmpty }
                .joined(separator: " · ")
        case "queued":
            if let note = download.error, !note.isEmpty {
                return "排队中 · \(note)"
            }
            return "排队中"
        case "paused":
            return download.received > 0 ? "已暂停 · \(amount)" : "已暂停"
        case "completed":
            var text = "已完成 · \(Format.size(download.total ?? download.received))"
            if let done = download.completedAt {
                text += " · \(Format.relative(done))"
            }
            return text
        case "failed":
            return "失败：\(download.error ?? "未知错误")"
        case "cancelled":
            return "已取消"
        default:
            return Format.status(download.status)
        }
    }
}

/// A small icon button in a row; the title is the tooltip and the
/// accessibility label.
@MainActor
private struct RowButton: View {
    let title: String
    let symbol: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.title3)
        }
        .buttonStyle(.borderless)
        .help(title)
        .accessibilityLabel(title)
    }
}
