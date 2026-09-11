//! One transfer: resolve a fresh media URL, pick the file name, then stream
//! into `<name>.part` with a Range resume and rename it when complete.

use std::{path::PathBuf, sync::Arc, time::Duration};

use futures_util::StreamExt;
use reqwest::{Client, StatusCode, cookie::Jar, header, redirect::Policy};
use tokio::{
    io::{AsyncWriteExt, BufWriter},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::{DownloadRow, Live, naming};
use crate::{
    canvas::{CanvasClient, validate_generated_url},
    db,
    error::AppError,
    events, fake, guard, http,
    models::CanvasProfile,
    state::AppState,
};

const MAX_ATTEMPTS: u32 = 3;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);
const SAVE_INTERVAL: Duration = Duration::from_secs(5);

/// Why a transfer stopped without completing.
#[derive(Debug)]
pub(super) enum Stop {
    /// Paused, cancelled or shut down; the row already says which.
    Stopped,
    /// The Canvas login is gone: wait in the queue for the next login.
    LoggedOut,
    /// Worth another attempt (network trouble, an expired media URL).
    Transient(String),
    Failed(String),
}

impl From<AppError> for Stop {
    fn from(error: AppError) -> Self {
        if error.is_transient() {
            Stop::Transient(error.to_string())
        } else {
            Stop::Failed(error.to_string())
        }
    }
}

/// A resolved media URL with the request headers it needs.
pub struct Media {
    pub url: Url,
    pub headers: Vec<(String, String)>,
    /// The school's file name; its extension decides ours.
    pub filename: String,
    /// Size known before the transfer (Canvas file size, probed video size).
    pub size: Option<u64>,
}

pub(super) async fn run(state: Arc<AppState>, id: String, cancel: CancellationToken) {
    let mut file: Option<PathBuf> = None;
    let mut refresh = false;
    let mut attempt = 0;
    let outcome = loop {
        attempt += 1;
        match transfer(&state, &id, &cancel, refresh, &mut file).await {
            Ok(()) => break Ok(()),
            Err(Stop::Transient(message)) if attempt < MAX_ATTEMPTS => {
                tracing::warn!(download = %id, attempt, %message, "transfer interrupted; retrying");
                let delay = Duration::from_secs(3 * u64::from(attempt));
                tokio::select! {
                    _ = cancel.cancelled() => break Err(Stop::Stopped),
                    _ = tokio::time::sleep(delay) => {}
                }
                refresh = true;
            }
            Err(stop) => break Err(stop),
        }
    };
    finish(&state, &id, outcome, file).await;
}

