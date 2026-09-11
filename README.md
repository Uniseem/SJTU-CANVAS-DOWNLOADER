# SJTU Canvas Downloader

SJTU Canvas Downloader 是 Windows 与 macOS 上的上海交通大学 Canvas 下载工具。用“交我办”扫码登录后，按课程浏览课堂录像和课程文件，一次选好要的讲次和画面，交给下载列表排队下载；可以随时暂停，关掉应用后下次打开从断点继续。

- **课堂录像**：电脑屏幕、教室摄像头（有的课程还有合成画面）分别保存为 MP4。自动适配 2026 年 8 月上线的新视频平台、原有的课堂视频接口，以及 Canvas 中的“课堂视频旧版”历史录像；列表会显示每讲所选画面的大小。
- **课程文件**：课件、讲义、作业说明等 Canvas 文件，完整分页读取。
- **下载列表**：同时下载 1–8 个文件，显示进度、速度和剩余时间；支持暂停、继续、取消、重试，断点续传；已下载过的内容不会重复下载，已有的文件从不覆盖。

两个应用都是原生界面：Windows 版使用 WinUI 3 与 Fluent Design，macOS 版使用 SwiftUI 并遵循 Apple 人机界面指南。它们共享同一个用 Rust 编写的下载引擎，设置与下载规则完全一致。

本应用与上海交通大学、Instructure、jAccount 均无隶属或官方合作关系。请只下载你有权访问的教学资料，并遵守学校、任课教师和 Canvas 平台的规定。

## 系统要求

| | Windows | macOS |
| --- | --- | --- |
| 系统 | Windows 10 2004（19041）或更高版本，x64 | macOS 14 Sonoma 或更高版本，Apple 芯片或 Intel |
| 其他 | 无需另装 .NET、Windows App SDK 或 Visual C++ 运行库 | — |
| 磁盘 | 应用约 240 MB，另需下载文件的空间 | 应用约 20 MB，另需下载文件的空间 |

需要能访问 Canvas（`oc.sjtu.edu.cn`）、jAccount 和学校的课堂视频服务。

## 安装

