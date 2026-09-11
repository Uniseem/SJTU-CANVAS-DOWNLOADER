//! Newline-delimited JSON-RPC over stdin/stdout.
//!
//! Host → engine: `{"id": 1, "method": "courses.list", "params": {...}}`
//! Engine → host: `{"id": 1, "result": ...}` or
//!                `{"id": 1, "error": {"code": "...", "message": "...", "retry_after_seconds": 30}}`
//! Engine → host notifications (no id): `login.status`, `account.changed`,
//! `download.changed`, `download.progress`, `download.removed`.
//!
//! stdout carries protocol lines only; logs go to a file. The engine exits
//! when stdin closes, so it can never outlive its host.

use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use anyhow::Result;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
};

use crate::{
    canvas::CanvasClient,
    config,
    downloads::{self, ListInput, NewDownload},
    error::AppError,
    fake, guard,
    settings::{self, MAX_CONCURRENCY, Preferences, TRACKS},
    state::AppState,
};

pub const PROTOCOL_VERSION: u32 = 1;
const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct Request {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_seconds: Option<u64>,
}

impl RpcError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_params",
            message: message.into(),
            retry_after_seconds: None,
        }
    }

    fn user(message: impl Into<String>) -> Self {
        Self {
            code: "user_error",
            message: message.into(),
            retry_after_seconds: None,
        }
    }
}

impl From<AppError> for RpcError {
    fn from(error: AppError) -> Self {
        if let AppError::Internal(source) = &error {
            tracing::error!(error = %format!("{source:#}"), "request failed");
        }
        Self {
            code: error.code(),
            message: error.to_string().chars().take(1_500).collect(),
            retry_after_seconds: error.retry_after(),
        }
    }
}

impl From<anyhow::Error> for RpcError {
    fn from(error: anyhow::Error) -> Self {
        Self::from(AppError::Internal(error))
    }
}

type RpcResult = std::result::Result<Value, RpcError>;

struct Server {
    state: Arc<AppState>,
    shutdown: tokio::sync::Notify,
}

/// Serves requests until stdin closes or the host asks the engine to stop.
pub async fn serve(
    state: Arc<AppState>,
    outgoing: mpsc::UnboundedSender<String>,
    mut lines: mpsc::UnboundedReceiver<String>,
) -> Result<()> {
    // The notification sink keeps a sender alive for the whole process, so
    // the writer is told explicitly when to drain the queue and stop.
    let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        loop {
            let line = tokio::select! {
                biased;
                line = lines.recv() => line,
                _ = &mut stopped => {
                    while let Ok(line) = lines.try_recv() {
                        let _ = stdout.write_all(format!("{line}\n").as_bytes()).await;
                    }
                    let _ = stdout.flush().await;
                    break;
                }
            };
            let Some(line) = line else {
                break;
            };
            if stdout.write_all(line.as_bytes()).await.is_err()
                || stdout.write_all(b"\n").await.is_err()
                || stdout.flush().await.is_err()
            {
                break;
            }
        }
    });
    let server = Arc::new(Server {
        state: state.clone(),
        shutdown: tokio::sync::Notify::new(),
    });
    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = tokio::select! {
            read = stdin.read_until(b'\n', &mut buffer) => read?,
            _ = server.shutdown.notified() => break,
        };
        if read == 0 {
            tracing::info!("host closed stdin; stopping");
            break;
        }
        if buffer.len() > MAX_REQUEST_BYTES {
            tracing::warn!(bytes = buffer.len(), "oversized request ignored");
            continue;
        }
        let line = String::from_utf8_lossy(&buffer).trim().to_string();
        if line.is_empty() {
            continue;
        }
        let server = server.clone();
        let outgoing = outgoing.clone();
        tokio::spawn(async move {
            if let Some(response) = server.handle(&line).await {
                let _ = outgoing.send(response);
            }
        });
    }
    // Flush partial files so paused and interrupted downloads resume cleanly.
    state.downloads.stop_all().await;
    drop(outgoing);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = stop.send(());
    let _ = tokio::time::timeout(Duration::from_secs(1), writer).await;
    Ok(())
}

impl Server {
    async fn handle(self: &Arc<Self>, line: &str) -> Option<String> {
        let request = match serde_json::from_str::<Request>(line) {
            Ok(request) => request,
            Err(error) => {
                return Some(
                    json!({"id": null, "error": {"code": "parse_error", "message": error.to_string()}})
                        .to_string(),
                );
            }
        };
        let id = request.id.clone();
        let result = self.dispatch(&request.method, request.params).await;
        let id = id?;
        Some(
            match result {
                Ok(value) => json!({"id": id, "result": value}),
                Err(error) => json!({"id": id, "error": error}),
            }
            .to_string(),
        )
    }

