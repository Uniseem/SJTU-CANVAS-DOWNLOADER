use std::{io, sync::Arc, time::Duration};

use async_stream::stream;
use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{SecondsFormat, Utc};
use dashmap::DashMap;
use futures_util::{StreamExt, stream as futures_stream};
use reqwest::{Client, cookie::Jar, redirect::Policy};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Semaphore};
use url::Url;

use crate::{
    canvas::{CanvasClient, safe_filename, validate_generated_url},
    config::Config,
    error::{AppError, AppResult},
    models::DownloadDescriptor,
    session::{SessionStore, random_token},
    video::VideoService,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareRequest {
    #[serde(default)]
    pub preference: DownloadPreference,
    pub items: Vec<PrepareItem>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PrepareItem {
    File {
        course_id: String,
        file_id: String,
    },
    Video {
        course_id: String,
        lesson_id: String,
        track: String,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadPreference {
    #[default]
    Auto,
    Direct,
    Proxy,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareResponse {
    pub items: Vec<DownloadDescriptor>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<PrepareFailure>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareFailure {
    pub index: usize,
    pub message: String,
}

struct DownloadTicket {
    owner_session: String,
    resource: PrepareItem,
    jar: Arc<Jar>,
    state: Mutex<DownloadTicketState>,
    hard_expires_at: chrono::DateTime<Utc>,
}

struct DownloadTicketState {
    upstream_url: Url,
    filename: String,
    headers: Vec<(String, String)>,
    refresh_at: chrono::DateTime<Utc>,
}

struct ResolvedDownload {
    upstream_url: Url,
    direct_url: Option<String>,
    filename: String,
    size: Option<u64>,
    headers: Vec<(String, String)>,
    source: String,
}

pub struct DownloadService {
    config: Arc<Config>,
    sessions: Arc<SessionStore>,
    videos: Arc<VideoService>,
    tickets: DashMap<String, Arc<DownloadTicket>>,
    proxy_slots: Arc<Semaphore>,
}

impl DownloadService {
    pub fn new(
        config: Arc<Config>,
        sessions: Arc<SessionStore>,
        videos: Arc<VideoService>,
    ) -> Self {
        Self {
            proxy_slots: Arc::new(Semaphore::new(config.proxy_concurrency)),
            config,
            sessions,
            videos,
            tickets: DashMap::new(),
        }
    }

    pub fn clear_owner(&self, owner: &str) {
        self.tickets
            .retain(|_, ticket| ticket.owner_session != owner);
    }

    pub async fn prepare(
        self: &Arc<Self>,
        owner: &str,
        request: PrepareRequest,
    ) -> AppResult<PrepareResponse> {
        if request.items.is_empty() {
            return Err(AppError::BadRequest("请至少选择一个下载项目".into()));
        }
        if request.items.len() > 100 {
            return Err(AppError::BadRequest("单次最多准备 100 个下载项目".into()));
        }
        let (_, jar, _) = self.sessions.authenticated(owner).await?;
        self.purge_expired();
        let service = self.clone();
        let owner = owner.to_string();
        let preference = request.preference;
        let results = futures_stream::iter(request.items.into_iter().enumerate())
            .map(|(index, item)| {
                let service = service.clone();
                let owner = owner.clone();
                let jar = jar.clone();
                async move {
                    (
                        index,
                        service.prepare_one(&owner, jar, preference, item).await,
                    )
                }
            })
            .buffer_unordered(4)
            .collect::<Vec<_>>()
            .await;

        let mut prepared = Vec::new();
        let mut failures = Vec::new();
        for (index, result) in results {
            match result {
                Ok(descriptor) => prepared.push((index, descriptor)),
                Err(error) => failures.push(PrepareFailure {
                    index,
                    message: error.to_string(),
                }),
            }
        }
        prepared.sort_by_key(|(index, _)| *index);
        failures.sort_by_key(|failure| failure.index);
        Ok(PrepareResponse {
            items: prepared.into_iter().map(|(_, item)| item).collect(),
            failures,
        })
    }

    async fn prepare_one(
        &self,
        owner: &str,
        jar: Arc<Jar>,
        preference: DownloadPreference,
        item: PrepareItem,
    ) -> AppResult<DownloadDescriptor> {
        let resolved = self.resolve_item(owner, jar.clone(), &item).await?;
        validate_generated_url(&resolved.upstream_url)?;
        let direct_supported = resolved.direct_url.is_some();
        if matches!(preference, DownloadPreference::Direct) && !direct_supported {
            return Err(AppError::Conflict(
                "该资源需要学校 Cookie 或视频令牌，无法仅使用浏览器直连".into(),
            ));
        }
        let direct_url = if matches!(preference, DownloadPreference::Proxy) {
            None
        } else {
            resolved.direct_url.clone()
        };
        let id = random_token(24);
        let refresh_at = Utc::now()
            + chrono::Duration::from_std(self.config.ticket_ttl)
                .unwrap_or_else(|_| chrono::Duration::minutes(15));
        // A ticket remains a session-bound authorization handle for long queues,
        // while its signed upstream URL is transparently refreshed on demand.
        let hard_ttl = self.config.session_ttl;
        let hard_expires_at = Utc::now()
            + chrono::Duration::from_std(hard_ttl).unwrap_or_else(|_| chrono::Duration::days(7));
        let filename = safe_filename(&resolved.filename);
        self.tickets.insert(
            id.clone(),
            Arc::new(DownloadTicket {
                owner_session: owner.to_string(),
                resource: item,
                jar,
                state: Mutex::new(DownloadTicketState {
                    upstream_url: resolved.upstream_url,
                    filename: filename.clone(),
                    headers: resolved.headers,
                    refresh_at,
                }),
                hard_expires_at,
            }),
        );
        Ok(DownloadDescriptor {
            id: id.clone(),
            filename,
            size: resolved.size,
            direct_url,
            proxy_url: format!("/api/downloads/proxy/{id}"),
            expires_at: refresh_at.to_rfc3339_opts(SecondsFormat::Secs, true),
            source: resolved.source,
            direct_supported,
        })
    }

    async fn resolve_item(
        &self,
        owner: &str,
        jar: Arc<Jar>,
        item: &PrepareItem,
    ) -> AppResult<ResolvedDownload> {
        match item {
            PrepareItem::File { course_id, file_id } => {
                let canvas = CanvasClient::new(self.config.clone(), jar.clone())?;
                let file = canvas.file(course_id, file_id).await?;
                let resolved = canvas.resolve_file_url(&file).await?;
                Ok(ResolvedDownload {
                    upstream_url: resolved.upstream_url,
                    direct_url: resolved.direct_url,
                    filename: resolved.filename,
                    size: resolved.size,
                    headers: Vec::new(),
                    source: "file".to_string(),
                })
            }
            PrepareItem::Video {
                course_id,
                lesson_id,
                track,
            } => {
                let resolved = self
                    .videos
                    .resolve_track(owner, jar, course_id, lesson_id, track)
                    .await?;
                Ok(ResolvedDownload {
                    upstream_url: resolved.upstream_url,
                    direct_url: resolved.direct_url,
                    filename: resolved.filename,
                    size: self
                        .videos
                        .cached_track_size(owner, course_id, lesson_id, track),
                    headers: resolved.headers,
                    source: format!("video:{track}"),
                })
            }
        }
    }

    pub async fn proxy(
        &self,
        owner: &str,
        ticket_id: &str,
        request_headers: &HeaderMap,
    ) -> AppResult<Response> {
        if ticket_id.len() > 64 || !ticket_id.bytes().all(is_token_byte) {
            return Err(AppError::BadRequest("下载凭证格式无效".into()));
        }
        let ticket = self
            .tickets
            .get(ticket_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| AppError::NotFound("下载凭证不存在或已经过期".into()))?;
        if ticket.owner_session != owner {
            return Err(AppError::Forbidden("下载凭证不属于当前浏览器".into()));
        }
        if ticket.hard_expires_at < Utc::now() {
            self.tickets.remove(ticket_id);
            return Err(AppError::NotFound(
                "下载凭证已经过期，请重新准备下载".into(),
            ));
        }
        let range = validated_range(request_headers.get(header::RANGE))?;
        let if_range = request_headers
            .get(header::IF_RANGE)
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.len() <= 256)
            .map(str::to_string);
        let (mut upstream_url, mut filename, mut headers) =
            self.resolve_ticket(owner, &ticket, false).await?;
        let permit = self
            .proxy_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| AppError::internal(anyhow::anyhow!(error)))?;
        let client = streaming_client(ticket.jar.clone())?;
        let mut upstream = open_upstream(
            &client,
            upstream_url,
            &headers,
            range.as_deref(),
            if_range.as_deref(),
        )
        .await?;
        if matches!(
            upstream.status(),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            (upstream_url, filename, headers) = self.resolve_ticket(owner, &ticket, true).await?;
            upstream = open_upstream(
                &client,
                upstream_url,
                &headers,
                range.as_deref(),
                if_range.as_deref(),
            )
            .await?;
        }
        let status = upstream.status();
        if !(status.is_success()
            || status == reqwest::StatusCode::PARTIAL_CONTENT
            || status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE)
        {
            return Err(AppError::Upstream(format!("文件服务返回 HTTP {status}")));
        }
        let upstream_headers = upstream.headers().clone();
        let byte_stream = upstream.bytes_stream();
        let body_stream = stream! {
            let _permit = permit;
            futures_util::pin_mut!(byte_stream);
            while let Some(chunk) = byte_stream.next().await {
                match chunk {
                    Ok(bytes) => yield Ok::<_, io::Error>(bytes),
                    Err(_) => {
                        tracing::warn!("upstream download stream interrupted");
                        yield Err(io::Error::other("upstream download stream interrupted"));
                        break;
                    }
                }
            }
        };
        let mut response = Response::new(Body::from_stream(body_stream));
        *response.status_mut() =
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        copy_response_header(
            &upstream_headers,
            response.headers_mut(),
            header::CONTENT_TYPE,
        );
        copy_response_header(
            &upstream_headers,
            response.headers_mut(),
            header::CONTENT_LENGTH,
        );
        copy_response_header(
            &upstream_headers,
            response.headers_mut(),
            header::CONTENT_RANGE,
        );
        copy_response_header(
            &upstream_headers,
            response.headers_mut(),
            header::ACCEPT_RANGES,
        );
        copy_response_header(&upstream_headers, response.headers_mut(), header::ETAG);
        copy_response_header(
            &upstream_headers,
            response.headers_mut(),
            header::LAST_MODIFIED,
        );
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-store, no-transform"),
        );
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, content_disposition(&filename)?);
        response.headers_mut().insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        Ok(response)
    }

    async fn resolve_ticket(
        &self,
        owner: &str,
        ticket: &DownloadTicket,
        force: bool,
    ) -> AppResult<(Url, String, Vec<(String, String)>)> {
        let mut state = ticket.state.lock().await;
        if force || state.refresh_at <= Utc::now() {
            if force && matches!(&ticket.resource, PrepareItem::Video { .. }) {
                self.videos.clear_owner(owner);
            }
            let resolved = self
                .resolve_item(owner, ticket.jar.clone(), &ticket.resource)
                .await?;
            validate_generated_url(&resolved.upstream_url)?;
            state.upstream_url = resolved.upstream_url;
            state.filename = safe_filename(&resolved.filename);
            state.headers = resolved.headers;
            state.refresh_at = Utc::now()
                + chrono::Duration::from_std(self.config.ticket_ttl)
                    .unwrap_or_else(|_| chrono::Duration::minutes(15));
        }
        Ok((
            state.upstream_url.clone(),
            state.filename.clone(),
            state.headers.clone(),
        ))
    }

    fn purge_expired(&self) {
        let now = Utc::now();
        self.tickets
            .retain(|_, ticket| ticket.hard_expires_at > now);
    }
}

async fn open_upstream(
    client: &Client,
    initial_url: Url,
    headers: &[(String, String)],
    range: Option<&str>,
    if_range: Option<&str>,
) -> AppResult<reqwest::Response> {
    let initial_host = initial_url.host_str().map(str::to_string);
    let mut upstream_url = initial_url;
    for _ in 0..6 {
        validate_generated_url(&upstream_url)?;
        let mut request = client.get(upstream_url.clone());
        for (name, value) in headers {
            let is_sensitive = name.eq_ignore_ascii_case("token");
            if !is_sensitive || upstream_url.host_str() == initial_host.as_deref() {
                request = request.header(name, value);
            }
        }
        if let Some(range) = range {
            request = request.header(header::RANGE, range);
        }
        if let Some(if_range) = if_range {
            request = request.header(header::IF_RANGE, if_range);
        }
        let response = request.send().await?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| AppError::Upstream("下载跳转缺少 Location".into()))?;
            upstream_url = upstream_url.join(location)?;
            continue;
        }
        return Ok(response);
    }
    Err(AppError::Upstream("下载跳转次数过多".into()))
}

