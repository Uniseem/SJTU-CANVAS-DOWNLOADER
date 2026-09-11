use std::{collections::HashMap, sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use dashmap::DashMap;
use reqwest::{Client, StatusCode, cookie::Jar, header, redirect::Policy};
use scraper::{ElementRef, Html, Selector};
use serde::Deserialize;
use serde_json::Value;
use tokio::{sync::Mutex, time::Instant};
use url::Url;

use crate::{
    canvas::{build_client, safe_filename, validate_canvas_id, validate_generated_url, value_id},
    config::Config,
    error::{AppError, AppResult},
    models::{Lesson, VideoTrack},
};

mod historical;
mod resource;
mod sizes;

#[derive(Clone, Copy, PartialEq, Eq)]
enum VideoProtocol {
    Legacy,
    Resource,
    Historical,
}

const DEFAULT_TOOL_ID: &str = "8329";
const LTI_UNAVAILABLE_RETRY_AFTER: Duration = Duration::from_secs(30);
const LTI_RETRY_JITTER_MIN_MS: u64 = 250;
const LTI_RETRY_JITTER_SPAN_MS: u16 = 501;
const MODERN_SOURCE_TIMEOUT: Duration = Duration::from_secs(20);
const HISTORICAL_SOURCE_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Default)]
struct LtiCircuit {
    unavailable_until: Option<Instant>,
}

#[derive(Clone)]
struct VideoSession {
    token: String,
    canvas_course_id: String,
    protocol: VideoProtocol,
}

