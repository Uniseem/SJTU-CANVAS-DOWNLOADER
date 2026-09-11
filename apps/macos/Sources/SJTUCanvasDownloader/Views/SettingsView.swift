import SwiftUI

/// The Settings window (⌘,). Preferences live in the engine, so the Windows
/// and macOS apps share their meaning.
@MainActor
struct SettingsView: View {
    var body: some View {
        TabView {
            GeneralSettings()
                .tabItem { Label("通用", systemImage: "gearshape") }
            NetworkSettings()
                .tabItem { Label("网络", systemImage: "network") }
            AccountSettings()
                .tabItem { Label("账户", systemImage: "person.crop.circle") }
        }
        .frame(width: 560)
        .frame(minHeight: 420)
    }
}

@MainActor
private struct GeneralSettings: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Form {
            if model.isDemo {
                Section {
                    Label("演示模式：课程、录像和文件都是演示数据，不会连接学校服务。", systemImage: "theatermasks")
                        .foregroundStyle(.orange)
                }
            }
            Section {
                LabeledContent("保存位置") {
                    VStack(alignment: .trailing, spacing: 6) {
                        Text(model.downloadDirectory)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .textSelection(.enabled)
                            .help(model.downloadDirectory)
                        HStack {
                            if !isDefaultDirectory {
                                Button("恢复默认") {
                                    model.setPreference(\.downloadDir, model.settings?.defaultDownloadDir ?? "")
                                }
                            }
                            Button("在访达中显示") { FileActions.openFolder(model.downloadDirectory) }
                            Button("更改…") { chooseDirectory() }
                        }
                    }
                }
                Toggle("每次下载前选择保存位置", isOn: preference(\.askDestination, default: false))
            } header: {
                Text("下载")
            } footer: {
                Text("课堂录像保存在“课程名 [课程号]/课堂录像/日期 讲次/”下，课程文件保存在“课程文件”文件夹。已有的文件不会被覆盖。")
                    .foregroundStyle(.secondary)
            }
            Section {
                Stepper(value: concurrency, in: 1...(model.settings?.concurrencyMax ?? 8)) {
                    LabeledContent("同时下载的文件数", value: "\(concurrency.wrappedValue)")
                }
            } footer: {
                Text("同时下载更多文件会占用更多带宽；学校服务繁忙时建议 2–3 个。")
                    .foregroundStyle(.secondary)
            }
            Section {
                ForEach(CourseModel.allTracks, id: \.self) { track in
                    Toggle(isOn: trackBinding(track)) {
                        Label(Format.trackLabel(track), systemImage: Format.trackSymbol(track))
                    }
                    .disabled(tracks.contains(track) && tracks.count == 1)
                }
            } header: {
                Text("默认下载的画面")
            } footer: {
                Text("打开课程时预先选中的画面，可以在每门课的工具栏里临时更改。合成画面并非每节课都有。")
                    .foregroundStyle(.secondary)
            }
            if let error = model.preferencesError {
                Section {
                    Label(error, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                }
            }
        }
        .formStyle(.grouped)
        .disabled(model.settings == nil)
    }

    private var isDefaultDirectory: Bool {
        guard let settings = model.settings else { return true }
        return settings.preferences.downloadDir == settings.defaultDownloadDir
    }

    private var tracks: [String] {
        model.settings?.preferences.defaultTracks ?? []
    }

    private var concurrency: Binding<Int> {
        preference(\.concurrency, default: 3)
    }

    private func preference<Value: Equatable>(_ keyPath: WritableKeyPath<Preferences, Value>, default fallback: Value) -> Binding<Value> {
        Binding(
            get: { model.settings?.preferences[keyPath: keyPath] ?? fallback },
            set: { model.setPreference(keyPath, $0) }
        )
    }

    private func trackBinding(_ track: String) -> Binding<Bool> {
        Binding(
            get: { tracks.contains(track) },
            set: { selected in
                var chosen = Set(tracks)
                if selected {
                    chosen.insert(track)
                } else if chosen.count > 1 {
                    chosen.remove(track)
                }
                model.setPreference(\.defaultTracks, CourseModel.allTracks.filter(chosen.contains))
            }
        )
    }

    private func chooseDirectory() {
        Task {
            if let folder = await FileActions.chooseFolder(
                message: "选择下载保存位置，课程和讲次会自动分文件夹保存。",
                prompt: "选择",
                startingAt: model.downloadDirectory
            ) {
                model.setPreference(\.downloadDir, folder.path)
            }
        }
    }
}

