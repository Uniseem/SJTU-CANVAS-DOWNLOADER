import SwiftUI

/// The main window: the login view while signed out, then the courses and
/// downloads in a split view.
@MainActor
struct MainView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        @Bindable var model = model
        content
            .alert(
                model.alert?.title ?? "",
                isPresented: $model.isAlertPresented,
                presenting: model.alert
            ) { _ in
                Button("好") {}
            } message: { alert in
                Text(alert.message)
            }
            .confirmationDialog(
                "退出 Canvas 登录？",
                isPresented: $model.isSignOutPresented,
                titleVisibility: .visible
            ) {
                Button("退出登录", role: .destructive) { model.signOut() }
                Button("取消", role: .cancel) {}
            } message: {
                Text(signOutMessage)
            }
            .onAppear {
                model.openMainWindow = { openWindow(id: "main") }
            }
    }

    @ViewBuilder
    private var content: some View {
        switch model.engineState {
        case .failed(let message):
            ContentUnavailableView {
                Label("下载引擎未运行", systemImage: "exclamationmark.triangle")
            } description: {
                Text(message)
                    .textSelection(.enabled)
            } actions: {
                Button("重新启动") { model.start() }
            }
        case .starting, .stopped:
            ProgressView("正在启动下载引擎…")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .ready:
            if model.account.authenticated {
                NavigationSplitView {
                    SidebarView()
                        .navigationSplitViewColumnWidth(min: 200, ideal: 240, max: 320)
                } detail: {
                    DetailView()
                }
            } else {
                LoginView()
            }
        }
    }

    private var signOutMessage: String {
        let running = model.counts.running
        return running > 0
            ? "还有 \(running) 个下载任务未完成，它们会在下次登录后继续。保存在本机的登录状态会被删除。"
            : "保存在本机的登录状态会被删除，下次使用需要重新扫码。"
    }
}

@MainActor
struct SidebarView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        List(selection: $model.selection) {
            Section {
                Label("下载", systemImage: "arrow.down.circle")
                    .badge(model.counts.running)
                    .tag(SidebarItem.downloads)
            }
            Section("本学期课程") {
                ForEach(model.currentCourses) { course in
                    CourseSidebarRow(course: course)
                        .tag(SidebarItem.course(course.id))
                }
            }
            if !model.finishedCourses.isEmpty {
                Section("已结束的课程", isExpanded: finishedExpanded) {
                    ForEach(model.finishedCourses) { course in
                        CourseSidebarRow(course: course)
                            .tag(SidebarItem.course(course.id))
                    }
                }
            }
        }
        .listStyle(.sidebar)
        .searchable(text: $model.courseQuery, placement: .sidebar, prompt: "搜索课程或教师")
        .overlay {
            if !model.courseQuery.trimmed.isEmpty && model.currentCourses.isEmpty && model.finishedCourses.isEmpty {
                ContentUnavailableView.search(text: model.courseQuery)
            } else if model.courses.isEmpty {
                if model.isLoadingCourses {
                    ProgressView()
                } else if let error = model.coursesError {
                    ContentUnavailableView {
                        Label("无法读取课程", systemImage: "exclamationmark.triangle")
                    } description: {
                        Text(error)
                    } actions: {
                        Button("重试") { model.refresh() }
                    }
                }
            }
        }
        .safeAreaInset(edge: .bottom) {
            AccountFooter()
        }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    model.refresh()
                } label: {
                    Label("刷新", systemImage: "arrow.clockwise")
                }
                .help("刷新课程列表（⌘R）")
                .disabled(model.isLoadingCourses)
            }
        }
    }

    /// Finished courses stay folded unless opened or found by a search.
    private var finishedExpanded: Binding<Bool> {
        Binding(
            get: { model.showsFinishedCourses || !model.courseQuery.trimmed.isEmpty },
            set: { model.showsFinishedCourses = $0 }
        )
    }
}

@MainActor
private struct CourseSidebarRow: View {
    let course: Course

    var body: some View {
        Label(course.name, systemImage: course.isCurrent ? "book.closed" : "archivebox")
            .lineLimit(1)
            .help([course.name, course.teacher ?? ""].filter { !$0.isEmpty }.joined(separator: " · "))
    }
}

/// The signed-in account at the bottom of the sidebar.
@MainActor
private struct AccountFooter: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let profile = model.account.profile
        HStack(spacing: 10) {
            ZStack {
                Circle()
                    .fill(Color.accentColor.opacity(0.18))
                Text(String((profile?.name ?? "").prefix(1)))
                    .font(.headline)
                    .foregroundStyle(Color.accentColor)
            }
            .frame(width: 30, height: 30)
            VStack(alignment: .leading, spacing: 1) {
                Text(profile?.name ?? "")
                    .font(.callout)
                    .lineLimit(1)
                Text(profile.map { "Canvas 用户 ID \($0.id)" } ?? "")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
            Menu {
                Button("退出 Canvas 登录…") { model.confirmSignOut() }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.button)
            .buttonStyle(.borderless)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("账户")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .accessibilityElement(children: .contain)
    }
}

@MainActor
struct DetailView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        switch model.selection {
        case .downloads:
            DownloadsView()
        case .course:
            if let current = model.current {
                CourseView(course: current)
                    .id(current.course.id)
            } else {
                placeholder
            }
        case nil:
            placeholder
        }
    }

    private var placeholder: some View {
        ContentUnavailableView(
            "选择一门课程",
            systemImage: "book.closed",
            description: Text("在左侧选择课程，浏览课堂录像和课程文件。")
        )
    }
}