#[derive(Clone)]
struct CachedCourse {
    session: VideoSession,
    lessons: Vec<Lesson>,
    expires_at: Instant,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccessParams {
    #[serde(default)]
    cour_id: String,
    #[serde(default)]
    lti_course_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExchangeData {
    token: String,
    params: AccessParams,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: Option<T>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    status: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoDetail {
    #[serde(default)]
    video_play_response_vo_list: Vec<VideoTrack>,
}

#[derive(Clone)]
struct CachedDetail {
    detail: VideoDetail,
    expires_at: Instant,
}

pub struct ResolvedVideo {
    pub upstream_url: Url,
    pub direct_url: Option<String>,
    pub filename: String,
    pub headers: Vec<(String, String)>,
}

pub struct VideoService {
    config: Arc<Config>,
    cache: DashMap<(String, String), CachedCourse>,
    course_locks: DashMap<(String, String), Arc<Mutex<()>>>,
    detail_cache: DashMap<(String, String, String), CachedDetail>,
    detail_locks: DashMap<(String, String, String), Arc<Mutex<()>>>,
    lti_circuit: Mutex<LtiCircuit>,
    sizes: sizes::SizeCache,
}

impl VideoService {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            cache: DashMap::new(),
            course_locks: DashMap::new(),
            detail_cache: DashMap::new(),
            detail_locks: DashMap::new(),
            lti_circuit: Mutex::new(LtiCircuit::default()),
            sizes: sizes::SizeCache::default(),
        }
    }

    pub fn clear_owner(&self, owner: &str) {
        self.sizes.clear_owner(owner);
        self.cache.retain(|(session, _), _| session != owner);
        self.course_locks.retain(|(session, _), _| session != owner);
        self.detail_cache
            .retain(|(session, _, _), _| session != owner);
        self.detail_locks
            .retain(|(session, _, _), _| session != owner);
    }

    pub async fn lessons(
        &self,
        owner: &str,
        jar: Arc<Jar>,
        course_id: &str,
    ) -> AppResult<Vec<Lesson>> {
        Ok(self.course(owner, jar, course_id, false).await?.lessons)
    }

    pub async fn resolve_track(
        &self,
        owner: &str,
        jar: Arc<Jar>,
        course_id: &str,
        lesson_id: &str,
        track_kind: &str,
    ) -> AppResult<ResolvedVideo> {
        validate_canvas_id(course_id)?;
        validate_resource_id(lesson_id)?;
        let mut cached = self.course(owner, jar.clone(), course_id, false).await?;
        let mut lesson = cached
            .lessons
            .iter()
            .find(|lesson| lesson.video_id == lesson_id && lesson.available)
            .cloned()
            .ok_or_else(|| AppError::NotFound("该讲次不存在、未开放，或不属于当前课程".into()))?;

        let detail = match self
            .cached_video_detail(owner, course_id, jar.clone(), &cached.session, lesson_id)
            .await
        {
            Ok(detail) => detail,
            Err(first_error) => {
                self.cache
                    .remove(&(owner.to_string(), course_id.to_string()));
                self.detail_cache.remove(&(
                    owner.to_string(),
                    course_id.to_string(),
                    lesson_id.to_string(),
                ));
                cached = self.course(owner, jar.clone(), course_id, true).await?;
                lesson = cached
                    .lessons
                    .iter()
                    .find(|lesson| lesson.video_id == lesson_id && lesson.available)
                    .cloned()
                    .ok_or_else(|| {
                        AppError::NotFound("重新授权后该讲次已不可用或不属于当前课程".into())
                    })?;
                self.cached_video_detail(owner, course_id, jar.clone(), &cached.session, lesson_id)
                    .await
                    .map_err(|_| first_error)?
            }
        };
        let requested_code = track_code(track_kind)?;
        let track = detail
            .video_play_response_vo_list
            .iter()
            .find(|track| track.cdvi_view_num == requested_code)
            .ok_or_else(|| {
                let available = detail
                    .video_play_response_vo_list
                    .iter()
                    .map(|track| track_label(track.cdvi_view_num))
                    .collect::<Vec<_>>()
                    .join("、");
                AppError::NotFound(format!(
                    "当前讲次没有所选分轨{}",
                    if available.is_empty() {
                        String::new()
                    } else {
                        format!("，可用分轨：{available}")
                    }
                ))
            })?;
        self.resolve_video_url(jar, &cached.session, &lesson, track)
            .await
    }

    async fn course(
        &self,
        owner: &str,
        jar: Arc<Jar>,
        course_id: &str,
        force: bool,
    ) -> AppResult<CachedCourse> {
        validate_canvas_id(course_id)?;
        let key = (owner.to_string(), course_id.to_string());
        if !force
            && let Some(entry) = self.cache.get(&key)
            && entry.expires_at > Instant::now()
        {
            return Ok(entry.clone());
        }
        let lock = self
            .course_locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;
        // A concurrent request may have refreshed this course while we waited.
        // This applies to forced refreshes too because their stale entry was
        // removed by the caller before reaching this lock.
        if let Some(entry) = self.cache.get(&key)
            && entry.expires_at > Instant::now()
        {
            return Ok(entry.clone());
        }
        let modern = source_with_timeout(MODERN_SOURCE_TIMEOUT, "新视频源", async {
            let session = self.authorize(jar.clone(), course_id).await?;
            let lessons = self.fetch_lessons(jar.clone(), course_id, &session).await?;
            Ok((session, lessons))
        })
        .await;
        let (session, lessons) = if historical::should_try_history(&modern) {
            let history = source_with_timeout(
                HISTORICAL_SOURCE_TIMEOUT,
                "旧视频源",
                self.historical_course(jar, course_id),
            )
            .await;
            let selected = historical::select_fallback(modern, history)?;
            if selected.0.protocol == VideoProtocol::Historical {
                tracing::info!("automatically selected historical course recordings");
            }
            selected
        } else {
            modern?
        };
        let cache_ttl = if lessons.is_empty() { 30 } else { 15 * 60 };
        let cached = CachedCourse {
            session,
            lessons,
            expires_at: Instant::now() + Duration::from_secs(cache_ttl),
        };
        self.cache.insert(key, cached.clone());
        Ok(cached)
    }

    async fn authorize(&self, jar: Arc<Jar>, course_id: &str) -> AppResult<VideoSession> {
        let client = build_client(jar.clone(), Policy::limited(10))?;
        let no_redirect = build_client(jar, Policy::none())?;
        let tool_id = self.external_tool_id(&client, course_id).await;
        let launch_response = client
            .get(format!(
                "{}/courses/{course_id}/external_tools/{tool_id}?display=borderless",
                self.config.canvas_origin
            ))
            .send()
            .await?;
        if launch_response.status() == StatusCode::UNAUTHORIZED {
            return Err(AppError::Unauthorized);
        }
        if launch_response.status() == StatusCode::FORBIDDEN {
            return Err(AppError::Forbidden(
                "当前账号无权访问该课程的课堂视频，课程可能尚未开放".into(),
            ));
        }
        let launch_html = launch_response.text().await?;
        let (initiation_action, initiation_fields) = find_form(
            &launch_html,
            |action| {
                action.contains("oidc/login_initiations")
                    || action.contains("/lti/canvas/oidc/login-initiation/canvas-record")
            },
            "未识别到课堂视频授权入口，请从 Canvas 官网核对该课程的视频入口",
        )?;
        if initiation_action.contains("/lti/canvas/oidc/login-initiation/canvas-record") {
            return self
                .authorize_resource_video(
                    &client,
                    &no_redirect,
                    &initiation_action,
                    &initiation_fields,
                )
                .await;
        }
        let initiation_url = absolute_action(&self.config.video_api, &initiation_action)?;
        validate_video_action(&initiation_url, &self.config.video_api)?;
        let initiation_response = self
            .initiate_lti(&client, &initiation_url, &initiation_fields)
            .await?;
        let token_id =
            match resolve_lti_launch(self, &client, &self.config, initiation_response).await? {
                LtiResolution::TokenId(token_id) => token_id,
                LtiResolution::Launch { url, fields } => {
                    let auth_response = self
                        .classify_video_response(no_redirect.post(url).form(&fields).send().await?)
                        .await?;
                    token_id_from_auth_response(auth_response).await?
                }
            };
        let exchange_response = client
            .get(format!(
                "{}/lti3/getAccessTokenByTokenId",
                self.config.video_api
            ))
            .query(&[("tokenId", token_id)])
            .send()
            .await?;
        let exchange: Envelope<ExchangeData> = self
            .classify_video_response(exchange_response)
            .await?
            .json()
            .await
            .map_err(|error| AppError::Upstream(format!("视频授权响应异常：{error}")))?;
        let error_message = exchange.message.clone().unwrap_or_else(|| {
            exchange
                .status
                .map(|status| format!("状态 {status}"))
                .unwrap_or_else(|| "服务未返回数据".into())
        });
        let data = exchange
            .data
            .ok_or_else(|| AppError::Upstream(format!("视频授权失败：{error_message}")))?;
        let canvas_course_id = [data.params.cour_id, data.params.lti_course_id]
            .into_iter()
            .find(|value| !value.trim().is_empty())
            .ok_or_else(|| AppError::Upstream("视频授权没有返回课程 ID".into()))?;
        Ok(VideoSession {
            token: data.token,
            canvas_course_id,
            protocol: VideoProtocol::Legacy,
        })
    }

    async fn initiate_lti(
        &self,
        client: &Client,
        initiation_url: &Url,
        initiation_fields: &HashMap<String, String>,
    ) -> AppResult<reqwest::Response> {
        // Serialize only the short initiation exchange. If the shared video API
        // is down, the first caller performs the sole retry and opens the
        // circuit; concurrent course loads then fail fast instead of amplifying
        // the outage into one request pair per course.
        let mut circuit = self.lti_circuit.lock().await;
        let now = Instant::now();
        if let Some(unavailable_until) = circuit.unavailable_until {
            if unavailable_until > now {
                return Err(lti_unavailable_error(retry_after_seconds(
                    unavailable_until,
                    now,
                )));
            }
            circuit.unavailable_until = None;
        }

        let mut response = client
            .post(initiation_url.clone())
            .form(initiation_fields)
            .send()
            .await?;
        if response.status() != StatusCode::SERVICE_UNAVAILABLE {
            return Ok(response);
        }

        if !retryable_lti_unavailable(response.status(), response.headers()) {
            let _ = response.bytes().await;
            circuit.unavailable_until = Some(Instant::now() + LTI_UNAVAILABLE_RETRY_AFTER);
            return Err(lti_unavailable_error(LTI_UNAVAILABLE_RETRY_AFTER.as_secs()));
        }

        // The initiation is a read-only OIDC bootstrap. Retry the transient 503
        // once with jitter only when the gateway returned neither a redirect nor
        // a cookie mutation. Never replay later launch/token mutations here.
        let _ = response.bytes().await;
        let jitter_ms =
            LTI_RETRY_JITTER_MIN_MS + u64::from(rand::random::<u16>() % LTI_RETRY_JITTER_SPAN_MS);
        tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
        response = client
            .post(initiation_url.clone())
            .form(initiation_fields)
            .send()
            .await?;
        if response.status() == StatusCode::SERVICE_UNAVAILABLE {
            let _ = response.bytes().await;
            circuit.unavailable_until = Some(Instant::now() + LTI_UNAVAILABLE_RETRY_AFTER);
            return Err(lti_unavailable_error(LTI_UNAVAILABLE_RETRY_AFTER.as_secs()));
        }

        Ok(response)
    }

    async fn classify_video_response(
        &self,
        response: reqwest::Response,
    ) -> AppResult<reqwest::Response> {
        if response.status() != StatusCode::SERVICE_UNAVAILABLE {
            return Ok(response);
        }
        let _ = response.bytes().await;
        let mut circuit = self.lti_circuit.lock().await;
        let unavailable_until = Instant::now() + LTI_UNAVAILABLE_RETRY_AFTER;
        if circuit
            .unavailable_until
            .is_none_or(|current| current < unavailable_until)
        {
            circuit.unavailable_until = Some(unavailable_until);
        }
        Err(lti_unavailable_error(LTI_UNAVAILABLE_RETRY_AFTER.as_secs()))
    }

    async fn external_tool_id(&self, client: &Client, course_id: &str) -> String {
        let Ok(response) = client
            .get(format!("{}/courses/{course_id}", self.config.canvas_origin))
            .send()
            .await
        else {
            return DEFAULT_TOOL_ID.into();
        };
        let Ok(html) = response.text().await else {
            return DEFAULT_TOOL_ID.into();
        };
        let document = Html::parse_document(&html);
        let Ok(selector) = Selector::parse("a[href*='/external_tools/']") else {
            return DEFAULT_TOOL_ID.into();
        };
        document
            .select(&selector)
            .filter_map(|link| {
                let text = link.text().collect::<String>();
                let href = link.value().attr("href")?;
                (text.contains("课堂视频") && !text.contains("旧版"))
                    .then(|| {
                        href.split("/external_tools/")
                            .nth(1)?
                            .split(['?', '/'])
                            .next()
                    })
                    .flatten()
                    .map(str::to_string)
            })
            .next()
            .unwrap_or_else(|| DEFAULT_TOOL_ID.into())
    }

    async fn fetch_lessons(
        &self,
        jar: Arc<Jar>,
        course_id: &str,
        session: &VideoSession,
    ) -> AppResult<Vec<Lesson>> {
        if session.protocol == VideoProtocol::Resource {
            return self.resource_lessons(jar, session).await;
        }
        let client = build_client(jar, Policy::limited(5))?;
        let candidates = course_id_candidates(course_id, &session.canvas_course_id);
        let mut recognized_empty = false;
        let mut last_message = String::new();

        for candidate in candidates {
            for body in [
                serde_json::json!({"canvasCourseId": candidate.clone()}),
                serde_json::json!({"canvasCourseId": candidate.clone(), "pageIndex": 1, "pageSize": 1000}),
                serde_json::json!({"courId": candidate.clone()}),
                serde_json::json!({"courId": candidate.clone(), "pageIndex": 1, "pageSize": 1000}),
                serde_json::json!({"courseId": candidate.clone()}),
                serde_json::json!({"ltiCourseId": candidate.clone()}),
            ] {
                let response = client
                    .post(format!(
                        "{}/directOnDemandPlay/findVodVideoList",
                        self.config.video_api
                    ))
                    .header("token", &session.token)
                    .json(&body)
                    .send()
                    .await?;
                let payload: Value = self
                    .classify_video_response(response)
                    .await?
                    .json()
                    .await
                    .map_err(|error| AppError::Upstream(format!("讲次响应异常：{error}")))?;
                if let Some(records) = extract_records(&payload) {
                    if records.is_empty() {
                        recognized_empty = true;
                        continue;
                    }
                    let mut lessons = records
                        .into_iter()
                        .enumerate()
                        .filter_map(|(index, value)| lesson_from_value(value, index))
                        .filter(|lesson| lesson.audit_status == 3)
                        .collect::<Vec<_>>();
                    lessons.sort_by(|left, right| left.begin_time.cmp(&right.begin_time));
                    return Ok(lessons);
                }
                last_message = payload
                    .get("message")
                    .or_else(|| payload.get("msg"))
                    .and_then(Value::as_str)
                    .unwrap_or("返回结构未知")
                    .to_string();
            }
        }
        if recognized_empty {
            return Ok(Vec::new());
        }
        Err(AppError::Upstream(format!(
            "视频列表接口未返回可识别数据：{last_message}"
        )))
    }

    async fn video_detail(
        &self,
        jar: Arc<Jar>,
        session: &VideoSession,
        lesson_id: &str,
    ) -> AppResult<VideoDetail> {
        if session.protocol == VideoProtocol::Resource {
            return self.resource_video_detail(jar, session, lesson_id).await;
        }
        if session.protocol == VideoProtocol::Historical {
            return self.historical_video_detail(jar, lesson_id).await;
        }
        let client = build_client(jar, Policy::limited(5))?;
        let form = reqwest::multipart::Form::new()
            .text("playTypeHls", "true")
            .text("isAudit", "true")
            .text("id", lesson_id.to_string());
        let response = client
            .post(format!(
                "{}/directOnDemandPlay/getVodVideoInfos",
                self.config.video_api
            ))
            .header("token", &session.token)
            .multipart(form)
            .send()
            .await?;
        let response = self.classify_video_response(response).await?;
        if !response.status().is_success() {
            return Err(AppError::Upstream(format!(
                "视频详情返回 HTTP {}",
                response.status()
            )));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|error| AppError::Upstream(format!("视频详情响应异常：{error}")))?;
        video_detail_from_payload(payload)
    }

    async fn cached_video_detail(
        &self,
        owner: &str,
        course_id: &str,
        jar: Arc<Jar>,
        session: &VideoSession,
        lesson_id: &str,
    ) -> AppResult<VideoDetail> {
        let key = (
            owner.to_string(),
            course_id.to_string(),
            lesson_id.to_string(),
        );
        if let Some(cached) = self.detail_cache.get(&key)
            && cached.expires_at > Instant::now()
        {
            return Ok(cached.detail.clone());
        }
        let lock = self
            .detail_locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;
        if let Some(cached) = self.detail_cache.get(&key)
            && cached.expires_at > Instant::now()
        {
            return Ok(cached.detail.clone());
        }
        let detail = self.video_detail(jar, session, lesson_id).await?;
        self.detail_cache.insert(
            key,
            CachedDetail {
                detail: detail.clone(),
                expires_at: Instant::now() + Duration::from_secs(90),
            },
        );
        Ok(detail)
    }

    async fn resolve_video_url(
        &self,
        jar: Arc<Jar>,
        session: &VideoSession,
        lesson: &Lesson,
        track: &VideoTrack,
    ) -> AppResult<ResolvedVideo> {
        if session.protocol == VideoProtocol::Resource {
            return resource::resolve_resource_track(lesson, track, &self.config);
        }
        if session.protocol == VideoProtocol::Historical {
            let mut resolved = resource::resolve_resource_track(lesson, track, &self.config)?;
            resolved.headers = vec![("referer".into(), self.config.courses_origin.clone())];
            return Ok(resolved);
        }
        let signed = track
            .direct_url()
            .ok_or_else(|| AppError::NotFound("所选分轨没有可用下载地址".into()))?;
        let signed_url = Url::parse(signed)?;
        validate_generated_url(&signed_url)?;
        if signed_url.path().to_ascii_lowercase().ends_with(".m3u8") {
            return Err(AppError::Conflict(
                "该讲次只提供 HLS 流，当前版本不会把播放列表误存为视频文件".into(),
            ));
        }

        let official_url = Url::parse(&format!(
            "{}/directOnDemandPlay/downloadVideo?id={}",
            self.config.video_api,
            urlencoding::encode(&STANDARD.encode(track.id.as_bytes()))
        ))?;
        let no_redirect = build_client(jar.clone(), Policy::none())?;
        let official_response = no_redirect
            .get(official_url.clone())
            .header("token", &session.token)
            .header(header::REFERER, &self.config.courses_origin)
            .header(header::RANGE, "bytes=0-0")
            .send()
            .await;

        let mut upstream_url = signed_url.clone();
        let mut direct_url = Some(signed_url.to_string());
        let mut headers = vec![("referer".to_string(), self.config.courses_origin.clone())];
        if let Ok(response) = official_response {
            // This is an optional redirect probe. A 503 deliberately falls
            // through to the already validated signed URL instead of blocking a
            // download that can still succeed without the API endpoint.
            if response.status().is_redirection() {
                if let Some(location) = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    && let Ok(resolved) = official_url.join(location)
                    && validate_generated_url(&resolved).is_ok()
                {
                    upstream_url = resolved.clone();
                    direct_url = Some(resolved.to_string());
                    headers.clear();
                }
            } else if (response.status().is_success()
                || response.status() == StatusCode::PARTIAL_CONTENT)
                && response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .is_none_or(|value| !value.to_ascii_lowercase().contains("json"))
            {
                upstream_url = official_url;
                direct_url = None;
                headers.push(("token".to_string(), session.token.clone()));
            }
        }

        let date = lesson.begin_time.get(..10).unwrap_or_default();
        let suffix = lesson.video_id.chars().take(8).collect::<String>();
        let extension = signed_url
            .path_segments()
            .and_then(|mut parts| parts.next_back())
            .and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))
            .filter(|extension| extension.len() <= 5)
            .unwrap_or("mp4");
        let filename = safe_filename(&format!(
            "{}_{}_{}_{}.{}",
            lesson.title,
            date,
            track_label(track.cdvi_view_num),
            suffix,
            extension
        ));
        Ok(ResolvedVideo {
            upstream_url,
            direct_url,
            filename,
            headers,
        })
    }
}