async fn finish(state: &AppState, id: &str, outcome: Result<(), Stop>, file: Option<PathBuf>) {
    let (status, error) = match outcome {
        Ok(()) => return,
        Err(Stop::Stopped) => {
            // A pause or shutdown keeps the partial file. A cancel (even one
            // already retried or removed) left the task without it: delete it.
            let Some(path) = file else { return };
            let kept = sqlx::query_scalar::<_, Option<String>>(
                "SELECT file_path FROM downloads WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&state.pool)
            .await;
            let orphaned = match kept {
                Ok(kept) => kept.flatten().as_deref() != path.to_str(),
                Err(_) => false,
            };
            if orphaned {
                let _ = tokio::fs::remove_file(naming::part_path(&path)).await;
            }
            return;
        }
        Err(Stop::LoggedOut) => ("queued", "登录已失效，重新登录后会自动继续".to_string()),
        Err(Stop::Transient(message)) | Err(Stop::Failed(message)) => ("failed", message),
    };
    tracing::warn!(download = %id, status, %error, "transfer stopped");
    let _ = sqlx::query(
        "UPDATE downloads SET status = $2, error = $3, updated_at = $4 WHERE id = $1 AND status = 'downloading'",
    )
    .bind(id)
    .bind(status)
    .bind(&error)
    .bind(db::now())
    .execute(&state.pool)
    .await;
    let _ = state.downloads.changed(id).await;
}

async fn transfer(
    state: &Arc<AppState>,
    id: &str,
    cancel: &CancellationToken,
    refresh: bool,
    file: &mut Option<PathBuf>,
) -> Result<(), Stop> {
    let row = state.downloads.row(id).await.map_err(Stop::from)?;
    if row.status != "downloading" {
        return Err(Stop::Stopped);
    }
    let (jar, profile) = state.account.current().await.ok_or(Stop::LoggedOut)?;
    if profile.id != row.owner_id {
        return Err(Stop::LoggedOut);
    }
    let mut media = tokio::select! {
        _ = cancel.cancelled() => return Err(Stop::Stopped),
        media = resolve(state, &row, &jar, &profile, refresh) => media?,
    };

    let path = match &row.file_path {
        Some(path) => PathBuf::from(path),
        None => {
            let layout = match row.kind.as_str() {
                "video" => naming::video_layout(
                    &row.course_name,
                    &row.course_id,
                    &row.title,
                    row.begin_time.as_deref(),
                    &row.resource_id,
                    row.track.as_deref().unwrap_or("teacher"),
                    &media.filename,
                ),
                _ => naming::file_layout(&row.course_name, &row.course_id, &media.filename),
            };
            let destination = PathBuf::from(&row.destination);
            let path = tokio::task::spawn_blocking(move || naming::allocate(&destination, &layout))
                .await
                .map_err(|error| Stop::Failed(format!("无法创建文件：{error}")))?
                .map_err(|error| Stop::Failed(format!("无法在保存位置创建文件：{error}")))?;
            let recorded = sqlx::query(
                "UPDATE downloads SET file_path = $2, updated_at = $3 WHERE id = $1 AND status <> 'cancelled'",
            )
            .bind(id)
            .bind(path.to_string_lossy().as_ref())
            .bind(db::now())
            .execute(&state.pool)
            .await
            .map_err(|error| Stop::Failed(format!("无法记录保存位置：{error}")))?;
            if recorded.rows_affected() == 0 {
                // Cancelled meanwhile: give the reserved name back.
                let _ = tokio::fs::remove_file(naming::part_path(&path)).await;
                return Err(Stop::Stopped);
            }
            let _ = state.downloads.changed(id).await;
            path
        }
    };
    *file = Some(path.clone());
    let part = naming::part_path(&path);
    if let Some(parent) = part.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| Stop::Failed(format!("无法创建文件夹：{error}")))?;
    }
    let mut resume_at = tokio::fs::metadata(&part)
        .await
        .map(|meta| meta.len())
        .unwrap_or(0);
    let client = media_client(jar.clone())?;

    // An expired or rejected media URL is resolved again once.
    let mut response = open(&client, &media, resume_at, row.etag.as_deref()).await?;
    if rejected(&row, &response) {
        drop(response);
        media = tokio::select! {
            _ = cancel.cancelled() => return Err(Stop::Stopped),
            media = resolve(state, &row, &jar, &profile, true) => media?,
        };
        response = open(&client, &media, resume_at, row.etag.as_deref()).await?;
        if rejected(&row, &response) {
            return Err(Stop::Failed(format!(
                "学校文件服务拒绝了下载请求（HTTP {}）",
                response.status().as_u16()
            )));
        }
    }

    let status = response.status();
    let mut total = row
        .total
        .and_then(|value| u64::try_from(value).ok())
        .or(media.size);
    match status {
        StatusCode::OK => {
            // The server ignored the Range (or the file changed): start over.
            resume_at = 0;
            total = content_length(&response).or(total);
        }
        StatusCode::PARTIAL_CONTENT => match content_range(&response) {
            Some((start, whole)) if start == resume_at => total = whole.or(total),
            _ => {
                let _ = tokio::fs::remove_file(&part).await;
                return Err(Stop::Transient(
                    "服务器返回的续传位置不一致，将从头下载".into(),
                ));
            }
        },
        StatusCode::RANGE_NOT_SATISFIABLE if resume_at > 0 => {
            if total == Some(resume_at) {
                return complete(state, id, &path, resume_at).await;
            }
            let _ = tokio::fs::remove_file(&part).await;
            return Err(Stop::Transient("服务器不接受断点续传，将从头下载".into()));
        }
        status if status.is_server_error() => {
            return Err(Stop::Transient(format!(
                "文件服务暂时不可用（HTTP {}）",
                status.as_u16()
            )));
        }
        StatusCode::NOT_FOUND | StatusCode::GONE => {
            return Err(Stop::Failed("文件已不存在或已被删除（HTTP 404）".into()));
        }
        status => {
            return Err(Stop::Failed(format!(
                "文件服务返回 HTTP {}",
                status.as_u16()
            )));
        }
    }
    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 256)
        .map(str::to_string);
    sqlx::query(
        "UPDATE downloads SET total = $2, etag = $3, received = $4, updated_at = $5 \
         WHERE id = $1 AND status <> 'cancelled'",
    )
    .bind(id)
    .bind(total.map(|value| value as i64))
    .bind(&etag)
    .bind(resume_at as i64)
    .bind(db::now())
    .execute(&state.pool)
    .await
    .map_err(|error| Stop::Failed(format!("无法记录下载进度：{error}")))?;

    let output = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resume_at > 0)
        .truncate(resume_at == 0)
        .open(&part)
        .await
        .map_err(|error| Stop::Failed(format!("无法写入文件：{error}")))?;
    let mut writer = BufWriter::with_capacity(1 << 20, output);
    let mut stream = response.bytes_stream();
    let mut received = resume_at;
    let mut tick = Instant::now();
    let mut tick_bytes = received;
    let mut saved = Instant::now();
    let mut speed = 0.0_f64;
    let live = |received: u64, speed: f64| Live {
        received: received as i64,
        total: total.map(|value| value as i64),
        speed,
    };
    state.downloads.set_live(id, live(received, 0.0));
    loop {
        let chunk = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                let _ = writer.flush().await;
                save_received(state, id, received).await;
                return Err(Stop::Stopped);
            }
            chunk = stream.next() => chunk,
        };
        match chunk {
            None => break,
            Some(Ok(bytes)) => {
                writer
                    .write_all(&bytes)
                    .await
                    .map_err(|error| Stop::Failed(write_error(&error)))?;
                received += bytes.len() as u64;
                state.downloads.set_live(id, live(received, speed));
                let elapsed = tick.elapsed();
                if elapsed >= PROGRESS_INTERVAL {
                    let current = (received - tick_bytes) as f64 / elapsed.as_secs_f64();
                    speed = if speed == 0.0 {
                        current
                    } else {
                        speed * 0.6 + current * 0.4
                    };
                    tick = Instant::now();
                    tick_bytes = received;
                    events::notify(
                        "download.progress",
                        serde_json::json!({
                            "id": id,
                            "received": received,
                            "total": total,
                            "speed": speed,
                        }),
                    );
                }
                if saved.elapsed() >= SAVE_INTERVAL {
                    saved = Instant::now();
                    save_received(state, id, received).await;
                }
            }
            Some(Err(_)) => {
                let _ = writer.flush().await;
                save_received(state, id, received).await;
                return Err(Stop::Transient("下载连接中断".into()));
            }
        }
    }
    writer
        .flush()
        .await
        .map_err(|error| Stop::Failed(write_error(&error)))?;
    let output = writer.into_inner();
    output
        .sync_all()
        .await
        .map_err(|error| Stop::Failed(write_error(&error)))?;
    drop(output);
    if let Some(total) = total
        && received != total
    {
        save_received(state, id, received).await;
        if received > total {
            let _ = tokio::fs::remove_file(&part).await;
        }
        return Err(Stop::Transient(format!(
            "下载连接提前结束（{received} / {total} 字节）"
        )));
    }
    complete(state, id, &path, received).await
}

