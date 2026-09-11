import SwiftUI

/// A course: lesson recordings and course files, selected and downloaded
/// from the bar at the bottom.
@MainActor
struct CourseView: View {
    @Environment(AppModel.self) private var model
    @Bindable var course: CourseModel

    var body: some View {
        VStack(spacing: 0) {
            notices
            switch course.tab {
            case .lessons:
                LessonListView(course: course)
            case .files:
                FileListView(course: course)
            }
        }
        .navigationTitle(course.course.name)
        .navigationSubtitle(course.subtitle)
        .searchable(text: $course.query, placement: .toolbar, prompt: course.tab == .lessons ? "搜索录像或教室" : "搜索文件")
        .toolbar {
            ToolbarItem(placement: .principal) {
                Picker("内容", selection: $course.tab) {
                    Text(lessonsTitle).tag(CourseTab.lessons)
                    Text(filesTitle).tag(CourseTab.files)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
            }
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    ForEach(CourseModel.allTracks, id: \.self) { track in
                        Toggle(isOn: Binding(
                            get: { course.hasTrack(track) },
                            set: { course.setTrack(track, $0) }
                        )) {
                            Label(Format.trackLabel(track), systemImage: Format.trackSymbol(track))
                        }
                        .disabled(course.hasTrack(track) && course.tracks.count == 1)
                    }
                } label: {
                    Label("画面：\(course.tracksText)", systemImage: "rectangle.on.rectangle")
                }
                .help("下载录像时包含的画面")
                .disabled(course.tab != .lessons)
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            SelectionBar(course: course)
        }
    }

    private var lessonsTitle: String {
        course.isLoadingLessons || course.lessonsError != nil ? "课堂录像" : "课堂录像 \(course.lessons.count)"
    }

    private var filesTitle: String {
        course.isLoadingFiles || course.filesError != nil ? "课程文件" : "课程文件 \(course.files.count)"
    }

    @ViewBuilder
    private var notices: some View {
        if let result = course.result {
            Notice(
                style: result.isError ? .error : result.isWarning ? .warning : .success,
                title: result.isError ? "无法添加下载" : "已加入下载",
                message: result.message,
                onClose: { course.result = nil }
            ) {
                if !result.isError {
                    Button("查看下载") { model.selection = .downloads }
                }
            }
        }
        if course.tab == .lessons {
            if let error = course.lessonsError {
                Notice(
                    style: course.lessonsNotice ? .info : .error,
                    title: course.lessonsNotice ? "暂时没有课堂录像" : "无法读取课堂录像",
                    message: course.retrySeconds > 0 ? "\(error)（\(course.retrySeconds) 秒后自动重试）" : error
                ) {
                    Button("重试") { Task { await course.loadLessons() } }
                        .disabled(course.isLoadingLessons)
                }
            } else if course.historicalCount > 0 && !course.historicalDismissed {
                Notice(
                    style: .info,
                    title: "来自旧版课堂视频",
                    message: "新视频平台没有这门课的录像，下面的 \(course.historicalCount) 条录像来自旧版课堂视频。",
                    onClose: { course.historicalDismissed = true }
                ) {
                    EmptyView()
                }
            }
        } else if let error = course.filesError {
            Notice(style: .error, title: "无法读取课程文件", message: error) {
                Button("重试") { Task { await course.loadFiles() } }
                    .disabled(course.isLoadingFiles)
            }
        }
    }
}

// MARK: - Lessons

@MainActor
private struct LessonListView: View {
    @Bindable var course: CourseModel

    var body: some View {
        let lessons = course.visibleLessons
        List(selection: $course.lessonSelection) {
            ForEach(lessons) { lesson in
                LessonRow(course: course, lesson: lesson)
                    .tag(lesson.id)
                    .selectionDisabled(!lesson.available)
                    .onAppear { course.rowAppeared(lesson) }
                    .onDisappear { course.rowDisappeared(lesson) }
            }
        }
        .contextMenu(forSelectionType: String.self) { ids in
            let chosen = course.lessons.filter { ids.contains($0.id) && $0.available }
            if !chosen.isEmpty {
                Button(chosen.count == 1 ? "下载这一讲" : "下载 \(chosen.count) 讲") {
                    course.lessonSelection = Set(chosen.map(\.id))
                    course.downloadSelection()
                }
            }
        } primaryAction: { ids in
            if let lesson = course.lessons.first(where: { ids.contains($0.id) && $0.available }) {
                course.download(lesson)
            }
        }
        .overlay {
            if lessons.isEmpty {
                if course.isLoadingLessons {
                    ProgressView()
                } else if course.lessonsError == nil {
                    if !course.query.trimmed.isEmpty {
                        ContentUnavailableView.search(text: course.query)
                    } else {
                        ContentUnavailableView(
                            "还没有课堂录像",
                            systemImage: "video.slash",
                            description: Text("视频平台暂未返回这门课的录像。录像通常在课后几小时内开放。")
                        )
                    }
                }
            }
        }
    }
}

