## 安装

**Windows**（Windows 10 或更新版本，x64）：下载并运行 `SJTUCanvasDownloader-win-x64-setup.exe`，按提示安装。不需要管理员权限。没有代码签名时 Windows 可能提示“Windows 已保护你的电脑”，点“更多信息 → 仍要运行”即可。

**macOS**（macOS 12 或更新版本，Apple 芯片或 Intel）：在“终端”中运行

```bash
curl -fsSL https://raw.githubusercontent.com/Uniseem/SJTU-CANVAS-DOWNLOADER/main/install.sh | bash
```

脚本会按这台 Mac 的芯片下载对应的 `SJTUCanvasDownloader-macos-arm64.zip`（Apple 芯片）或 `SJTUCanvasDownloader-macos-x86_64.zip`（Intel 芯片），校验 SHA-256 后放进“应用程序”文件夹并打开。应用没有经过 Apple 公证，直接用浏览器下载解压的话，第一次打开需要在“系统设置 → 隐私与安全性”中点“仍要打开”。

`SHA256SUMS.txt` 列出了每个文件的 SHA-256 校验值。使用说明见 [README](https://github.com/Uniseem/SJTU-CANVAS-DOWNLOADER#readme)。