async fn source_with_timeout<T>(
    budget: Duration,
    source: &str,
    future: impl std::future::Future<Output = AppResult<T>>,
) -> AppResult<T> {
    tokio::time::timeout(budget, future)
        .await
        .unwrap_or_else(|_| {
            Err(AppError::upstream_unavailable(
                format!("{source}响应超时"),
                30,
            ))
        })
}

fn retry_after_seconds(unavailable_until: Instant, now: Instant) -> u64 {
    let remaining = unavailable_until.saturating_duration_since(now);
    remaining
        .as_secs()
        .saturating_add(u64::from(remaining.subsec_nanos() > 0))
        .max(1)
}

fn retryable_lti_unavailable(status: StatusCode, headers: &header::HeaderMap) -> bool {
    status == StatusCode::SERVICE_UNAVAILABLE
        && !headers.contains_key(header::LOCATION)
        && !headers.contains_key(header::SET_COOKIE)
}

fn lti_unavailable_error(retry_after_seconds: u64) -> AppError {
    AppError::upstream_unavailable(
        "课堂视频 API 当前维护或过载，请稍后重试",
        retry_after_seconds,
    )
}

fn find_form(
    html: &str,
    predicate: impl Fn(&str) -> bool,
    missing_message: &str,
) -> AppResult<(String, HashMap<String, String>)> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("form")
        .map_err(|error| AppError::internal(anyhow::anyhow!(error.to_string())))?;
    let form = document
        .select(&selector)
        .find(|form| form.value().attr("action").is_some_and(&predicate))
        .ok_or_else(|| AppError::Upstream(missing_message.into()))?;
    let action = form.value().attr("action").unwrap_or_default().to_string();
    Ok((action, form_inputs(form)))
}