    async fn dispatch(self: &Arc<Self>, method: &str, params: Value) -> RpcResult {
        let state = &self.state;
        match method {
            "engine.initialize" => {
                let input: InitializeInput = parse_or_default(params)?;
                if let Some(folder) = input
                    .downloads_folder
                    .filter(|value| !value.trim().is_empty())
                {
                    state.set_default_download_dir(config::fallback_download_dir(Some(
                        &PathBuf::from(folder),
                    )));
                }
                state
                    .account
                    .unlock(input.session_key.as_deref())
                    .await
                    .map_err(|error| RpcError::invalid(format!("{error:#}")))?;
                let preferences =
                    settings::load(&state.pool, &state.default_download_dir()).await?;
                state.set_preferences(preferences);
                if !state.scheduler_started.swap(true, Ordering::SeqCst) {
                    tokio::spawn(downloads::run_scheduler(state.clone()));
                }
                Ok(json!({
                    "protocol": PROTOCOL_VERSION,
                    "version": env!("CARGO_PKG_VERSION"),
                    "data_dir": state.config.data_root,
                    "settings": settings_view(state),
                    "account": state.account.view().await,
                }))
            }
            "engine.shutdown" => {
                self.shutdown.notify_one();
                Ok(json!({}))
            }
            "settings.get" => view(settings_view(state)),
            "settings.update" => {
                let input: SettingsUpdate = parse(params)?;
                let saved = settings::save(
                    &state.pool,
                    &input.preferences,
                    &state.default_download_dir(),
                )
                .await
                .map_err(|error| RpcError::user(format!("{error:#}")))?;
                state.set_preferences(saved);
                view(settings_view(state))
            }
            "account.get" => {
                let input: AccountInput = parse_or_default(params)?;
                account(state, input.verify).await
            }
            "account.logout" => {
                state.auth.cancel().await;
                let profile = state.account.profile().await;
                state.account.logout().await?;
                state
                    .after_logout(profile.as_ref().map(|profile| profile.id.as_str()))
                    .await;
                view(state.account.view().await)
            }
            "login.start" => view(state.auth.start().await?),
            "login.refresh" => {
                state.auth.refresh().await?;
                Ok(json!({}))
            }
            "login.cancel" => {
                state.auth.cancel().await;
                Ok(json!({}))
            }
            "login.status" => view(state.auth.status().await),
            "courses.list" => {
                if state.config.fake_school {
                    signed_in(state).await?;
                    return Ok(json!({ "courses": fake::courses() }));
                }
                let config = state.config.clone();
                let courses = guard::with_login(state, |jar, _| async move {
                    CanvasClient::new(config, jar)?.courses().await
                })
                .await?;
                Ok(json!({ "courses": courses }))
            }
            "courses.files" => {
                let input: CourseInput = parse(params)?;
                if state.config.fake_school {
                    signed_in(state).await?;
                    return Ok(json!({ "files": fake::files(&input.course_id)? }));
                }
                let config = state.config.clone();
                let files = guard::with_login(state, |jar, _| async move {
                    CanvasClient::new(config, jar)?
                        .files(&input.course_id)
                        .await
                })
                .await?;
                Ok(json!({ "files": files }))
            }
            "courses.lessons" => {
                let input: CourseInput = parse(params)?;
                if state.config.fake_school {
                    signed_in(state).await?;
                    return Ok(json!({ "lessons": fake::lessons(&input.course_id)? }));
                }
                let videos = state.videos.clone();
                let lessons = guard::with_login(state, |jar, profile| async move {
                    videos.lessons(&profile.id, jar, &input.course_id).await
                })
                .await?;
                Ok(json!({ "lessons": lessons }))
            }
            "lessons.sizes" => {
                let input: SizesInput = parse(params)?;
                if input.tracks.is_empty() {
                    return Err(RpcError::invalid("请至少选择一个画面"));
                }
                if state.config.fake_school {
                    signed_in(state).await?;
                    return view(fake::sizes(
                        &input.course_id,
                        &input.lesson_id,
                        &input.tracks,
                    )?);
                }
                let videos = state.videos.clone();
                let tracks = input.tracks.join(",");
                let sizes = guard::with_login(state, |jar, profile| async move {
                    videos
                        .lesson_sizes(
                            &profile.id,
                            jar,
                            &input.course_id,
                            &input.lesson_id,
                            &tracks,
                            input.refresh,
                        )
                        .await
                })
                .await?;
                view(sizes)
            }
            "downloads.create" => {
                let input: CreateInput = parse(params)?;
                let profile = signed_in(state).await?;
                let destination = PathBuf::from(
                    input
                        .destination
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| state.preferences().download_dir),
                );
                if !destination.is_absolute() {
                    return Err(RpcError::user("保存位置必须是完整的文件夹路径"));
                }
                tokio::fs::create_dir_all(&destination)
                    .await
                    .map_err(|error| {
                        RpcError::user(format!(
                            "无法使用保存位置 {}：{error}",
                            destination.display()
                        ))
                    })?;
                view(
                    state
                        .downloads
                        .create(&profile.id, &destination, input.items)
                        .await?,
                )
            }
            "downloads.list" => {
                let input: ListInput = parse_or_default(params)?;
                view(state.downloads.list(input).await?)
            }
            "downloads.get" => view(state.downloads.get(&parse::<IdInput>(params)?.id).await?),
            "downloads.pause" => view(state.downloads.pause(&parse::<IdInput>(params)?.id).await?),
            "downloads.resume" => view(
                state
                    .downloads
                    .resume(&parse::<IdInput>(params)?.id)
                    .await?,
            ),
            "downloads.cancel" => view(
                state
                    .downloads
                    .cancel(&parse::<IdInput>(params)?.id)
                    .await?,
            ),
            "downloads.retry" => view(state.downloads.retry(&parse::<IdInput>(params)?.id).await?),
            "downloads.remove" => {
                state
                    .downloads
                    .remove(&parse::<IdInput>(params)?.id)
                    .await?;
                Ok(json!({ "removed": true }))
            }
            "downloads.pauseAll" => Ok(json!({ "count": state.downloads.pause_all().await? })),
            "downloads.resumeAll" => Ok(json!({ "count": state.downloads.resume_all().await? })),
            "downloads.clearCompleted" => {
                Ok(json!({ "count": state.downloads.clear_completed().await? }))
            }
            other => Err(RpcError {
                code: "method_not_found",
                message: format!("未知方法：{other}"),
                retry_after_seconds: None,
            }),
        }
    }
}

