import AppKit
import SwiftUI

/// Menu bar commands.
@MainActor
struct AppCommands: Commands {
    let model: AppModel

    var body: some Commands {
        CommandGroup(replacing: .appInfo) {
            Button("关于 SJTU Canvas Downloader") { model.showAbout() }
        }

        CommandGroup(after: .appSettings) {
            Button("退出 Canvas 登录…") { model.confirmSignOut() }
                .disabled(!model.isSignedIn)
        }

        CommandGroup(replacing: .newItem) {
            Button("刷新") { model.refresh() }
                .keyboardShortcut("r")
                .disabled(!model.isSignedIn)
        }

        CommandGroup(after: .newItem) {
            Divider()
            Button("在访达中显示下载文件夹") {
                FileActions.openFolder(model.downloadDirectory)
            }
            .disabled(model.downloadDirectory.isEmpty)
        }

        SidebarCommands()

        CommandMenu("下载") {
            Button("显示下载列表") { model.showDownloads() }
                .keyboardShortcut("l", modifiers: [.command, .option])
                .disabled(!model.isSignedIn)
            Divider()
            Button("全部暂停") { model.downloads.pauseAll() }
                .disabled(!model.downloads.canPauseAll)
            Button("全部继续") { model.downloads.resumeAll() }
                .disabled(!model.downloads.canResumeAll)
            Divider()
            Button("清除已完成的任务") { model.downloads.clearCompleted() }
                .disabled(!model.downloads.canClearCompleted)
        }

        CommandGroup(replacing: .help) {
            Button("SJTU Canvas Downloader 源代码与说明") {
                if let url = URL(string: "https://github.com/Uniseem/SJTU-CANVAS-DOWNLOADER") {
                    NSWorkspace.shared.open(url)
                }
            }
        }
    }
}
