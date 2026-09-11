import AppKit
import SwiftUI

/// Signed out: the jAccount QR code to scan with the 交我办 app.
@MainActor
struct LoginView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let login = model.login
        ScrollView {
            VStack(spacing: 18) {
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .frame(width: 72, height: 72)
                    .accessibilityHidden(true)
                VStack(spacing: 6) {
                    Text("登录 Canvas")
                        .font(.largeTitle)
                        .fontWeight(.semibold)
                    Text("用“交我办”扫描二维码，并在手机上确认登录。")
                        .foregroundStyle(.secondary)
                }
                ZStack {
                    RoundedRectangle(cornerRadius: 12)
                        .fill(Color.white)
                    RoundedRectangle(cornerRadius: 12)
                        .strokeBorder(Color.gray.opacity(0.25), lineWidth: 1)
                    if let image = login.qrImage {
                        Image(nsImage: image)
                            .resizable()
                            .interpolation(.none)
                            .frame(width: 200, height: 200)
                            .opacity(login.isExpired ? 0.15 : 1)
                            .accessibilityLabel("交我办登录二维码")
                    } else if login.isBusy {
                        ProgressView()
                            .controlSize(.large)
                    }
                    if login.isExpired {
                        Label("二维码已过期", systemImage: "clock.arrow.circlepath")
                            .font(.headline)
                            .foregroundStyle(Color.black)
                    } else if login.isFailed {
                        Image(systemName: "exclamationmark.triangle")
                            .font(.largeTitle)
                            .foregroundStyle(.orange)
                    }
                }
                .frame(width: 240, height: 240)
                Text(login.message)
                    .font(.headline)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 380)
                    .textSelection(.enabled)
                Button {
                    login.refresh()
                } label: {
                    Label("刷新二维码", systemImage: "arrow.clockwise")
                }
                .buttonStyle(.bordered)
                .controlSize(.large)
                .disabled(login.isBusy)
                .keyboardShortcut("r")
                Text("扫码确认后会自动继续。本应用不会读取、输入或保存你的 jAccount 密码和短信验证码。")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 380)
                SettingsLink {
                    Text("网络与代理设置…")
                }
                .buttonStyle(.link)
            }
            .padding(40)
            .frame(maxWidth: .infinity)
        }
        .navigationTitle("SJTU Canvas Downloader")
    }
}
