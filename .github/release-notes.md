## 安装

**macOS**（macOS 14 或更新版本，Apple 芯片或 Intel）：在“终端”中运行

```bash
curl -fsSL https://raw.githubusercontent.com/Uniseem/SJTU-CANVAS-DOWNLOADER/main/install.sh | bash
```

脚本会按这台 Mac 的芯片下载对应的安装包，校验 SHA-256 后安装到“应用程序”文件夹（需要输入登录密码），装好后可以直接打开。也可以手动下载 `SJTUCanvasDownloader-macos-arm64.pkg`（Apple 芯片）或 `SJTUCanvasDownloader-macos-x86_64.pkg`（Intel 芯片）双击安装；未公证的安装包第一次打开时需要在“系统设置 → 隐私与安全性”中点“仍要打开”。

**Windows**（Windows 10 2004 或更新版本，x64）：下载并运行 `SJTUCanvasDownloader-win-x64-setup.exe`，按提示安装。不需要管理员权限，也不需要另装 .NET。没有代码签名时 Windows 可能提示“Windows 已保护你的电脑”，点“更多信息 → 仍要运行”即可。

`SHA256SUMS.txt` 列出了每个文件的 SHA-256 校验值。使用说明见 [README](https://github.com/Uniseem/SJTU-CANVAS-DOWNLOADER#readme)。