目前还没有正式发行版：可以从仓库 [Actions](https://github.com/Uniseem/SJTU-CANVAS-DOWNLOADER/actions/workflows/desktop.yml) 中最近一次成功构建的产物（Artifacts）下载，或按下文[从源码构建](#构建)。Windows 用 `SJTUCanvasDownloader-win-x64.zip`，Mac 用 `SJTUCanvasDownloader-macos-arm64.pkg`（Apple 芯片）或 `SJTUCanvasDownloader-macos-x86_64.pkg`（Intel 芯片）。

### Windows

解压 `SJTUCanvasDownloader-win-x64.zip`，运行其中的 `SJTUCanvasDownloader.exe`。不需要管理员权限，可以放在任何文件夹，包括含中文的路径。

没有代码签名的构建第一次运行时，Windows 可能显示“Windows 已保护你的电脑”：点“更多信息 → 仍要运行”即可，之后不再询问。

### macOS

双击 `SJTUCanvasDownloader-macos-<架构>.pkg`，按提示安装到“应用程序”文件夹。下错了架构，安装器会直接提示该下载哪一个。

经过 Apple 公证的安装包可以直接打开。**未公证的安装包**第一次打开时，macOS 会提示“无法验证开发者”（macOS 15 显示“未打开”）——这是 macOS 对所有未公证软件的统一提示，手动允许一次即可：

- macOS 15 及以后：双击安装包，在提示中点“完成”；打开“系统设置 → 隐私与安全性”，在页面下方点“仍要打开”并输入登录密码，再点“打开”。
- macOS 14：按住 Control 点按安装包，选“打开”，再点“打开”。

之后安装器会把 SJTU Canvas Downloader 放进“应用程序”文件夹。由安装器安装的应用不带下载隔离标记，打开时不会再有任何提示，也不会出现“已损坏”之类的错误。更新到新版本后，如果系统询问是否允许应用使用钥匙串中的“SJTU Canvas Downloader 登录密钥”，请选择“始终允许”。

## 使用

1. **登录**：打开应用，用手机上的“交我办”扫描窗口中的二维码，并在手机上确认。应用不会读取、输入或保存 jAccount 密码和短信验证码。登录状态加密保存在本机，下次打开不用再扫码；登录过期时会自动回到扫码页。
2. **选择课程**：课程分为本学期和已结束的课程，可以搜索课程名、课程代码和教师。
3. **选择内容**：课程中分“课堂录像”和“课程文件”两栏。勾选需要的讲次或文件（也可以全选），在“画面”中选择要下载电脑屏幕、教室摄像头还是合成画面，点“下载”；每一行也有单独的下载按钮。录像的大小会在列表中逐行显示。尚未开放的讲次不能选择。
4. **查看下载**：下载列表按“全部 / 进行中 / 已完成 / 失败与取消”分类，显示进度、速度和剩余时间。可以暂停、继续、取消（删除已下载的部分）、重新下载，完成后直接打开文件或在文件夹中显示。

有未完成的下载时退出应用，会先询问一次；下次打开应用时，下载从断点继续。在 macOS 上关闭窗口不会停止下载，按 ⌘Q 退出时才会停止；在 Windows 上关闭窗口即退出应用。在后台下载完成时，应用会发送系统通知。

默认使用系统的代理设置。全局代理或 VPN 导致无法访问学校服务时，在“设置”中选择“不使用代理”，或填写自定义代理（HTTP、HTTPS、SOCKS5）。

### 下载的文件放在哪里

默认保存在“下载”文件夹里的 `SJTU Canvas` 中，可以在设置中更改，或打开“每次下载前选择保存位置”。文件按课程和讲次整理：

```text
SJTU Canvas/
  常微分方程 [87084]/
    课堂录像/
      2026-09-01_08-00 第 01 讲 [讲次标识]/
        电脑屏幕.mp4
        教室摄像头.mp4
    课程文件/
      讲义.pdf
      讲义 (2).pdf
```

- 下载中的文件带 `.part` 后缀，完成后改为正式文件名；暂停或退出后保留，继续时用 HTTP Range 从断点接着下载。
- 已有的文件从不覆盖：同名文件会在名称后加序号。已经下载完成且文件还在的内容，再次选择时会跳过；删除了文件，或从下载列表中移除了任务，才会重新下载。
- 文件名会去掉路径分隔符和 Windows 保留名称等非法字符，过长的名称会截短，讲次标识用来区分同名录像。
- 某个画面不存在时，只有这个任务失败并说明原因，其余画面照常下载，不会用另一个画面代替。

### 数据保存在哪里

| | Windows | macOS |
| --- | --- | --- |
| 应用数据 | `%LOCALAPPDATA%\SJTU Canvas Downloader` | `~/Library/Application Support/SJTU Canvas Downloader` |
| 登录密钥 | Windows 凭据管理器（`SJTU Canvas Downloader/session-key`） | 钥匙串（服务 `SJTU Canvas Downloader`，账户 `session-key`） |

```text
SJTU Canvas Downloader/
├── downloads.db   # SQLite：下载任务与设置
├── session.bin    # 加密的 Canvas 登录状态
└── logs/          # 引擎日志（engine.log）和应用日志
```

Canvas 的登录 Cookie 用 ChaCha20-Poly1305 加密后才写入 `session.bin`，密钥只保存在系统的凭据存储中，由应用在内存中交给引擎；jAccount 的 Cookie 从不保存。退出登录会删除保存的登录状态，未完成的下载会暂停，下次登录同一账户后继续。

## 课堂视频接口

引擎根据 Canvas 课程中“课堂视频”工具的授权表单自动选择接口：

- **新视频平台（2026 年 8 月起）**：`jy-lti-adapter` → `jwt-token` → `jy-application-resourcemanage/lms/launch-context` 取得教学班，再用 `/v1/subject_vod_list_new` 读取讲次、`/v1/course_vod_urls_new` 读取各画面的地址。按教学班完整分页，未开放的讲次不可下载。JWT 只在引擎内使用。
- **原课堂视频接口**：`v.sjtu.edu.cn/jy-application-canvas-sjtu` 的 LTI 流程，仍适用于尚未迁移的课程。
- **课堂视频旧版**：新平台没有这门课的录像、没有排课映射、网关故障或超时的时候，自动使用课程导航中实际存在的“课堂视频旧版”工具（`courses.sjtu.edu.cn/lti`）读取历史录像，并在列表中标注来源。明确的登录或权限拒绝不会触发回退。

Canvas 接口返回 401 时，引擎会先用个人信息接口复核登录（最多 8 秒）：身份有效说明只是没有这门课的权限；确认过期或账户不符才退出登录；暂时无法复核时保留登录并提示重试。视频服务连续出错时会短暂熔断，并在界面上倒计时后自动重试一次。

每个下载任务在真正开始传输时才向学校取一次新的媒体地址，所以排在长队列后面的任务不会因签名地址过期而失败；地址在传输中过期或被拒绝时会重新获取一次。下载只接受课程、文件、讲次和画面的 ID，不接受外部提供的 URL；学校返回的地址和每一次重定向都会重新检查：只允许 HTTPS，拒绝带用户名密码的地址和指向本机或内网的 IP 地址。视频服务的 `token` 请求头只发给签发它的主机。

## 构建

两个平台的构建脚本都会先构建下载引擎，再构建应用并组装成可直接运行的应用。

### Windows

需要 Rust（MSVC 工具链）、.NET 10 SDK 和 Python 3（构建时的 DLL 检查）。在 PowerShell 中：

```powershell
./apps/windows/build.ps1 -Zip
```

产物是 `apps/windows/dist/SJTUCanvasDownloader/SJTUCanvasDownloader.exe`（自包含，无需安装 .NET），`-Zip` 另外生成 `SJTUCanvasDownloader-win-x64.zip`。脚本会：

- 以静态 C 运行库链接引擎，自包含发布 .NET 与 Windows App SDK；
- 检查应用和引擎中每个 `.exe/.dll` 的依赖都能在干净的 Windows 上找到（`apps/windows/tools/check-dlls.py`），否则构建失败；
- 可选地用 Authenticode 签名所有未签名的二进制文件：`-SignPfx 证书.pfx`（密码放在 `SJTU_CANVAS_SIGN_PASSWORD`）或 `-SignThumbprint <证书指纹>`，使用 RFC 3161 时间戳。

`-Arch arm64` 构建 ARM64 版（需要 Rust 的 `aarch64-pc-windows-msvc` 目标和 ARM64 生成工具）。

### macOS

需要 Xcode 15.3 或更高版本（或对应的 Command Line Tools）和 Rust：

```bash
bash apps/macos/build.sh --pkg
```

产物是 `apps/macos/dist/SJTU Canvas Downloader.app`，`--pkg` 另外生成安装包 `SJTUCanvasDownloader-macos-<arch>.pkg`（没有 Developer ID 时推荐用它分发）；`--dmg` 和 `--zip` 生成磁盘映像和 zip。默认为当前 Mac 的架构构建，`--arch x86_64` 或 `--arch arm64` 指定架构。

- 构建会检查应用和引擎要求的最低系统版本不高于 macOS 14。
- 没有 Developer ID 时使用临时（ad-hoc）签名：引擎和应用由内向外签名并封存，构建时严格校验；安装包只包含与签名时完全一致的应用（打包后会再校验一次）。“已损坏”只会出现在签名无效的应用上，这些检查保证构建出的应用签名有效。
- 有 Apple Developer ID 时，可以签名并公证，用户打开时没有任何提示：

  ```bash
  xcrun notarytool store-credentials sjtu-canvas --apple-id <Apple ID> --team-id <团队 ID> --password <App 专用密码>
  bash apps/macos/build.sh --sign "Developer ID Application: 姓名 (团队 ID)" \
      --installer-sign "Developer ID Installer: 姓名 (团队 ID)" --notarize sjtu-canvas --pkg --dmg
  ```

`.github/workflows/desktop.yml` 在 macOS（arm64 与 x86_64）和 Windows 上运行引擎测试并构建两个应用；配置了签名相关的仓库机密时自动签名和公证，没有时 macOS 只生成未签名的安装包，Windows 生成未签名的 zip。发布新版本时，同时修改 `engine/Cargo.toml` 与 `apps/windows/SJTUCanvasDownloader/SJTUCanvasDownloader.csproj` 中的版本号。

## 架构

```text
apps/windows/   WinUI 3 应用（.NET 10、Windows App SDK、Fluent Design）
  tools/          构建检查与界面自动化测试脚本
apps/macos/     SwiftUI 应用（macOS 14+，Swift Package + 组装 .app 的脚本）
apps/shared/    应用图标的生成脚本
engine/         sjtu-canvas-engine：Rust 编写的下载引擎
  src/            扫码登录、Canvas、课堂视频、下载管理、设置、JSON-RPC
  migrations/     SQLite 结构
  tests/          端到端测试与模拟媒体服务
```

- **引擎**（`sjtu-canvas-engine`）是应用启动的子进程，通过标准输入输出上逐行的 JSON-RPC 通信，日志写入数据目录的 `logs/`。标准输入关闭时引擎暂停正在进行的下载（保留 `.part` 文件）并退出，因此不会在应用退出后残留；Windows 版另外用作业对象确保应用崩溃时引擎一并结束。
- **扫码登录**：引擎通过 jAccount 的 WebSocket 接收二维码更新和确认结果，完成 Canvas 的 OpenID Connect 登录，把二维码以 PNG 推送给应用。
- **下载管理**：任务保存在 SQLite（WAL 模式）中，调度器按设置的并发数启动传输；每次传输前重新解析媒体地址，写入 `.part` 文件，用 `Range` 与 `If-Range`（强 ETag）续传；服务器忽略 Range 时从头下载。网络中断和临时故障退避后重试，最多 3 次。进度每 0.5 秒推送一次，每 5 秒写入数据库。
- **应用**：Windows 版把登录密钥保存在凭据管理器，导航栏的“下载”上显示进行中的任务数，窗口不在前台时下载结束会发送通知；macOS 版把密钥保存在钥匙串，Dock 图标显示进行中的任务数，下载期间阻止 App Nap，应用不在前台时下载结束会发送通知。

### 引擎协议

请求 `{"id": 1, "method": "courses.list", "params": {…}}`，响应 `{"id": 1, "result": …}` 或 `{"id": 1, "error": {"code": "…", "message": "…", "retry_after_seconds": 30}}`；引擎另外推送没有 `id` 的通知：`login.status`、`account.changed`、`download.changed`、`download.progress`、`download.removed`。当前协议版本为 1。

| 方法 | 作用 |
| --- | --- |
| `engine.initialize` | 传入登录密钥和系统“下载”文件夹，启动调度器，返回版本、设置和账户 |
| `engine.shutdown` | 停止引擎（关闭标准输入效果相同） |
| `settings.get` / `settings.update` | 读取、修改设置（保存位置、每次询问、并发数、默认画面、代理） |
| `login.start` / `login.refresh` / `login.cancel` / `login.status` | 开始或加入扫码登录、换一个二维码、取消、读取状态 |
| `account.get` / `account.logout` | 读取账户（`verify` 时向 Canvas 复核）、退出登录 |
| `courses.list` / `courses.files` / `courses.lessons` | 课程列表、课程文件、课堂录像 |
| `lessons.sizes` | 某一讲各画面的大小 |
| `downloads.create` | 新建下载任务（视频按讲次与画面、文件按文件 ID），返回新建与跳过的项目 |
| `downloads.list` / `downloads.get` | 任务列表（筛选、搜索）与计数、单个任务 |
| `downloads.pause` / `resume` / `cancel` / `retry` / `remove` | 暂停、继续、取消、重新下载、从列表中移除 |
| `downloads.pauseAll` / `resumeAll` / `clearCompleted` | 全部暂停、全部继续、清除已完成的任务 |

## 开发与测试

```bash
cargo test --manifest-path engine/Cargo.toml          # 引擎单元测试（登录复核、视频接口、命名、断点续传、取消、重试…）
cargo build --manifest-path engine/Cargo.toml
python engine/tests/e2e.py --engine engine/target/debug/sjtu-canvas-engine
```

`e2e.py` 通过 JSON-RPC 驱动真实的引擎，用演示学校和本地的模拟媒体服务（`engine/tests/mock_media.py`，支持 Range、ETag 和限速）走完扫码登录、课程、大小、下载、暂停与 Range 续传、取消、设置校验、中途退出后恢复和退出登录，不访问学校服务。

调试版引擎在 `SJTU_CANVAS_TEST_MODE=1` 时提供测试模式（发行版没有）：`SJTU_CANVAS_FAKE_SCHOOL=1` 使用演示课程、录像和文件，扫码几秒后自动登录（`SJTU_CANVAS_FAKE_LOGIN_SECONDS`）；`SJTU_CANVAS_FAKE_MEDIA=http://127.0.0.1:8765` 让演示下载来自 `python engine/tests/mock_media.py 8765`，`SJTU_CANVAS_FAKE_RATE` 限制速度。学校的各个服务地址也可以用 `SJTU_CANVAS_CANVAS_ORIGIN`、`SJTU_CANVAS_VIDEO_API` 等变量指向本地模拟服务。

开发时可以直接运行应用：Windows 版 `dotnet build` 后运行，macOS 版在 `apps/macos` 中执行 `swift run`。调试版应用优先使用 `engine/target/debug` 下的引擎；`SJTU_CANVAS_ENGINE`、`SJTU_CANVAS_DATA_DIR` 和 `SJTU_CANVAS_DOWNLOADS_FOLDER` 分别指定引擎、数据目录和“下载”文件夹，`SJTU_CANVAS_SESSION_KEY` 用固定的测试密钥代替凭据管理器或钥匙串。`apps/windows/tools/uia.ps1` 与 `capture-window.ps1` 用 UI 自动化操作和截取应用窗口（不移动鼠标、不抢焦点），用于界面测试。

## 许可

SJTU Canvas Downloader 以 [MIT 许可证](LICENSE)发布。引擎使用的 Rust 库、Windows 版内置的 .NET 与 Windows App SDK 各自保留其许可证，详见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。扫码登录、LTI 与课堂视频的实现参考了 [canvas-downloader](https://github.com/FengYuchen1314/canvas-downloader)（MIT）公开的行为，课程与文件浏览参考了 [canvas-sjtu-skill](https://github.com/Neko-Yukari/canvas-sjtu-skill) 的公开文档；后者在调研时没有许可证，因此没有复制其源码。
