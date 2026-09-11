import AppKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationWillFinishLaunching(_ notification: Notification) {
        // A write to an engine that just exited must fail, not kill the app.
        signal(SIGPIPE, SIG_IGN)
        NSWindow.allowsAutomaticWindowTabbing = false
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        Notifier.shared.activate {
            AppModel.shared.showDownloads()
        }
        AppModel.shared.start()
    }

    /// Downloads keep running with the window closed; the Dock icon brings
    /// the window back.
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag {
            AppModel.shared.showMainWindow()
        }
        return true
    }

    /// Asks before quitting with unfinished downloads, then stops the engine;
    /// the downloads resume from their partial files on the next launch.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        let model = AppModel.shared
        guard model.needsShutdown else { return .terminateNow }
        let running = model.counts.running
        if running > 0 {
            let alert = NSAlert()
            alert.messageText = "退出 SJTU Canvas Downloader？"
            alert.informativeText = "还有 \(running) 个下载任务未完成。退出后它们会暂停，下次打开应用时从断点继续。"
            alert.addButton(withTitle: "退出")
            alert.addButton(withTitle: "取消")
            if alert.runModal() != .alertFirstButtonReturn {
                return .terminateCancel
            }
        }
        Task {
            await model.shutdown()
            sender.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }
}