@MainActor
private struct NetworkSettings: View {
    @Environment(AppModel.self) private var model
    @State private var mode = "system"
    @State private var address = ""

    var body: some View {
        Form {
            Section {
                Picker("代理", selection: $mode) {
                    Text("跟随系统设置").tag("system")
                    Text("不使用代理").tag("direct")
                    Text("自定义代理").tag("custom")
                }
                .pickerStyle(.radioGroup)
                .onChange(of: mode) { _, newValue in
                    // A custom proxy is saved with its button once the address is typed.
                    if newValue != "custom" {
                        model.setPreference(\.proxy, ProxySettings(mode: newValue, url: nil))
                    }
                }
                if mode == "custom" {
                    HStack {
                        TextField("代理地址", text: $address, prompt: Text("http://127.0.0.1:7890"))
                            .onSubmit(apply)
                        Button("应用", action: apply)
                            .disabled(address.trimmed.isEmpty || address.trimmed == model.settings?.preferences.proxy.url)
                    }
                }
            } footer: {
                Text("学校服务一般直接连接即可；系统代理（如全局 VPN）导致无法访问时，可以选择“不使用代理”。自定义代理支持 http://、https:// 和 socks5:// 地址。")
                    .foregroundStyle(.secondary)
            }
            if let error = model.preferencesError {
                Section {
                    Label(error, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                }
            }
        }
        .formStyle(.grouped)
        .disabled(model.settings == nil)
        .onAppear(perform: load)
        .onChange(of: model.settings?.preferences.proxy) { load() }
    }

    private func load() {
        let proxy = model.settings?.preferences.proxy ?? ProxySettings()
        mode = proxy.mode
        address = proxy.url ?? ""
    }

    private func apply() {
        let url = address.trimmed
        guard !url.isEmpty else { return }
        model.setPreference(\.proxy, ProxySettings(mode: "custom", url: url))
    }
}

@MainActor
private struct AccountSettings: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Form {
            Section("Canvas 账户") {
                if model.account.authenticated, let profile = model.account.profile {
                    LabeledContent("姓名", value: profile.name)
                    LabeledContent("Canvas 用户 ID", value: profile.id)
                    Text(model.account.persisted
                        ? "登录状态已加密保存在本机，密钥保存在“钥匙串访问”中。"
                        : "登录状态仅在本次运行期间有效（无法使用钥匙串）。")
                        .foregroundStyle(.secondary)
                    Button("退出登录…") { model.confirmSignOut() }
                } else {
                    Text("未登录。在主窗口用“交我办”扫码登录后即可浏览课程和下载。")
                        .foregroundStyle(.secondary)
                }
            }
            Section("数据") {
                LabeledContent("数据位置") {
                    HStack {
                        Text(model.settings?.dataDir ?? DataLocation.current.path)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .textSelection(.enabled)
                        Button("在访达中显示") {
                            FileActions.openFolder(model.settings?.dataDir ?? DataLocation.current.path)
                        }
                    }
                }
                LabeledContent("版本", value: versionText)
            }
        }
        .formStyle(.grouped)
    }

    private var versionText: String {
        let app = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "开发版"
        return model.engineVersion.isEmpty ? "应用 \(app)" : "应用 \(app) · 引擎 \(model.engineVersion)"
    }
}