const MAX_LTI_FORM_HOPS: usize = 4;

enum LtiResolution {
    TokenId(String),
    Launch {
        url: Url,
        fields: HashMap<String, String>,
    },
}

enum LtiFormStep {
    Launch {
        url: Url,
        fields: HashMap<String, String>,
    },
    Submit {
        url: Url,
        method: LtiFormMethod,
        fields: HashMap<String, String>,
    },
}

enum LtiFormMethod {
    Get,
    Post,
}

async fn resolve_lti_launch(
    service: &VideoService,
    client: &Client,
    config: &Config,
    mut response: reqwest::Response,
) -> AppResult<LtiResolution> {
    for _ in 0..MAX_LTI_FORM_HOPS {
        response = service.classify_video_response(response).await?;
        let status = response.status();
        let page_url = response.url().clone();
        if status == StatusCode::UNAUTHORIZED {
            return Err(AppError::Unauthorized);
        }
        if status == StatusCode::FORBIDDEN {
            return Err(AppError::Forbidden(
                "课堂视频授权被拒绝，请在 Canvas 官网确认课程访问权限".into(),
            ));
        }
        if !status.is_success() {
            return Err(AppError::Upstream(format!(
                "课堂视频 LTI 鉴权返回 HTTP {status}"
            )));
        }
        if let Some(token_id) = redirect_parameter(page_url.as_str(), "tokenId") {
            return Ok(LtiResolution::TokenId(token_id));
        }
        let html = response.text().await?;
        if let Some(token_id) = token_id_from_html(&html) {
            return Ok(LtiResolution::TokenId(token_id));
        }
        match lti_form_step(&html, &page_url, config)? {
            Some(LtiFormStep::Launch { url, fields }) => {
                return Ok(LtiResolution::Launch { url, fields });
            }
            Some(LtiFormStep::Submit {
                url,
                method,
                fields,
            }) => {
                let request = match method {
                    LtiFormMethod::Get => client.get(url).query(&fields),
                    LtiFormMethod::Post => client.post(url).form(&fields),
                };
                response = request
                    .header(header::REFERER, page_url.as_str())
                    .send()
                    .await?;
            }
            None => {
                return Err(AppError::Upstream(
                    "课堂视频 LTI 鉴权页面没有返回可识别的授权表单".into(),
                ));
            }
        }
    }
    Err(AppError::Upstream(
        "课堂视频 LTI 鉴权自动跳转次数过多".into(),
    ))
}

