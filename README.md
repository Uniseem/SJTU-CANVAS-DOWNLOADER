# SJTU Canvas Downloader

上海交通大学 Canvas（oc.sjtu.edu.cn）课堂录像与课程文件下载器，Windows 与 macOS 桌面应用。

## 功能

- 交我办扫码登录 Canvas，登录状态加密保存在本机，不读取也不保存 jAccount 密码。
- 浏览本学期和历史课程；每门课程列出新版课堂视频平台（v.sjtu.edu.cn）上已开放的录像和课程文件。
- 按讲次批量下载录像，可选电脑屏幕、教室摄像头和合成画面；下载前显示文件大小。
- 下载管理：暂停、继续、取消、重试，断点续传，可调整同时下载的文件数以跑满带宽。
- 文件按“课程名 [课程号]/课堂录像/日期 讲次/”和“课程文件”整理，已有文件不会被覆盖。

> Canvas 已切换到新版课堂视频平台，本版本只支持新版接口；旧版 canvas-sjtu API 和“课堂视频旧版”入口不再支持。

## 安装

### Windows

到 [Releases](https://github.com/Uniseem/SJTU-CANVAS-DOWNLOADER/releases/latest) 下载 `SJTUCanvasDownloader-win-x64-setup.exe`，运行后按提示安装（不需要管理员权限）。没有代码签名，Windows 可能提示“Windows 已保护你的电脑”，点“更多信息 → 仍要运行”即可。

### macOS

没有购买 Apple 开发者证书，请用命令行安装。打开“终端”，粘贴并运行：

```bash
curl -fsSL https://raw.githubusercontent.com/Uniseem/SJTU-CANVAS-DOWNLOADER/main/install.sh | bash
```

脚本会按这台 Mac 的芯片下载对应版本，校验后放进“应用程序”文件夹并打开。再次运行即可更新，下载记录和设置会保留。需要 macOS 12 或更新版本。

安装指定版本：`SJTU_CANVAS_VERSION=v2.0.0 bash -c "$(curl -fsSL https://raw.githubusercontent.com/Uniseem/SJTU-CANVAS-DOWNLOADER/main/install.sh)"`。

## 使用

1. 打开应用，用交我办扫描二维码并在手机上确认。
2. 在“课程”里打开一门课，勾选讲次（或切换到“课程文件”勾选文件），选择要下载的画面，点“下载”。
3. 在“下载”里查看进度；“设置”里可以更改保存位置、同时下载数、默认画面和代理。

数据目录（下载列表、设置、加密的登录状态和日志）：Windows 在 `%LOCALAPPDATA%\SJTU Canvas Downloader`，macOS 在 `~/Library/Application Support/SJTU Canvas Downloader`。

## 开发

项目由两部分组成：

- `engine/`：Rust 下载引擎，负责登录、Canvas 与课堂视频平台的接口、下载队列。应用通过 stdin/stdout 上的 JSON-RPC 驱动它（协议见 `engine/src/rpc.rs`）。
- `apps/desktop/`：Electron + React + HeroUI 的桌面应用。主进程启动引擎并转发请求，渲染进程是界面。

需要 Rust（stable）和 Node.js 22。

```bash
cargo build --manifest-path engine/Cargo.toml     # 调试版引擎（含演示模式）
cd apps/desktop
npm install
npm run dev:fake     # 用引擎的演示学校运行，不需要交大账号
npm run dev          # 连接真实的学校服务
```

检查与测试：

```bash
cargo test --manifest-path engine/Cargo.toml
cd apps/desktop && npm run typecheck
npm run build && npm run test:ui   # 对演示学校的端到端测试，截图在 apps/desktop/dist/smoke/
```

## 打包

```bash
cd apps/desktop
npm run build:engine   # 发布版引擎 → resources/engine/
npm run dist:win       # Windows：dist/SJTUCanvasDownloader-win-x64-setup.exe
npm run dist:mac       # macOS：dist/SJTUCanvasDownloader-macos-<arm64|x86_64>.zip
```

`.github/workflows/desktop.yml` 在 GitHub Actions 上构建两个平台，推送 `vX.Y.Z` 标签（与 `apps/desktop/package.json` 和 `engine/Cargo.toml` 的版本一致）会发布 Release。代码签名可选，见工作流文件开头的说明。

## 许可

SJTU Canvas Downloader 以 [MIT 许可证](LICENSE)发布。使用的第三方组件见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。扫码登录、LTI 与课堂视频的实现参考了 [canvas-downloader](https://github.com/FengYuchen1314/canvas-downloader)（MIT）公开的行为，课程与文件浏览参考了 [canvas-sjtu-skill](https://github.com/Neko-Yukari/canvas-sjtu-skill) 的公开文档。