@MainActor
private struct LessonRow: View {
    let course: CourseModel
    let lesson: Lesson

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: lesson.available ? "play.rectangle" : "clock")
                .font(.title2)
                .foregroundStyle(.secondary)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 3) {
                Text(lesson.title)
                    .font(.headline)
                    .lineLimit(1)
                Text(meta)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 12)
            if lesson.available {
                Text(course.sizeText(lesson))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                    .help(course.sizeDetail(lesson))
                if course.canRetrySize(lesson) {
                    Button {
                        course.retrySize(lesson)
                    } label: {
                        Image(systemName: "arrow.clockwise")
                    }
                    .buttonStyle(.borderless)
                    .help("重新读取大小")
                }
                Button {
                    course.download(lesson)
                } label: {
                    Image(systemName: "arrow.down.circle")
                        .font(.title3)
                }
                .buttonStyle(.borderless)
                .help("下载 \(lesson.title)（\(course.tracksText)）")
                .accessibilityLabel("下载 \(lesson.title)")
            } else {
                Text("尚未开放")
                    .font(.callout)
                    .foregroundStyle(.tertiary)
            }
        }
        .padding(.vertical, 4)
        .opacity(lesson.available ? 1 : 0.6)
    }

    private var meta: String {
        var parts = [Format.lessonTime(begin: lesson.beginTime, end: lesson.endTime)]
        parts.append(lesson.classroom.trimmed.isEmpty ? "教室未知" : lesson.classroom)
        if lesson.source == "historical" {
            parts.append("旧版录像")
        }
        return parts.filter { !$0.isEmpty }.joined(separator: " · ")
    }
}

// MARK: - Files

@MainActor
private struct FileListView: View {
    @Bindable var course: CourseModel

    var body: some View {
        let files = course.visibleFiles
        List(selection: $course.fileSelection) {
            ForEach(files) { file in
                FileRow(course: course, file: file)
                    .tag(file.id)
            }
        }
        .contextMenu(forSelectionType: String.self) { ids in
            let chosen = course.files.filter { ids.contains($0.id) }
            if !chosen.isEmpty {
                Button(chosen.count == 1 ? "下载这个文件" : "下载 \(chosen.count) 个文件") {
                    course.fileSelection = Set(chosen.map(\.id))
                    course.downloadSelection()
                }
            }
        } primaryAction: { ids in
            if let file = course.files.first(where: { ids.contains($0.id) }) {
                course.download(file)
            }
        }
        .overlay {
            if files.isEmpty {
                if course.isLoadingFiles {
                    ProgressView()
                } else if course.filesError == nil {
                    if !course.query.trimmed.isEmpty {
                        ContentUnavailableView.search(text: course.query)
                    } else {
                        ContentUnavailableView(
                            "还没有课程文件",
                            systemImage: "doc",
                            description: Text("教师发布资料后会出现在这里。")
                        )
                    }
                }
            }
        }
    }
}

@MainActor
private struct FileRow: View {
    let course: CourseModel
    let file: CourseFile

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: Format.fileSymbol(contentType: file.contentType, name: file.filename))
                .font(.title2)
                .foregroundStyle(.secondary)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 3) {
                Text(file.name)
                    .font(.headline)
                    .lineLimit(1)
                if !meta.isEmpty {
                    Text(meta)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 12)
            Text(Format.size(file.size))
                .font(.callout)
                .foregroundStyle(.secondary)
                .monospacedDigit()
            Button {
                course.download(file)
            } label: {
                Image(systemName: "arrow.down.circle")
                    .font(.title3)
            }
            .buttonStyle(.borderless)
            .help("下载 \(file.name)")
            .accessibilityLabel("下载 \(file.name)")
        }
        .padding(.vertical, 4)
    }

    /// The file name when it differs from the title, and when it was updated.
    private var meta: String {
        var parts: [String] = []
        if file.filename != file.name {
            parts.append(file.filename)
        }
        if let text = file.updatedAt, let date = EngineDate.parse(text) {
            parts.append("更新于 \(Format.relative(date))")
        }
        return parts.joined(separator: " · ")
    }
}

// MARK: - Selection bar

@MainActor
private struct SelectionBar: View {
    let course: CourseModel

    var body: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: 12) {
                Toggle("全选", isOn: Binding(
                    get: { course.isAllSelected },
                    set: { _ in course.toggleSelectAll() }
                ))
                .toggleStyle(.checkbox)
                .disabled(course.tab == .lessons ? !course.lessons.contains(where: \.available) : course.files.isEmpty)
                Text(course.selectionSummary)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                Spacer()
                Button {
                    course.downloadSelection()
                } label: {
                    Label(course.downloadTitle, systemImage: "arrow.down.circle")
                }
                .buttonStyle(.borderedProminent)
                .disabled(!course.canDownloadSelection)
                .keyboardShortcut(.return, modifiers: .command)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        }
        .background(.bar)
    }
}

// MARK: - Notices

/// An inline message above the list, like an InfoBar.
@MainActor
struct Notice<Actions: View>: View {
    enum Style {
        case info, success, warning, error

        var symbol: String {
            switch self {
            case .info: return "info.circle.fill"
            case .success: return "checkmark.circle.fill"
            case .warning: return "exclamationmark.triangle.fill"
            case .error: return "xmark.octagon.fill"
            }
        }

        var color: Color {
            switch self {
            case .info: return .accentColor
            case .success: return .green
            case .warning: return .orange
            case .error: return .red
            }
        }
    }

    let style: Style
    let title: String
    let message: String
    var onClose: (() -> Void)?
    @ViewBuilder let actions: () -> Actions

    init(style: Style, title: String, message: String, onClose: (() -> Void)? = nil, @ViewBuilder actions: @escaping () -> Actions) {
        self.style = style
        self.title = title
        self.message = message
        self.onClose = onClose
        self.actions = actions
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: style.symbol)
                .foregroundStyle(style.color)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.headline)
                Text(message)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
            actions()
            if let onClose {
                Button {
                    onClose()
                } label: {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.borderless)
                .help("关闭")
            }
        }
        .padding(12)
        .background(style.color.opacity(0.1), in: RoundedRectangle(cornerRadius: 8))
        .padding(.horizontal, 16)
        .padding(.top, 10)
    }
}