fn lti_form_step(html: &str, page_url: &Url, config: &Config) -> AppResult<Option<LtiFormStep>> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("form")
        .map_err(|error| AppError::internal(anyhow::anyhow!(error.to_string())))?;
    let forms = document.select(&selector).collect::<Vec<_>>();

    // Keep the verified SJTU endpoint as the strongest signal. Canvas may render
    // unrelated OIDC-shaped forms before it, so this preference must span the
    // whole document rather than depend on DOM order.
    let mut known_launches = forms.iter().filter(|form| {
        form.value()
            .attr("action")
            .is_some_and(action_path_is_lti3_auth)
    });
    if let Some(form) = known_launches.next() {
        if known_launches.next().is_some() {
            return Err(AppError::Upstream(
                "课堂视频 LTI 鉴权页面包含多个授权表单".into(),
            ));
        }
        let action = form.value().attr("action").unwrap_or_default();
        let url = resolve_video_form_action(page_url, &config.video_api, action)?;
        return Ok(Some(LtiFormStep::Launch {
            url,
            fields: form_inputs(*form),
        }));
    }

    let mut semantic_launches = forms.iter().filter(|form| is_semantic_lti_launch(**form));
    if let Some(form) = semantic_launches.next() {
        if semantic_launches.next().is_some() {
            return Err(AppError::Upstream(
                "课堂视频 LTI 鉴权页面包含多个语义授权表单".into(),
            ));
        }
        let action = form.value().attr("action").unwrap_or_default();
        let url = resolve_video_form_action(page_url, &config.video_api, action)?;
        return Ok(Some(LtiFormStep::Launch {
            url,
            fields: form_inputs(*form),
        }));
    }

    let canvas_origin = Url::parse(&config.canvas_origin)?;
    for form in forms {
        let Some(action) = form.value().attr("action") else {
            continue;
        };
        let Ok(url) = Url::parse(action).or_else(|_| page_url.join(action)) else {
            continue;
        };
        let path = url.path().trim_end_matches('/').to_ascii_lowercase();
        if !same_origin(&url, &canvas_origin) || path != "/api/lti/authorize" {
            continue;
        }
        let method = match form
            .value()
            .attr("method")
            .unwrap_or("get")
            .to_ascii_lowercase()
            .as_str()
        {
            "post" => LtiFormMethod::Post,
            "get" | "" => LtiFormMethod::Get,
            _ => continue,
        };
        return Ok(Some(LtiFormStep::Submit {
            url,
            method,
            fields: form_inputs(form),
        }));
    }
    Ok(None)
}

fn action_path_is_lti3_auth(action: &str) -> bool {
    let path = Url::parse(action)
        .ok()
        .map(|url| url.path().to_string())
        .unwrap_or_else(|| {
            action
                .split(['?', '#'])
                .next()
                .unwrap_or_default()
                .to_string()
        });
    path.split('/')
        .any(|segment| segment.eq_ignore_ascii_case("lti3auth"))
}

fn is_semantic_lti_launch(form: ElementRef<'_>) -> bool {
    let method_is_post = form
        .value()
        .attr("method")
        .is_none_or(|method| method.is_empty() || method.eq_ignore_ascii_case("post"));
    method_is_post
        && has_nonempty_hidden_input(form, "id_token")
        && has_nonempty_hidden_input(form, "state")
}

fn has_nonempty_hidden_input(form: ElementRef<'_>, expected_name: &str) -> bool {
    let Ok(selector) = Selector::parse("input[name]") else {
        return false;
    };
    form.select(&selector).any(|input| {
        input
            .value()
            .attr("name")
            .is_some_and(|name| name.eq_ignore_ascii_case(expected_name))
            && input
                .value()
                .attr("type")
                .is_some_and(|kind| kind.eq_ignore_ascii_case("hidden"))
            && input
                .value()
                .attr("value")
                .is_some_and(|value| !value.trim().is_empty())
    })
}

fn resolve_video_form_action(page_url: &Url, configured_api: &str, action: &str) -> AppResult<Url> {
    let mut candidates = Vec::new();
    if let Ok(url) = Url::parse(action) {
        candidates.push(url);
    }
    if let Ok(url) = page_url.join(action)
        && !candidates.iter().any(|candidate| candidate == &url)
    {
        candidates.push(url);
    }
    let mut configured = Url::parse(configured_api)?;
    if !configured.path().ends_with('/') {
        configured.set_path(&format!("{}/", configured.path()));
    }
    if let Ok(url) = configured.join(action.trim_start_matches('/'))
        && !candidates.iter().any(|candidate| candidate == &url)
    {
        candidates.push(url);
    }
    candidates
        .into_iter()
        .find(|candidate| validate_video_action(candidate, configured_api).is_ok())
        .ok_or_else(|| AppError::Upstream("课堂视频授权表单跳转到了未配置的服务".into()))
}

