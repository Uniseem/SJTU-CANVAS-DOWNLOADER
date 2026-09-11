import AppKit
import Observation

/// The QR login: the image the engine pushes and the status text.
@MainActor
@Observable
final class LoginModel {
    @ObservationIgnored weak var app: AppModel?

    private(set) var qrImage: NSImage?
    private(set) var message = "正在准备扫码登录…"
    private(set) var isBusy = true
    private(set) var isWaiting = false
    private(set) var isExpired = false
    private(set) var isFailed = false

    @ObservationIgnored private var attempt = ""
    @ObservationIgnored private var revision: Int64 = -1
    @ObservationIgnored private var generation = -1

    /// Starts a login, or joins the one already running.
    func start() async {
        guard let app else { return }
        do {
            let status: LoginStatus = try await app.call("login.start")
            apply(status)
        } catch {
            showFailure(error.localizedDescription)
        }
    }

    func refresh() {
        guard let app else { return }
        isBusy = true
        isFailed = false
        message = "正在获取新的二维码…"
        Task {
            do {
                let _: Empty = try await app.call("login.refresh")
            } catch let error as EngineError where error.code == "conflict" {
                // A refresh right after another one: the current code stays valid.
                message = error.message
                isBusy = false
            } catch {
                showFailure(error.localizedDescription)
            }
        }
    }

    private func showFailure(_ text: String) {
        isBusy = false
        isWaiting = false
        isExpired = false
        isFailed = true
        message = text
    }

    func apply(_ status: LoginStatus) {
        // Pushed and returned states can arrive out of order.
        if status.attemptId == attempt && (status.generation < generation || status.revision <= revision) {
            return
        }
        attempt = status.attemptId
        revision = status.revision
        generation = status.generation
        message = status.message
        isBusy = ["preparing", "reconnecting", "authorizing"].contains(status.state)
        isWaiting = status.state == "waiting"
        isExpired = status.state == "expired"
        isFailed = status.state == "error" || status.state == "cancelled"
        if status.state == "waiting", let png = status.qrPng, let data = Data(base64Encoded: png) {
            qrImage = NSImage(data: data)
        } else if !isExpired {
            qrImage = nil
        }
    }
}
