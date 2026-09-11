//! QR-code login through 交我办 (jAccount). The engine opens the school login
//! page, keeps the jAccount WebSocket that pushes fresh QR codes, and after
//! the user confirms on the phone completes the jAccount and Canvas logins.
//! The app only ever sees the QR image and status messages; no password or
//! SMS code is read, typed or stored.

use std::{
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{SecondsFormat, Utc};
use futures_util::{SinkExt, StreamExt};
use reqwest::{
    Client,
    cookie::{CookieStore, Jar},
    header,
    redirect::Policy,
};
use scraper::{Html, Selector};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use url::Url;

use crate::{
    account::Account,
    canvas::{CanvasClient, build_client},
    config::Config,
    error::{AppError, AppResult},
    events, fake, http,
};

/// A new QR session is not started more often than this.
const START_COOLDOWN: Duration = Duration::from_secs(3);
/// One login attempt (with its automatic QR refreshes) lasts at most this long.
const ATTEMPT_LIFETIME: Duration = Duration::from_secs(8 * 60);

/// Pushed to the app as the `login.status` notification.
#[derive(Clone, Debug, Serialize)]
pub struct LoginEvent {
    pub attempt_id: String,
    /// preparing | waiting | reconnecting | authorizing | authorized |
    /// expired | cancelled | error
    pub state: String,
    pub message: String,
    pub generation: u32,
    pub revision: u64,
    /// The QR code as a base64 PNG while `state` is `waiting`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qr_png: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

impl LoginEvent {
    pub fn new(attempt_id: &str, state: &str, message: impl Into<String>, generation: u32) -> Self {
        Self {
            attempt_id: attempt_id.to_string(),
            state: state.to_string(),
            message: message.into(),
            generation,
            revision: 0,
            qr_png: None,
            expires_at: None,
        }
    }
}

pub enum LoginCommand {
    Refresh,
    Cancel,
}

pub struct LoginAttempt {
    pub id: String,
    latest: RwLock<LoginEvent>,
    commands: mpsc::Sender<LoginCommand>,
    finished: AtomicBool,
}

impl LoginAttempt {
    pub async fn publish(&self, mut event: LoginEvent) {
        let mut latest = self.latest.write().await;
        event.revision = latest.revision + 1;
        *latest = event.clone();
        events::notify("login.status", &event);
    }

    pub async fn latest(&self) -> LoginEvent {
        self.latest.read().await.clone()
    }
}

type LoginHook = Box<dyn Fn() + Send + Sync>;

pub struct AuthManager {
    pub(crate) config: Arc<Config>,
    pub(crate) account: Arc<Account>,
    current: Mutex<Option<Arc<LoginAttempt>>>,
    last_start: StdMutex<Option<Instant>>,
    on_login: LoginHook,
}

impl AuthManager {
    /// `on_login` runs after a completed login has been saved.
    pub fn new(config: Arc<Config>, account: Arc<Account>, on_login: LoginHook) -> Self {
        Self {
            config,
            account,
            current: Mutex::new(None),
            last_start: StdMutex::new(None),
            on_login,
        }
    }

    /// Starts a login attempt, or returns the status of the running one.
    pub async fn start(self: &Arc<Self>) -> AppResult<LoginEvent> {
        let mut current = self.current.lock().await;
        if let Some(attempt) = current.as_ref()
            && !attempt.finished.load(Ordering::Acquire)
        {
            return Ok(attempt.latest().await);
        }
        {
            let mut last = self
                .last_start
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if last.is_some_and(|started| started.elapsed() < START_COOLDOWN) {
                return Err(AppError::Conflict("扫码请求过于频繁，请几秒后再试".into()));
            }
            *last = Some(Instant::now());
        }
        let id = crate::random_token(12);
        let (commands, command_rx) = mpsc::channel(8);
        let initial = LoginEvent::new(&id, "preparing", "正在建立安全扫码会话…", 0);
        let attempt = Arc::new(LoginAttempt {
            id,
            latest: RwLock::new(initial.clone()),
            commands,
            finished: AtomicBool::new(false),
        });
        *current = Some(attempt.clone());
        drop(current);
        events::notify("login.status", &initial);

        let manager = self.clone();
        tokio::spawn(async move {
            manager.run(attempt, command_rx).await;
        });
        Ok(initial)
    }

    /// A fresh QR code: restarts the running attempt, or starts a new one.
    pub async fn refresh(self: &Arc<Self>) -> AppResult<()> {
        let running = self.running().await;
        match running {
            Some(attempt) if attempt.commands.send(LoginCommand::Refresh).await.is_ok() => Ok(()),
            _ => self.start().await.map(|_| ()),
        }
    }

    pub async fn cancel(&self) {
        if let Some(attempt) = self.running().await {
            let _ = attempt.commands.send(LoginCommand::Cancel).await;
        }
    }

    pub async fn status(&self) -> Option<LoginEvent> {
        match self.current.lock().await.as_ref() {
            Some(attempt) => Some(attempt.latest().await),
            None => None,
        }
    }

    async fn running(&self) -> Option<Arc<LoginAttempt>> {
        self.current
            .lock()
            .await
            .as_ref()
            .filter(|attempt| !attempt.finished.load(Ordering::Acquire))
            .cloned()
    }

    /// Saves a completed login and tells the app.
    pub(crate) async fn finish_login(
        &self,
        jar: Arc<Jar>,
        profile: crate::models::CanvasProfile,
    ) -> Result<(), String> {
        if let Some(previous) = self.account.profile().await
            && previous.id != profile.id
        {
            return Err("重新登录的 Canvas 账号与当前账号不一致，请先退出登录".into());
        }
        self.account
            .complete_login(jar, profile)
            .await
            .map_err(|error| format!("无法保存登录状态：{error:#}"))?;
        events::notify("account.changed", self.account.view().await);
        (self.on_login)();
        Ok(())
    }

    async fn run(
        self: Arc<Self>,
        attempt: Arc<LoginAttempt>,
        mut commands: mpsc::Receiver<LoginCommand>,
    ) {
        if self.config.fake_school {
            fake::run_login(&self, &attempt, &mut commands).await;
            attempt.finished.store(true, Ordering::Release);
            return;
        }
        let jar = Arc::new(Jar::default());
        let started = tokio::time::Instant::now();
        let mut retry = 0u8;
        let mut generation = 0u32;

        loop {
            if started.elapsed() > ATTEMPT_LIFETIME {
                attempt
                    .publish(LoginEvent::new(
                        &attempt.id,
                        "expired",
                        "扫码会话已超时，请重新获取二维码",
                        generation,
                    ))
                    .await;
                break;
            }
            generation += 1;
            attempt
                .publish(LoginEvent::new(
                    &attempt.id,
                    "preparing",
                    if retry == 0 {
                        "正在获取交我办二维码…"
                    } else {
                        "连接已中断，正在自动重连…"
                    },
                    generation,
                ))
                .await;

            let remaining = ATTEMPT_LIFETIME.saturating_sub(started.elapsed());
            let result = tokio::time::timeout(
                remaining,
                self.run_generation(&attempt, jar.clone(), &mut commands, generation),
            )
            .await
            .unwrap_or_else(|_| GenerationResult::Failed("扫码会话超时，请重新获取二维码".into()));
            match result {
                GenerationResult::Authorized => break,
                GenerationResult::Cancelled => {
                    attempt
                        .publish(LoginEvent::new(
                            &attempt.id,
                            "cancelled",
                            "已取消扫码登录",
                            generation,
                        ))
                        .await;
                    break;
                }
                GenerationResult::Restart => {
                    retry = 0;
                    continue;
                }
                GenerationResult::Retry(message) if retry < 3 => {
                    retry += 1;
                    attempt
                        .publish(LoginEvent::new(
                            &attempt.id,
                            "reconnecting",
                            format!("{message}，正在进行第 {retry} 次重连…"),
                            generation,
                        ))
                        .await;
                    tokio::time::sleep(Duration::from_secs(1 << retry)).await;
                }
                GenerationResult::Retry(message) | GenerationResult::Failed(message) => {
                    attempt
                        .publish(LoginEvent::new(&attempt.id, "error", message, generation))
                        .await;
                    break;
                }
            }
        }
        attempt.finished.store(true, Ordering::Release);
    }

    async fn run_generation(
        &self,
        attempt: &Arc<LoginAttempt>,
        jar: Arc<Jar>,
        commands: &mut mpsc::Receiver<LoginCommand>,
        generation: u32,
    ) -> GenerationResult {
        let client = match build_client(jar.clone(), Policy::limited(10)) {
            Ok(client) => client,
            Err(error) => return GenerationResult::Failed(error.to_string()),
        };
        let page = tokio::select! {
            command = commands.recv() => return match command {
                Some(LoginCommand::Refresh) => GenerationResult::Restart,
                _ => GenerationResult::Cancelled,
            },
            result = tokio::time::timeout(Duration::from_secs(20), fetch_login_page(&client, &self.config)) => result,
        };
        let (uuid, referer) = match page {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => return GenerationResult::Retry(error),
            Err(_) => return GenerationResult::Retry("学校登录页响应超时".into()),
        };
        let cookie = match cookie_header_for_websocket(&jar, &referer, &self.config) {
            Ok(cookie) => cookie,
            Err(error) => return GenerationResult::Retry(error),
        };
        let ws_origin = self
            .config
            .jaccount_origin
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        let ws_url = format!("{ws_origin}/jaccount/sub/{uuid}");
        let mut request = match ws_url.into_client_request() {
            Ok(request) => request,
            Err(_) => return GenerationResult::Failed("无法建立扫码连接".into()),
        };
        let origin = match header::HeaderValue::from_str(&self.config.jaccount_origin) {
            Ok(value) => value,
            Err(_) => return GenerationResult::Failed("jAccount 地址配置无效".into()),
        };
        let cookie = match header::HeaderValue::from_str(&cookie) {
            Ok(value) => value,
            Err(_) => return GenerationResult::Failed("扫码会话 Cookie 无效".into()),
        };
        let user_agent = match header::HeaderValue::from_str(&http::user_agent()) {
            Ok(value) => value,
            Err(_) => return GenerationResult::Failed("无法建立扫码连接".into()),
        };
        request.headers_mut().insert(header::ORIGIN, origin);
        request.headers_mut().insert(header::USER_AGENT, user_agent);
        request.headers_mut().insert(header::COOKIE, cookie);

        let connection = tokio::select! {
            command = commands.recv() => return match command {
                Some(LoginCommand::Refresh) => GenerationResult::Restart,
                _ => GenerationResult::Cancelled,
            },
            result = tokio::time::timeout(Duration::from_secs(12), connect_async(request)) => result,
        };
        let (stream, _) = match connection {
            Ok(Ok(stream)) => stream,
            Ok(Err(_)) => return GenerationResult::Retry("交我办扫码服务连接失败".into()),
            Err(_) => return GenerationResult::Retry("交我办扫码连接超时".into()),
        };
        let (mut writer, mut reader) = stream.split();
        if writer
            .send(Message::Text(r#"{"type":"UPDATE_QR_CODE"}"#.into()))
            .await
            .is_err()
        {
            return GenerationResult::Retry("二维码请求发送失败".into());
        }
        let mut refresh = tokio::time::interval(Duration::from_secs(55));
        refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        refresh.tick().await;
        let mut heartbeat = tokio::time::interval(Duration::from_secs(25));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        heartbeat.tick().await;
        let first_qr_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        let mut has_qr = false;

        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(first_qr_deadline), if !has_qr => {
                    return GenerationResult::Retry("扫码服务未及时返回二维码".into());
                },
                command = commands.recv() => match command {
                    Some(LoginCommand::Cancel) | None => return GenerationResult::Cancelled,
                    Some(LoginCommand::Refresh) => {
                        return GenerationResult::Restart;
                    }
                },
                _ = refresh.tick() => {
                    if writer.send(Message::Text(r#"{"type":"UPDATE_QR_CODE"}"#.into())).await.is_err() {
                        return GenerationResult::Restart;
                    }
                },
                _ = heartbeat.tick() => {
                    if writer.send(Message::Ping(Vec::new().into())).await.is_err() {
                        return GenerationResult::Restart;
                    }
                },
                message = reader.next() => {
                    let Some(message) = message else {
                        return GenerationResult::Retry("扫码连接已关闭".into());
                    };
                    let message = match message {
                        Ok(message) => message,
                        Err(_) => return GenerationResult::Retry("扫码连接意外中断".into()),
                    };
                    match message {
                        Message::Ping(payload) => {
                            let _ = writer.send(Message::Pong(payload)).await;
                        }
                        Message::Text(text) => {
                            let Ok(payload) = serde_json::from_str::<Value>(&text) else {
                                continue;
                            };
                            match payload.get("type").and_then(Value::as_str) {
                                Some("UPDATE_QR_CODE") => {
                                    if payload.get("error").and_then(Value::as_i64).is_some_and(|value| value != 0) {
                                        continue;
                                    }
                                    let Some(body) = payload.get("payload") else { continue; };
                                    let Some(timestamp) = stringish(body.get("ts")) else { continue; };
                                    let Some(signature) = stringish(body.get("sig")) else { continue; };
                                    match fetch_qr(&client, &self.config, &uuid, &timestamp, &signature, &referer).await {
                                        Ok(bytes) => {
                                            has_qr = true;
                                            let mut event = LoginEvent::new(
                                                &attempt.id,
                                                "waiting",
                                                "请使用交我办扫码，并在手机上确认登录",
                                                generation,
                                            );
                                            event.qr_png = Some(STANDARD.encode(bytes));
                                            event.expires_at = Some(
                                                (Utc::now() + chrono::Duration::seconds(58))
                                                    .to_rfc3339_opts(SecondsFormat::Secs, true),
                                            );
                                            attempt.publish(event).await;
                                        }
                                        Err(error) => return GenerationResult::Retry(error),
                                    }
                                }
                                Some("LOGIN") => {
                                    attempt.publish(LoginEvent::new(
                                        &attempt.id,
                                        "authorizing",
                                        "扫码成功，正在验证 Canvas 身份…",
                                        generation,
                                    )).await;
                                    if let Err(error) = finish_jaccount(&client, &self.config, &uuid).await {
                                        return GenerationResult::Failed(error);
                                    }
                                    if let Err(error) = finish_canvas(&client, &self.config).await {
                                        return GenerationResult::Failed(error);
                                    }
                                    let canvas = match CanvasClient::new(self.config.clone(), jar.clone()) {
                                        Ok(canvas) => canvas,
                                        Err(error) => return GenerationResult::Failed(error.to_string()),
                                    };
                                    let profile = match canvas.profile().await {
                                        Ok(profile) => profile,
                                        Err(_) => return GenerationResult::Failed("Canvas 身份验证失败，请重新扫码".into()),
                                    };
                                    let name = profile.name.clone();
                                    if let Err(error) = self.finish_login(jar.clone(), profile).await {
                                        return GenerationResult::Failed(error);
                                    }
                                    tracing::info!("QR login completed and Canvas session saved");
                                    attempt.publish(LoginEvent::new(
                                        &attempt.id,
                                        "authorized",
                                        format!("已登录 {name} 的 Canvas"),
                                        generation,
                                    )).await;
                                    return GenerationResult::Authorized;
                                }
                                Some("ERROR_MESSAGE") => {
                                    let message = payload
                                        .get("payload")
                                        .and_then(Value::as_str)
                                        .unwrap_or("二维码已失效");
                                    attempt.publish(LoginEvent::new(
                                        &attempt.id,
                                        "expired",
                                        message,
                                        generation,
                                    )).await;
                                }
                                _ => {}
                            }
                        }
                        Message::Close(_) => return GenerationResult::Retry("扫码连接已关闭".into()),
                        _ => {}
                    }
                }
            }
        }
    }
}

enum GenerationResult {
    Authorized,
    Cancelled,
    Restart,
    Retry(String),
    Failed(String),
}

async fn fetch_login_page(client: &Client, config: &Config) -> Result<(String, String), String> {
    let response = client
        .get(config.login_init_url())
        .header(header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9")
        .send()
        .await
        .map_err(|error| {
            let host = error
                .url()
                .and_then(url::Url::host_str)
                .unwrap_or("unknown")
                .to_string();
            let path = error
                .url()
                .map(url::Url::path)
                .unwrap_or("unknown")
                .to_string();
            tracing::warn!(host, path, error = ?error.without_url(), "failed to open school login page");
            "无法打开学校登录页".to_string()
        })?;
    if !response.status().is_success() {
        return Err(format!("学校登录页返回 HTTP {}", response.status()));
    }
    let referer = response.url().to_string();
    let html = response
        .text()
        .await
        .map_err(|_| "无法读取学校登录页".to_string())?;
    let uuid = extract_uuid(&html)
        .ok_or_else(|| "未找到交我办扫码入口，学校登录页可能已更新".to_string())?;
    Ok((uuid, referer))
}

fn extract_uuid(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let selectors = ["a#firefox_link", "a[href*='uuid=']"];
    for selector in selectors {
        let Ok(selector) = Selector::parse(selector) else {
            continue;
        };
        for node in document.select(&selector) {
            let href = node.value().attr("href")?;
            let query = href.split_once('?').map(|(_, query)| query).unwrap_or(href);
            if let Some((_, uuid)) =
                url::form_urlencoded::parse(query.as_bytes()).find(|(key, _)| key == "uuid")
                && !uuid.is_empty()
            {
                return Some(uuid.into_owned());
            }
        }
    }
    None
}

fn cookie_header_for_websocket(
    jar: &Jar,
    referer: &str,
    config: &Config,
) -> Result<String, String> {
    let mut pairs = Vec::<(String, String)>::new();
    for value in [
        referer,
        &format!("{}/jaccount/", config.jaccount_origin),
        &config.jaccount_origin,
        &config.courses_origin,
    ] {
        let Ok(url) = Url::parse(value) else {
            continue;
        };
        let Some(header) = jar
            .cookies(&url)
            .and_then(|value| value.to_str().ok().map(str::to_string))
        else {
            continue;
        };
        for part in header.split(';').map(str::trim) {
            let Some((name, value)) = part.split_once('=') else {
                continue;
            };
            if let Some(existing) = pairs.iter_mut().find(|(key, _)| key == name) {
                existing.1 = value.to_string();
            } else {
                pairs.push((name.to_string(), value.to_string()));
            }
        }
    }
    if pairs.is_empty() {
        return Err("登录页没有建立 jAccount 会话".into());
    }
    Ok(pairs
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; "))
}

async fn fetch_qr(
    client: &Client,
    config: &Config,
    uuid: &str,
    timestamp: &str,
    signature: &str,
    referer: &str,
) -> Result<Vec<u8>, String> {
    let response = client
        .get(format!("{}/jaccount/qrcode", config.jaccount_origin))
        .query(&[("uuid", uuid), ("ts", timestamp), ("sig", signature)])
        .header(header::REFERER, referer)
        .send()
        .await
        .map_err(|_| "无法获取二维码图片".to_string())?;
    if !response.status().is_success() {
        return Err(format!("二维码服务返回 HTTP {}", response.status()));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "无法读取二维码图片".to_string())?;
    if bytes.is_empty() || bytes.len() > 512 * 1024 {
        return Err("二维码图片大小异常".into());
    }
    Ok(bytes.to_vec())
}

async fn finish_jaccount(client: &Client, config: &Config, uuid: &str) -> Result<(), String> {
    let response = client
        .get(format!("{}/jaccount/expresslogin", config.jaccount_origin))
        .query(&[("uuid", uuid)])
        .header(header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9")
        .send()
        .await
        .map_err(|_| "交我办确认失败".to_string())?;
    if !response.status().is_success() {
        return Err(format!("交我办确认返回 HTTP {}", response.status()));
    }
    if response.url().path().ends_with("/expresslogin") {
        return Err("交我办尚未确认或二维码已过期".into());
    }
    Ok(())
}

async fn finish_canvas(client: &Client, config: &Config) -> Result<(), String> {
    let response = client
        .get(config.canvas_login_url())
        .header(header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9")
        .send()
        .await
        .map_err(|_| "无法连接 Canvas".to_string())?;
    if !response.status().is_success() {
        return Err(format!("Canvas 登录返回 HTTP {}", response.status()));
    }
    Ok(())
}

fn stringish(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_uuid_from_login_link() {
        let html = r#"<a id="firefox_link" href="/jaccount/jalogin?client=x&amp;uuid=test-uuid&amp;returl=x">扫码</a>"#;
        assert_eq!(extract_uuid(html).as_deref(), Some("test-uuid"));
        assert_eq!(extract_uuid("<a href='/jaccount/jalogin'>x</a>"), None);
    }

    #[tokio::test]
    async fn demo_login_completes_once_and_runs_the_login_hook() {
        let mut config = Config::for_tests();
        config.fake_school = true;
        config.fake_login_delay = Duration::from_millis(20);
        let config = Arc::new(config);
        let account = Arc::new(Account::new(&config));
        let hooked = Arc::new(AtomicBool::new(false));
        let flag = hooked.clone();
        let manager = Arc::new(AuthManager::new(
            config,
            account.clone(),
            Box::new(move || flag.store(true, Ordering::SeqCst)),
        ));
        let first = manager.start().await.unwrap();
        assert_eq!(first.state, "preparing");
        // A second start joins the running attempt instead of opening another.
        assert_eq!(manager.start().await.unwrap().attempt_id, first.attempt_id);
        for _ in 0..300 {
            if manager
                .status()
                .await
                .is_some_and(|event| event.state == "authorized")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let last = manager.status().await.unwrap();
        assert_eq!(last.state, "authorized");
        assert!(last.revision >= 3);
        assert!(account.view().await.authenticated);
        assert!(hooked.load(Ordering::SeqCst));
        // A new QR session right after the last one is rate limited.
        for _ in 0..100 {
            if manager.running().await.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(matches!(manager.start().await, Err(AppError::Conflict(_))));
    }
}