async fn complete(
    state: &AppState,
    id: &str,
    path: &std::path::Path,
    size: u64,
) -> Result<(), Stop> {
    let target = path.to_path_buf();
    let finished = tokio::task::spawn_blocking(move || naming::finish(&target))
        .await
        .map_err(|error| Stop::Failed(format!("无法保存文件：{error}")))?
        .map_err(|error| Stop::Failed(format!("无法保存文件：{error}")))?;
    sqlx::query(
        "UPDATE downloads SET status = 'completed', received = $2, total = $2, file_path = $3, error = NULL, \
         completed_at = $4, updated_at = $4 WHERE id = $1",
    )
    .bind(id)
    .bind(size as i64)
    .bind(finished.to_string_lossy().as_ref())
    .bind(db::now())
    .execute(&state.pool)
    .await
    .map_err(|error| Stop::Failed(format!("无法记录下载结果：{error}")))?;
    let _ = state.downloads.changed(id).await;
    Ok(())
}

/// Progress of a paused task is kept; a cancelled one has none.
async fn save_received(state: &AppState, id: &str, received: u64) {
    let _ =
        sqlx::query("UPDATE downloads SET received = $2 WHERE id = $1 AND status <> 'cancelled'")
            .bind(id)
            .bind(received as i64)
            .execute(&state.pool)
            .await;
}

fn write_error(error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::StorageFull => "磁盘空间不足，下载已停止".into(),
        std::io::ErrorKind::PermissionDenied => "没有写入保存位置的权限".into(),
        _ => format!("写入文件失败：{error}"),
    }
}