async fn token_id_from_auth_response(response: reqwest::Response) -> AppResult<String> {
    let status = response.status();
    let response_url = response.url().clone();
    if let Some(token_id) = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|location| {
            response_url
                .join(location)
                .ok()
                .and_then(|url| redirect_parameter(url.as_str(), "tokenId"))
                .or_else(|| token_id_from_candidate(location))
        })
        .or_else(|| redirect_parameter(response_url.as_str(), "tokenId"))
    {
        return Ok(token_id);
    }
    if !(status.is_success() || status.is_redirection()) {
        return Err(AppError::Upstream(format!(
            "视频平台授权返回 HTTP {status}"
        )));
    }
    let html = response.text().await?;
    token_id_from_html(&html)
        .ok_or_else(|| AppError::Upstream("视频平台授权响应缺少 tokenId".into()))
}

fn token_id_from_html(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    if let Ok(selector) = Selector::parse("input[name]") {
        for input in document.select(&selector) {
            if input
                .value()
                .attr("name")
                .is_some_and(|name| name.eq_ignore_ascii_case("tokenId"))
                && let Some(token_id) = input.value().attr("value").and_then(valid_token_id)
            {
                return Some(token_id);
            }
        }
    }
    if let Ok(selector) = Selector::parse("[href], [src], [content], form[action]") {
        for node in document.select(&selector) {
            for attribute in ["href", "src", "content", "action"] {
                if let Some(token_id) = node
                    .value()
                    .attr(attribute)
                    .and_then(token_id_from_candidate)
                {
                    return Some(token_id);
                }
            }
        }
    }
    token_id_from_candidate(html)
}

fn token_id_from_candidate(value: &str) -> Option<String> {
    if let Some(token_id) = redirect_parameter(value, "tokenId") {
        return valid_token_id(&token_id);
    }
    if let Some(start) = value.find("tokenId=") {
        let encoded = value[start + "tokenId=".len()..]
            .chars()
            .take_while(|character| {
                !matches!(
                    character,
                    '&' | '#' | '"' | '\'' | '<' | '>' | '\\' | ';' | ')' | '}'
                ) && !character.is_whitespace()
            })
            .collect::<String>();
        let query = format!("tokenId={encoded}");
        if let Some((_, token_id)) =
            url::form_urlencoded::parse(query.as_bytes()).find(|(key, _)| key == "tokenId")
            && let Some(token_id) = valid_token_id(&token_id)
        {
            return Some(token_id);
        }
    }
    for marker in [r#""tokenId":""#, r#""tokenId": ""#] {
        if let Some(start) = value.find(marker) {
            let token_id = value[start + marker.len()..]
                .split('"')
                .next()
                .unwrap_or_default();
            if let Some(token_id) = valid_token_id(token_id) {
                return Some(token_id);
            }
        }
    }
    None
}

fn valid_token_id(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 4096
        && !value.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(
                    character,
                    '"' | '\'' | '<' | '>' | '{' | '}' | '$' | '\\' | ';'
                )
        }))
    .then(|| value.to_string())
}

fn form_inputs(form: ElementRef<'_>) -> HashMap<String, String> {
    let Ok(selector) = Selector::parse("input[name]") else {
        return HashMap::new();
    };
    form.select(&selector)
        .filter_map(|input| {
            Some((
                input.value().attr("name")?.to_string(),
                input.value().attr("value").unwrap_or_default().to_string(),
            ))
        })
        .collect()
}

fn absolute_action(base: &str, action: &str) -> AppResult<Url> {
    if let Ok(url) = Url::parse(action) {
        return Ok(url);
    }
    let mut base = Url::parse(base)?;
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    base.join(action).map_err(AppError::from)
}

fn validate_video_action(url: &Url, configured_api: &str) -> AppResult<()> {
    validate_generated_url(url)?;
    let configured = Url::parse(configured_api)?;
    let configured_path = configured.path().trim_end_matches('/');
    let path_is_allowed = configured_path.is_empty()
        || configured_path == "/"
        || url.path() == configured_path
        || url
            .path()
            .strip_prefix(configured_path)
            .is_some_and(|suffix| suffix.starts_with('/'));
    if url.scheme() != configured.scheme()
        || url.host_str() != configured.host_str()
        || url.port_or_known_default() != configured.port_or_known_default()
        || !path_is_allowed
    {
        return Err(AppError::Upstream(
            "课堂视频授权表单跳转到了未配置的服务".into(),
        ));
    }
    Ok(())
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn redirect_parameter(value: &str, key: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    url.query_pairs()
        .chain(url.fragment().into_iter().flat_map(|fragment| {
            let query = fragment
                .split_once('?')
                .map(|(_, query)| query)
                .unwrap_or(fragment);
            url::form_urlencoded::parse(query.as_bytes())
        }))
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

fn extract_records(payload: &Value) -> Option<Vec<Value>> {
    if let Some(array) = payload.as_array() {
        return Some(array.clone());
    }
    for path in [
        &["data", "records"][..],
        &["data", "list"][..],
        &["data", "rows"][..],
        &["data", "items"][..],
        &["data", "page", "records"][..],
        &["data", "page", "list"][..],
        &["body", "list"][..],
        &["body"][..],
        &["data"][..],
    ] {
        let mut node = payload;
        let mut found = true;
        for key in path {
            let Some(next) = node.get(*key) else {
                found = false;
                break;
            };
            node = next;
        }
        if found && let Some(records) = node.as_array() {
            return Some(records.clone());
        }
    }
    None
}

fn course_id_candidates(course_id: &str, canvas_course_id: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for value in [canvas_course_id, course_id] {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        push_unique(&mut candidates, value.to_string());
        if value.bytes().all(|byte| byte.is_ascii_digit()) {
            let trimmed = value.trim_start_matches('0');
            if !trimmed.is_empty() {
                push_unique(&mut candidates, trimmed.to_string());
            }
        }
        push_unique(&mut candidates, urlencoding::encode(value).into_owned());
    }
    candidates
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !value.is_empty() && !values.contains(&value) {
        values.push(value);
    }
}

fn lesson_from_value(value: Value, index: usize) -> Option<Lesson> {
    let video_id = value_id(&value, "videoId")?;
    let string = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| value.get(*key).and_then(Value::as_str))
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let audit_status = value
        .get("videAuditStatus")
        .and_then(|entry| entry.as_i64().or_else(|| entry.as_str()?.parse().ok()))
        .unwrap_or_default();
    Some(Lesson {
        video_id,
        title: {
            let title = string(&["courseName", "videoName", "title"]);
            if title.is_empty() {
                format!("第 {:02} 讲", index + 1)
            } else {
                title
            }
        },
        begin_time: string(&["courseBeginTime", "beginTime"]),
        end_time: string(&["courseEndTime", "endTime"]),
        classroom: string(&["classroomName", "classroom", "roomName"]),
        audit_status,
        available: audit_status == 3,
        source: Some("canvas-lti".into()),
    })
}

fn video_detail_from_payload(payload: Value) -> AppResult<VideoDetail> {
    if payload.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(AppError::Upstream(
            payload
                .get("message")
                .or_else(|| payload.get("msg"))
                .and_then(Value::as_str)
                .unwrap_or("视频服务拒绝请求")
                .to_string(),
        ));
    }
    for key in ["data", "body"] {
        if let Some(node) = payload.get(key)
            && let Ok(detail) = serde_json::from_value::<VideoDetail>(node.clone())
            && !detail.video_play_response_vo_list.is_empty()
        {
            return Ok(detail);
        }
    }
    Err(AppError::Upstream("视频详情没有返回可用分轨".into()))
}

