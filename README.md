# Canvas Pocket Web

一个面向上海交通大学 Canvas 的自托管 Web 下载器。后端使用 Rust/Axum，前端使用 Vue 3、TypeScript 与 Vuetify；可通过 Docker 部署到 VPS。

它把 [canvas-sjtu-skill](https://github.com/Neko-Yukari/canvas-sjtu-skill) 的 Canvas REST 浏览思路与 [canvas-downloader](https://github.com/FengYuchen1314/canvas-downloader) 的扫码、LTI 课堂录像能力重构成了多用户 Web 服务。不是 Tauri 桌面程序的远程套壳。

## 已实现

- 交我办扫码登录；服务端从不读取或保存 jAccount 用户名、密码、短信验证码。
- 每个浏览器独立的扫码 actor、Cookie Jar、课堂视频短期会话与下载凭证；扫码启动带原子幂等与全局限流。
- 登录状态通过 SSE 推送，并每 3 秒查询一次状态兜底；进入/返回登录页会自动启动或重连。请求和二维码等待均有超时，已结束的扫码会话会重新创建。
- 课程权限不足不再等同于登录失效：Canvas 资源接口返回 401 时，会用个人身份接口复核（最长 8 秒）；身份有效则返回权限提示（403），核验暂时失败则保留会话并提示重试，只有确认过期或账号不符才退出。并发请求迟到的 401 不会重置新二维码或注销已经更新的登录会话。状态带递增 revision，避免旧查询覆盖新推送。
- Canvas 课程和课程文件按 Canvas `Link` 头完整分页；课程默认紧凑卡片，页面不再展示或请求作业、待办。
- 根据 Canvas 授权表单自动选择旧 LTI 或 2026 年 8 月的新资源管理视频接口；新接口按教学班完整分页，未开放课次不可下载。
- 新平台列表为空、没有排课映射、网关故障（502/503）、连接异常或响应超时（新源整体最多等待 20 秒）时，自动发现课程导航中的“课堂视频旧版”，通过独立 LTI 1.1 授权读取历史录像，并标注来源。明确的登录/权限拒绝不会触发回退；旧源仍独立核验课程权限。若旧源也失败、没有入口或没有录像，会说明已尝试回退，不把新源故障伪装成空课程。
- 录像列表后台查询可见讲次的分轨大小，并按当前所选视角合计；大小查询复用下载时的清晰度和地址选择，优先 HEAD，必要时请求 `bytes=0-0`，不读取完整视频。每浏览器最多并发 2 个讲次、服务端最多并发 4 个大小任务，成功结果缓存 15 分钟；失败可单独重试。下载准备会复用已知大小，查询失败不阻止下载。
- 支持教师、PPT、学生与合成分轨的普通媒体文件下载；仅提供 HLS/专用播放流时会明确提示，不会把播放列表保存成视频。
- 浏览器直连优先：Chromium 可用 File System Access API 直接流式写入用户选择的目录。
- 每次单项、批量或重新下载前选择保存位置；默认电脑屏幕＋教室摄像头双视角，按课程、讲次组织目录，同名文件保留副本。
- 直连受 CORS、Referer 或令牌限制时，可自动切换为 VPS 同源流式代理。
- 大批量下载按真实空闲位即时签票；代理会在临时媒体地址过期后用同一会话透明重解析。
- 代理支持单段 `Range`、`If-Range`、`206`、`Content-Range` 与 `ETag`，全程背压传输，不把视频落盘到 VPS。
- Docker 多阶段构建、Caddy 自动 HTTPS、健康检查、非 root/只读容器。

## 2026 年 8 月视频接口迁移

新流程为 `jy-lti-adapter` → `jwt-token` → `jy-application-resourcemanage/lms/launch-context`，取得教学班 ID 后请求 `/v1/subject_vod_list_new`，分轨详情使用 `/v1/course_vod_urls_new`。这些地址来自学校实际授权表单与公开播放器代码；JWT 仅用于服务端请求，不下发给前端。

`VIDEO_API` 仍指原 `v.sjtu.edu.cn` 接口；新服务分别由 `VIDEO_LTI_ADAPTER`、`RESOURCE_VIDEO_API` 配置，通常无需修改。没有教学班映射、接口失败、成功返回空列表分别处理，不再统一误报为停机。空列表缓存仅 30 秒。

Canvas 的“课堂视频旧版”是另一个系统：`courses.sjtu.edu.cn/lti/launch` → `/lti/vodVideo/findVodVideoList` → `/lti/vodVideo/getVodVideoInfos`。程序只使用当前课程导航中实际存在的旧版工具，不猜测工具 ID；新版有记录时使用新版，为空或没有排课时才尝试旧版，不自动合并两套列表。旧版课程参数保留官网要求的 URL 编码，录像 ID 使用 URL-safe 包装，下载前再次检查课程归属和开放状态。旧版域名跟随 `COURSES_ORIGIN`。

已实测新登录、教学班匹配与空列表响应，以及旧版常微分方程课程的 74 条录像；教师、课件分轨的直连和代理都通过了 1 KB Range 分段验证（HTTP 206、MP4 文件头、两种路径字节一致），未测试完整视频下载。新平台非空课次的媒体下载仍需真实可播放课程验证。普通媒体地址继续浏览器直连优先、可选代理；HLS 合并与官方整课打包下载尚未实现。

## 下载路径

每次点击下载，先选择一个本地文件夹。取消选择不会签票或发起媒体请求，也不会复用上次目录。不同批次各自绑定目录，暂停后继续使用原文件；失败后重新下载会再次询问位置。

```text
所选文件夹/
  常微分方程 [87084]/
    课堂录像/
      2026-06-18_18-55 第74讲 [讲次标识]/
        电脑屏幕.mp4
        教室摄像头.mp4
    课程文件/
      讲义.pdf
      讲义 (2).pdf
```

文件名会清除路径分隔符、Windows 保留名称等非法字符，过长名称会截短，讲次标识用于区分同名录像。不会覆盖已有文件；再次下载会添加序号。某个视角未开放时单独提示失败，其余视角继续，不会以另一视角冒充。

目录选择依赖 [File System Access API](https://developer.mozilla.org/en-US/docs/Web/API/Window/showDirectoryPicker)，需要浏览器支持、安全上下文和用户点击。请使用支持此接口的桌面 Chrome / Edge；VPS 使用 HTTPS，本地 localhost / 127.0.0.1 可用。Firefox、Safari 或受限内置浏览器若不支持，会明确阻止创建任务，不再静默交给默认下载目录。页面只能获得文件夹名称，不能得知磁盘绝对路径。

```text
默认（auto）
  浏览器 ──签名 URL──> 学校/CDN             字节不经过 VPS
       └─直连不可用──> Rust 流式代理 ──> 学校/CDN

可选（proxy）
  浏览器 ───────────> Rust 流式代理 ──> 学校/CDN
```

跨域 `fetch` 是否可用由学校/CDN 的 CORS 策略决定，前端 JavaScript 也不能伪造 `courses.sjtu.edu.cn` Referer。因此“浏览器直连”是优先策略，不是对所有资源的保证；自动回退不是绕过权限，而是继续使用当前用户已经授权的服务端会话。

## VPS 部署

要求：一台有公网域名的 Linux VPS、Docker Engine 与 Docker Compose v2。

```bash
git clone https://github.com/your-name/canvas-pocket-web.git
cd canvas-pocket-web
cp .env.example .env
```

编辑 `.env`：

```dotenv
DOMAIN=canvas.example.com
PUBLIC_URL=https://canvas.example.com
APP_SECRET=用-openssl-rand-base64-48-生成的随机值
```

确保域名 A/AAAA 记录指向 VPS，然后启动：

```bash
docker compose up -d --build
```

Caddy 会自动申请 HTTPS 证书。运行数据位于 Docker volume `canvas-data`；`APP_SECRET` 必须长期保持不变，否则已加密的 Canvas 会话无法恢复。

### 已有反向代理

只启动应用服务并将现有 Nginx/Caddy 代理到它：

```bash
docker compose up -d --build canvas-pocket
```

代理下载接口时应关闭响应缓冲，并保留 `Range`、`If-Range` 与客户端断开信号。生产环境必须让 `PUBLIC_URL` 与浏览器实际访问的 HTTPS origin 完全一致。

## 本地开发

要求 Rust 1.96+ 与 Node.js 24+。

```bash
# 终端 1：后端（演示数据，不访问学校服务）
DEMO_MODE=true COOKIE_SECURE=false cargo run -p canvas-pocket-server

# 终端 2：前端
cd frontend
npm install
npm run dev
```

前端开发服务器会把 `/api` 代理到 `http://127.0.0.1:8080`。若只想查看界面，也可使用 `VITE_DEMO_MODE=true npm run dev`。

## 配置

| 环境变量 | 默认值 | 说明 |
|---|---:|---|
| `BIND` | `0.0.0.0:8080` | HTTP 监听地址 |
| `PUBLIC_URL` | 空 | 生产站点 origin；设置后用于写操作的 Origin 校验 |
| `APP_SECRET` | 自动生成 | 会话加密密钥；生产必须显式设置 |
| `DATA_DIR` | `./data` | 密钥和加密会话目录 |
| `WEB_DIST` | `./frontend/dist` | SPA 构建目录 |
| `COOKIE_SECURE` | 随 `PUBLIC_URL` | HTTPS 时使用 `__Host-` Secure Cookie |
| `SESSION_TTL_DAYS` | `7` | 上游会话本地保留时长；学校实际会话可能更早失效 |
| `DOWNLOAD_TICKET_TTL_MINUTES` | `15` | 临时上游地址的刷新间隔；opaque ticket 最长随本站会话保留 |
| `PROXY_CONCURRENCY` | `16` | 整个实例的并发代理流上限 |
| `DEMO_MODE` | `false` | 使用服务端演示数据 |

学校端地址也可通过 `CANVAS_ORIGIN`、`COURSES_ORIGIN`、`VIDEO_API`、`JACCOUNT_ORIGIN` 覆盖，主要用于兼容性调试。

## 安全模型

- 浏览器只持有本站随机、HttpOnly、SameSite 会话 Cookie；Canvas/jAccount Cookie 永不下发。
- Canvas Cookie 使用 ChaCha20-Poly1305 加密后才写入 `data/sessions.json`；视频 token、二维码、签名媒体 URL与下载 ticket 仅在内存中短时存在。
- 服务端严格执行会话 TTL，并限制匿名会话与扫码速率，避免公开 VPS 被无界占用。
- 下载接口只接受课程/文件/讲次/分轨 ID，不接受用户提供的 URL；每次代理重定向都会重新验证 HTTPS，并拒绝 localhost 与私网字面 IP。
- 视频讲次在后端再次校验“属于当前课程且审核开放”，不信任前端传来的标题或文件名。
- 全局设置 CSP、`no-referrer`、`nosniff`、frame deny；写操作拒绝跨站请求并可绑定 `PUBLIC_URL`。
- 退出登录会取消扫码状态，并立即撤销该浏览器的所有视频缓存和下载凭证。

不要公开分享实例、会话数据卷、日志或下载地址。仅下载本人有权访问的教学材料，并遵守学校、课程教师与 Canvas 平台的规定。

## 测试与构建

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets

cd frontend
npm ci
npm test
npm run build

docker build -t canvas-pocket-web .
```

## 上游与许可证

本项目采用 MIT License。详细的第三方说明见 [NOTICE.md](NOTICE.md)。`canvas-downloader` 为 MIT；`canvas-sjtu-skill` 在调研时未提供许可证，因此这里只依据公开文档与可观察协议行为重新实现，没有复制其源码。

本项目与上海交通大学、Instructure、jAccount 均无隶属或官方合作关系。
