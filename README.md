# SJTU Canvas Downloader

## 功能说明
交我办扫码后可获取本学期课程和历史课程，进入相应课程后可下载对应的录像。

后台可以修改下载的并发数目，以跑满带宽。

## 安装

### Windows

下载 Release 相应文件，然后无视风险继续安装

### macOS

没买证书，必须使用命令安装。

打开“终端”，粘贴并运行：

```bash
curl -fsSL https://raw.githubusercontent.com/Uniseem/SJTU-CANVAS-DOWNLOADER/main/install.sh | bash
```

之后需要手动输入电脑密码，之后就能用了。


## 许可

SJTU Canvas Downloader 以 [MIT 许可证](LICENSE)发布。引擎使用的 Rust 库、Windows 版内置的 .NET 与 Windows App SDK 各自保留其许可证，详见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。扫码登录、LTI 与课堂视频的实现参考了 [canvas-downloader](https://github.com/FengYuchen1314/canvas-downloader)（MIT）公开的行为，课程与文件浏览参考了 [canvas-sjtu-skill](https://github.com/Neko-Yukari/canvas-sjtu-skill) 的公开文档。