fn track_code(value: &str) -> AppResult<i64> {
    match value {
        "teacher" | "0" | "教师" => Ok(0),
        "student1" | "1" | "学生1" => Ok(1),
        "student2" | "2" | "学生2" => Ok(2),
        "slides" | "ppt" | "3" | "PPT" => Ok(3),
        "composite" | "4" | "合成" => Ok(4),
        _ => Err(AppError::BadRequest("未知的视频分轨".into())),
    }
}

pub fn track_label(code: i64) -> &'static str {
    match code {
        0 => "教师",
        1 => "学生1",
        2 => "学生2",
        3 => "PPT",
        4 => "合成",
        _ => "视频",
    }
}

fn validate_resource_id(value: &str) -> AppResult<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(AppError::BadRequest("录像资源 ID 无效".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn lti_test_config() -> Config {
        Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            data_dir: std::path::PathBuf::from("data"),
            web_dist: std::path::PathBuf::from("web"),
            public_url: None,
            app_secret: Some("test-secret".into()),
            cookie_secure: false,
            session_ttl: Duration::from_secs(86_400),
            ticket_ttl: Duration::from_secs(900),
            canvas_origin: "https://oc.sjtu.edu.cn".into(),
            courses_origin: "https://courses.sjtu.edu.cn".into(),
            video_api: "https://v.sjtu.edu.cn/jy-application-canvas-sjtu".into(),
            video_lti_adapter: "https://v.sjtu.edu.cn/jy-lti-adapter".into(),
            resource_video_api: "https://v.sjtu.edu.cn/jy-application-resourcemanage".into(),
            jaccount_origin: "https://jaccount.sjtu.edu.cn".into(),
            demo_mode: false,
            proxy_concurrency: 2,
        }
    }

    #[test]
    fn parses_token_from_fragment_route() {
        let url = "https://v.sjtu.edu.cn/ui/#/ivsModules/index?tokenId=hello%2Bworld";
        assert_eq!(
            redirect_parameter(url, "tokenId").as_deref(),
            Some("hello+world")
        );
    }

    #[test]
    fn only_open_lessons_are_available() {
        let lesson = lesson_from_value(
            serde_json::json!({"videoId": "abc", "videAuditStatus": 3}),
            0,
        )
        .unwrap();
        assert!(lesson.available);
    }

    #[test]
    fn video_course_candidates_include_verified_encoded_form() {
        let candidates = course_id_candidates("87954", "course-v1:SJTU+CS101/2026");
        assert_eq!(
            candidates,
            vec![
                "course-v1:SJTU+CS101/2026",
                "course-v1%3ASJTU%2BCS101%2F2026",
                "87954",
            ]
        );
    }

    #[test]
    fn extracts_page_list_video_shape() {
        let payload = serde_json::json!({"data": {"page": {"list": [{"videoId": "1"}]}}});
        let records = extract_records(&payload).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["videoId"], "1");
    }

    #[test]
    fn recognizes_changed_lti_launch_action_by_oidc_fields() {
        let config = lti_test_config();
        let page_url = Url::parse("https://oc.sjtu.edu.cn/api/lti/authorize").unwrap();
        let html = r#"
            <form action="https://v.sjtu.edu.cn/jy-application-canvas-sjtu/lti3/launch/v2" method="post">
              <input type="hidden" name="id_token" value="jwt" />
              <input type="hidden" name="state" value="state" />
            </form>
        "#;
        let Some(LtiFormStep::Launch { url, fields }) =
            lti_form_step(html, &page_url, &config).unwrap()
        else {
            panic!("expected launch form");
        };
        assert_eq!(
            url.as_str(),
            "https://v.sjtu.edu.cn/jy-application-canvas-sjtu/lti3/launch/v2"
        );
        assert_eq!(fields.get("id_token").map(String::as_str), Some("jwt"));
    }

    #[test]
    fn prioritizes_known_lti3_auth_path_for_error_form() {
        let config = lti_test_config();
        let page_url = Url::parse("https://oc.sjtu.edu.cn/api/lti/authorize").unwrap();
        let html = r#"
            <form action="/lti3/lti3Auth/ivs" method="post">
              <input type="hidden" name="error" value="login_required" />
              <input type="hidden" name="state" value="state" />
            </form>
        "#;
        let Some(LtiFormStep::Launch { url, .. }) =
            lti_form_step(html, &page_url, &config).unwrap()
        else {
            panic!("expected launch form");
        };
        assert_eq!(
            url.as_str(),
            "https://v.sjtu.edu.cn/jy-application-canvas-sjtu/lti3/lti3Auth/ivs"
        );
    }

    #[test]
    fn lti3_auth_path_wins_over_earlier_oidc_shaped_form() {
        let config = lti_test_config();
        let page_url = Url::parse("https://oc.sjtu.edu.cn/api/lti/authorize").unwrap();
        let html = r#"
            <form action="https://v.sjtu.edu.cn/jy-application-canvas-sjtu/lti3/generic-launch" method="post">
              <input type="hidden" name="id_token" value="wrong" />
              <input type="hidden" name="state" value="wrong" />
            </form>
            <form action="/lti3/lti3Auth/ivs?source=canvas" method="post">
              <input type="hidden" name="id_token" value="right" />
              <input type="hidden" name="state" value="right" />
            </form>
        "#;
        let Some(LtiFormStep::Launch { url, fields }) =
            lti_form_step(html, &page_url, &config).unwrap()
        else {
            panic!("expected launch form");
        };
        assert_eq!(
            url.as_str(),
            "https://v.sjtu.edu.cn/jy-application-canvas-sjtu/lti3/lti3Auth/ivs?source=canvas"
        );
        assert_eq!(fields.get("id_token").map(String::as_str), Some("right"));
    }

    #[test]
    fn relative_initiation_action_keeps_video_api_prefix() {
        let url = absolute_action(
            "https://v.sjtu.edu.cn/jy-application-canvas-sjtu",
            "oidc/login_initiations",
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://v.sjtu.edu.cn/jy-application-canvas-sjtu/oidc/login_initiations"
        );
    }

    #[test]
    fn retry_after_rounds_up_and_never_returns_zero() {
        let now = Instant::now();
        assert_eq!(
            retry_after_seconds(now + Duration::from_millis(29_001), now),
            30
        );
        assert_eq!(retry_after_seconds(now, now), 1);
    }

    #[test]
    fn initiation_503_is_replayed_only_without_stateful_headers() {
        let mut headers = header::HeaderMap::new();
        assert!(retryable_lti_unavailable(
            StatusCode::SERVICE_UNAVAILABLE,
            &headers
        ));

        headers.insert(
            header::SET_COOKIE,
            header::HeaderValue::from_static("state=x"),
        );
        assert!(!retryable_lti_unavailable(
            StatusCode::SERVICE_UNAVAILABLE,
            &headers
        ));
        headers.remove(header::SET_COOKIE);
        headers.insert(
            header::LOCATION,
            header::HeaderValue::from_static("https://oc.sjtu.edu.cn/api/lti/authorize"),
        );
        assert!(!retryable_lti_unavailable(
            StatusCode::SERVICE_UNAVAILABLE,
            &headers
        ));
    }

    #[test]
    fn follows_canvas_lti_auto_submit_retry_form() {
        let config = lti_test_config();
        let page_url = Url::parse("https://oc.sjtu.edu.cn/api/lti/authorize").unwrap();
        let html = r#"
            <form id="retry_login" action="/api/lti/authorize" method="get">
              <input type="hidden" name="retried" value="true" />
              <input type="hidden" name="client_id" value="8329" />
            </form>
        "#;
        let Some(LtiFormStep::Submit {
            url,
            method,
            fields,
        }) = lti_form_step(html, &page_url, &config).unwrap()
        else {
            panic!("expected intermediate form");
        };
        assert!(matches!(method, LtiFormMethod::Get));
        assert_eq!(url.as_str(), "https://oc.sjtu.edu.cn/api/lti/authorize");
        assert_eq!(fields.get("retried").map(String::as_str), Some("true"));
    }

    #[test]
    fn extracts_token_id_from_meta_refresh_or_inline_json() {
        let meta = r#"<meta http-equiv="refresh" content="0;url=https://v.sjtu.edu.cn/ui/#/index?tokenId=hello%2Bworld">"#;
        assert_eq!(token_id_from_html(meta).as_deref(), Some("hello+world"));
        assert_eq!(
            token_id_from_html(r#"<script>window.payload={"tokenId":"direct-token"}</script>"#)
                .as_deref(),
            Some("direct-token")
        );
        assert_eq!(
            token_id_from_html(r#"<script>const next = `?tokenId=${token}`</script>"#),
            None
        );
    }

    #[test]
    fn rejects_semantic_launch_form_on_unconfigured_origin() {
        let config = lti_test_config();
        let page_url = Url::parse("https://oc.sjtu.edu.cn/api/lti/authorize").unwrap();
        let html = r#"
            <form action="https://example.com/lti3/launch" method="post">
              <input type="hidden" name="id_token" value="jwt" />
              <input type="hidden" name="state" value="state" />
            </form>
        "#;
        assert!(lti_form_step(html, &page_url, &config).is_err());
    }

    #[test]
    fn semantic_launch_requires_nonempty_hidden_fields_and_post() {
        let config = lti_test_config();
        let page_url = Url::parse("https://oc.sjtu.edu.cn/api/lti/authorize").unwrap();
        for html in [
            r#"<form action="https://v.sjtu.edu.cn/jy-application-canvas-sjtu/lti3/launch" method="get">
                  <input type="hidden" name="id_token" value="jwt" />
                  <input type="hidden" name="state" value="state" />
                </form>"#,
            r#"<form action="https://v.sjtu.edu.cn/jy-application-canvas-sjtu/lti3/launch" method="post">
                  <input type="text" name="id_token" value="jwt" />
                  <input type="hidden" name="state" value="state" />
                </form>"#,
            r#"<form action="https://v.sjtu.edu.cn/jy-application-canvas-sjtu/lti3/launch" method="post">
                  <input type="hidden" name="id_token" value="" />
                  <input type="hidden" name="state" value="state" />
                </form>"#,
        ] {
            assert!(lti_form_step(html, &page_url, &config).unwrap().is_none());
        }
    }

    #[test]
    fn rejects_ambiguous_semantic_launch_forms() {
        let config = lti_test_config();
        let page_url = Url::parse("https://oc.sjtu.edu.cn/api/lti/authorize").unwrap();
        let html = r#"
            <form action="https://v.sjtu.edu.cn/jy-application-canvas-sjtu/lti3/launch-a" method="post">
              <input type="hidden" name="id_token" value="jwt-a" />
              <input type="hidden" name="state" value="state-a" />
            </form>
            <form action="https://v.sjtu.edu.cn/jy-application-canvas-sjtu/lti3/launch-b" method="post">
              <input type="hidden" name="id_token" value="jwt-b" />
              <input type="hidden" name="state" value="state-b" />
            </form>
        "#;
        assert!(lti_form_step(html, &page_url, &config).is_err());
    }

    #[test]
    fn video_form_action_must_stay_under_configured_path() {
        let configured = "https://v.sjtu.edu.cn/jy-application-canvas-sjtu";
        let allowed =
            Url::parse("https://v.sjtu.edu.cn/jy-application-canvas-sjtu/lti3/launch").unwrap();
        let outside = Url::parse("https://v.sjtu.edu.cn/other-app/lti3/launch").unwrap();
        assert!(validate_video_action(&allowed, configured).is_ok());
        assert!(validate_video_action(&outside, configured).is_err());
    }
}