async fn signed_in(
    state: &AppState,
) -> std::result::Result<crate::models::CanvasProfile, RpcError> {
    state
        .account
        .profile()
        .await
        .ok_or_else(|| RpcError::from(AppError::Unauthorized))
}

/// The account, optionally confirmed with Canvas. A failed check for lack of
/// network keeps the login (`verified` is then false).
async fn account(state: &Arc<AppState>, verify: bool) -> RpcResult {
    let mut view_value =
        serde_json::to_value(state.account.view().await).map_err(anyhow::Error::from)?;
    let mut verified = false;
    if verify
        && !state.config.fake_school
        && let Some((jar, profile)) = state.account.current().await
    {
        let config = state.config.clone();
        let check = tokio::time::timeout(Duration::from_secs(8), async {
            CanvasClient::new(config, jar.clone())?.profile().await
        })
        .await;
        match check {
            Ok(Ok(fresh)) if fresh.id == profile.id => {
                verified = true;
                view_value["profile"] = serde_json::to_value(fresh).map_err(anyhow::Error::from)?;
            }
            Ok(Ok(_)) | Ok(Err(AppError::Unauthorized)) => {
                verified = true;
                if state.account.logout_if_current(&jar).await? {
                    state.after_logout(Some(&profile.id)).await;
                }
                view_value = serde_json::to_value(state.account.view().await)
                    .map_err(anyhow::Error::from)?;
            }
            _ => {}
        }
    } else if state.config.fake_school {
        verified = true;
    }
    view_value["verified"] = json!(verified);
    Ok(view_value)
}

#[derive(Serialize)]
struct SettingsView {
    preferences: Preferences,
    default_download_dir: String,
    concurrency_max: usize,
    tracks: [&'static str; 3],
    data_dir: PathBuf,
    engine_version: &'static str,
    test_mode: bool,
    fake_school: bool,
}

fn settings_view(state: &AppState) -> SettingsView {
    SettingsView {
        preferences: state.preferences(),
        default_download_dir: state.default_download_dir().to_string_lossy().into_owned(),
        concurrency_max: MAX_CONCURRENCY,
        tracks: TRACKS,
        data_dir: state.config.data_root.clone(),
        engine_version: env!("CARGO_PKG_VERSION"),
        test_mode: state.config.test_mode,
        fake_school: state.config.fake_school,
    }
}

fn parse<T: DeserializeOwned>(params: Value) -> std::result::Result<T, RpcError> {
    serde_json::from_value(params)
        .map_err(|error| RpcError::invalid(format!("参数格式不正确：{error}")))
}

fn parse_or_default<T: DeserializeOwned + Default>(
    params: Value,
) -> std::result::Result<T, RpcError> {
    if params.is_null() || params.as_object().is_some_and(|object| object.is_empty()) {
        return Ok(T::default());
    }
    parse(params)
}

fn view(value: impl Serialize) -> RpcResult {
    Ok(serde_json::to_value(value).map_err(anyhow::Error::from)?)
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct InitializeInput {
    /// Base64 key from the OS credential store that protects the saved login.
    #[serde(default)]
    session_key: Option<String>,
    /// The platform's Downloads folder; downloads default to a folder in it.
    #[serde(default)]
    downloads_folder: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsUpdate {
    preferences: Preferences,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountInput {
    #[serde(default)]
    verify: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CourseInput {
    course_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SizesInput {
    course_id: String,
    lesson_id: String,
    tracks: Vec<String>,
    #[serde(default)]
    refresh: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    items: Vec<NewDownload>,
    /// A folder chosen for this download; the preference is used otherwise.
    #[serde(default)]
    destination: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdInput {
    id: String,
}
