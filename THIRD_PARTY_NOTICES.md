# 第三方组件

SJTU Canvas Downloader 以 [MIT 许可证](LICENSE)发布。它使用了下列第三方组件，各自保留其许可证。

## 桌面应用（apps/desktop）

| 组件 | 许可证 | 用途 |
| --- | --- | --- |
| [Electron](https://www.electronjs.org/)（含 Chromium、Node.js） | MIT（Chromium 为 BSD 等许可证，见应用内 `LICENSES.chromium.html`） | 窗口与运行时 |
| [React](https://react.dev/)、react-dom | MIT | 界面 |
| [HeroUI](https://heroui.com/)（`@heroui/react`、`@heroui/styles`） | MIT | 界面组件与主题 |
| [React Aria Components](https://react-spectrum.adobe.com/react-aria/) | Apache-2.0 | 无障碍交互（HeroUI 的基础） |
| [Tailwind CSS](https://tailwindcss.com/) | MIT | 样式 |
| [Lucide](https://lucide.dev/) | ISC | 图标 |
| [Zustand](https://github.com/pmndrs/zustand) | MIT | 状态管理 |
| [electron-vite](https://electron-vite.org/)、[Vite](https://vite.dev/)、[electron-builder](https://www.electron.build/)、TypeScript、Playwright | MIT / Apache-2.0 | 构建、打包与测试（不随应用分发） |

完整的依赖清单见 `apps/desktop/package-lock.json`；打包后的应用在 `resources/app.asar` 中附带各包的许可证文件。

## 下载引擎（engine）

引擎是一个 Rust 程序，静态链接了下列主要库（许可证均为 MIT 或 Apache-2.0 双许可，除特别说明）：

| 库 | 用途 |
| --- | --- |
| tokio、tokio-util、futures-util | 异步运行时 |
| reqwest、tokio-tungstenite（Windows 上使用系统的 SChannel，其他平台使用 rustls 与 webpki-roots） | HTTP 与 WebSocket |
| sqlx（SQLite） | 下载列表与设置的数据库 |
| serde、serde_json | JSON-RPC 协议 |
| scraper（及 html5ever、selectors，MPL-2.0） | 解析 Canvas 页面 |
| chacha20poly1305、sha2、rand | 登录状态加密 |
| chrono、url、uuid、base64、urlencoding、unicode-normalization、dashmap、anyhow、thiserror、tracing | 工具库 |

完整清单及版本见 `engine/Cargo.lock`。

## 参考

扫码登录、LTI 与课堂视频的实现参考了 [canvas-downloader](https://github.com/FengYuchen1314/canvas-downloader)（MIT）公开的行为，课程与文件浏览参考了 [canvas-sjtu-skill](https://github.com/Neko-Yukari/canvas-sjtu-skill) 的公开文档。