/// A media URL for the task, resolved through the account's current login.
async fn resolve(
    state: &Arc<AppState>,
    row: &DownloadRow,
    jar: &Arc<Jar>,
    profile: &CanvasProfile,
    refresh: bool,
) -> Result<Media, Stop> {
    if state.config.fake_school {
        return fake::resolve_download(&state.config, row).map_err(Stop::from);
    }
    let result = match row.kind.as_str() {
        "video" => {
            if refresh {
                state.videos.clear_owner(&profile.id);
            }
            let track = row.track.as_deref().unwrap_or("teacher");
            state
                .videos
                .resolve_track(
                    &profile.id,
                    jar.clone(),
                    &row.course_id,
                    &row.resource_id,
                    track,
                )
                .await
                .map(|video| Media {
                    url: video.upstream_url,
                    headers: video.headers,
                    filename: video.filename,
                    size: state.videos.cached_track_size(
                        &profile.id,
                        &row.course_id,
                        &row.resource_id,
                        track,
                    ),
                })
        }
        _ => {
            async {
                let canvas = CanvasClient::new(state.config.clone(), jar.clone())?;
                let file = canvas.file(&row.course_id, &row.resource_id).await?;
                let resolved = canvas.resolve_file_url(&file).await?;
                Ok(Media {
                    url: resolved.upstream_url,
                    headers: Vec::new(),
                    filename: resolved.filename,
                    size: resolved.size,
                })
            }
            .await
        }
    };
    match result {
        Ok(media) => Ok(media),
        Err(AppError::Unauthorized) => {
            match guard::confirm_unauthorized(state, jar, &profile.id).await {
                AppError::Unauthorized => Err(Stop::LoggedOut),
                AppError::Conflict(message) => Err(Stop::Transient(message)),
                other => Err(Stop::from(other)),
            }
        }
        Err(error) => Err(Stop::from(error)),
    }
}

/// The school refused the URL (it expired or needs a fresh token), or sent a
/// web page where a video was expected.
fn rejected(row: &DownloadRow, response: &reqwest::Response) -> bool {
    if matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        return true;
    }
    row.kind == "video"
        && response.status().is_success()
        && response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                let value = value.to_ascii_lowercase();
                value.starts_with("text/html") || value.starts_with("application/json")
            })
}

fn media_client(jar: Arc<Jar>) -> Result<Client, Stop> {
    http::apply(Client::builder())
        .cookie_provider(jar)
        .user_agent(http::user_agent())
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .gzip(false)
        .brotli(false)
        .deflate(false)
        .redirect(Policy::none())
        .build()
        .map_err(|error| Stop::Failed(format!("无法创建下载连接：{error}")))
}

/// GET with manual redirects: every hop is validated, and the school's
/// `token` header is only sent to the host it was issued for.
async fn open(
    client: &Client,
    media: &Media,
    resume_at: u64,
    etag: Option<&str>,
) -> Result<reqwest::Response, Stop> {
    let initial_host = media.url.host_str().map(str::to_string);
    let mut url = media.url.clone();
    for _ in 0..8 {
        validate_generated_url(&url).map_err(Stop::from)?;
        let mut request = client
            .get(url.clone())
            .header(header::ACCEPT_ENCODING, "identity");
        for (name, value) in &media.headers {
            let sensitive = name.eq_ignore_ascii_case("token");
            if !sensitive || url.host_str() == initial_host.as_deref() {
                request = request.header(name, value);
            }
        }
        if resume_at > 0 {
            request = request.header(header::RANGE, format!("bytes={resume_at}-"));
            // Weak validators are not allowed in If-Range.
            if let Some(etag) = etag.filter(|etag| !etag.starts_with("W/")) {
                request = request.header(header::IF_RANGE, etag);
            }
        }
        let response = request
            .send()
            .await
            .map_err(|error| Stop::from(AppError::from(error)))?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| Stop::Failed("下载跳转缺少目标地址".into()))?;
            url = url
                .join(location)
                .map_err(|_| Stop::Failed("下载跳转地址无效".into()))?;
            continue;
        }
        return Ok(response);
    }
    Err(Stop::Failed("下载跳转次数过多".into()))
}

fn content_length(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

/// `(start, total)` of `Content-Range: bytes start-end/total`.
fn content_range(response: &reqwest::Response) -> Option<(u64, Option<u64>)> {
    let value = response
        .headers()
        .get(header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    Some((start.trim().parse().ok()?, total.trim().parse().ok()))
}