fn streaming_client(jar: Arc<Jar>) -> AppResult<Client> {
    Client::builder()
        .cookie_provider(jar)
        .user_agent("CanvasPocketWeb/0.1 (+self-hosted)")
        .connect_timeout(Duration::from_secs(10))
        .redirect(Policy::none())
        .build()
        .map_err(AppError::internal)
}

fn validated_range(value: Option<&HeaderValue>) -> AppResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| AppError::BadRequest("Range 请求头无效".into()))?;
    let Some(specification) = value.strip_prefix("bytes=") else {
        return Err(AppError::BadRequest("仅支持 bytes Range".into()));
    };
    if specification.contains(',') {
        return Err(AppError::BadRequest("不支持多段 Range".into()));
    }
    let Some((start, end)) = specification.split_once('-') else {
        return Err(AppError::BadRequest("Range 格式无效".into()));
    };
    if start.is_empty()
        || !start.bytes().all(|byte| byte.is_ascii_digit())
        || (!end.is_empty() && !end.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(AppError::BadRequest("Range 格式无效".into()));
    }
    Ok(Some(value.to_string()))
}

fn copy_response_header(
    source: &reqwest::header::HeaderMap,
    destination: &mut HeaderMap,
    name: header::HeaderName,
) {
    if let Some(value) = source.get(&name) {
        destination.insert(name, value.clone());
    }
}

fn content_disposition(filename: &str) -> AppResult<HeaderValue> {
    let fallback = filename
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>()
        .replace('"', "_");
    let encoded = urlencoding::encode(filename);
    HeaderValue::from_str(&format!(
        "attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}"
    ))
    .map_err(|error| AppError::internal(anyhow::anyhow!(error)))
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

impl IntoResponse for PrepareResponse {
    fn into_response(self) -> Response {
        let mut response = axum::Json(self).into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_single_open_range() {
        let header = HeaderValue::from_static("bytes=1024-");
        assert_eq!(
            validated_range(Some(&header)).unwrap().as_deref(),
            Some("bytes=1024-")
        );
    }

    #[test]
    fn rejects_multiple_ranges() {
        let header = HeaderValue::from_static("bytes=0-1,4-5");
        assert!(validated_range(Some(&header)).is_err());
    }
}
