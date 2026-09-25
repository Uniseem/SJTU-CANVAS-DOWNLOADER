//! Classroom recordings from the SJTU video platform (v.sjtu.edu.cn, the
//! resource-management platform Canvas switched to in August 2026).
//!
//! Canvas opens the platform's LTI 1.3 tool through `jy-lti-adapter`. The
//! launch ends on the platform's web UI with a `jwt_token`, which the engine
//! keeps in memory and sends as the `jwt-token` header to the platform API
//! (see `video/resource.rs`). The former canvas-sjtu API and the LTI 1.1
//! "课堂视频旧版" player are gone; neither is tried any more.

use std::{collections::HashMap, sync::Arc, time::Duration};

use dashmap::DashMap;
use reqwest::{Client, StatusCode, cookie::Jar, header, redirect::Policy};
use scraper::{ElementRef, Html, Selector};
use tokio::{sync::Mutex, time::Instant};
use url::Url;

use crate::{
    canvas::{build_client, safe_filename, validate_canvas_id, validate_generated_url, value_id},
    config::Config,
    error::{AppError, AppResult},
    models::{Lesson, VideoTrack},
};

mod resource;
mod sizes;

const DEFAULT_TOOL_ID: &str = "8329";
const LTI_UNAVAILABLE_RETRY_AFTER: Duration = Duration::from_secs(30);
const LTI_RETRY_JITTER_MIN_MS: u64 = 250;
const LTI_RETRY_JITTER_SPAN_MS: u16 = 501;
const SOURCE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Default)]
struct LtiCircuit {
    unavailable_until: Option<Instant>,
}

#[derive(Clone)]
struct VideoSession {
    /// The platform JWT; sent as `jwt-token`, never written to disk.
    token: String,
    /// The teaching class (教学班) the Canvas course maps to.
    teaching_class_id: String,
}

#[derive(Clone)]
struct CachedCourse {
    session: VideoSession,
    lessons: Vec<Lesson>,
    expires_at: Instant,
}

/// The camera views of one recording.
#[derive(Clone, Debug)]
struct VideoDetail {
    tracks: Vec<VideoTrack>,
}

#[derive(Clone)]
struct CachedDetail {
    detail: VideoDetail,
    expires_at: Instant,
}

/// A media URL ready to download, with the headers its host expects.
pub struct ResolvedVideo {
    pub upstream_url: Url,
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
        let mut lesson = find_lesson(&cached, lesson_id)
            .ok_or_else(|| AppError::NotFound("该讲次不存在、未开放，或不属于当前课程".into()))?;

        let detail = match self
            .cached_video_detail(owner, course_id, jar.clone(), &cached.session, lesson_id)
            .await
        {
            Ok(detail) => detail,
            Err(first_error) => {
                // The JWT may have expired: authorize once more and retry.
                self.cache
                    .remove(&(owner.to_string(), course_id.to_string()));
                self.detail_cache.remove(&(
                    owner.to_string(),
                    course_id.to_string(),
                    lesson_id.to_string(),
                ));
                cached = self.course(owner, jar.clone(), course_id, true).await?;
                lesson = find_lesson(&cached, lesson_id).ok_or_else(|| {
                    AppError::NotFound("重新授权后该讲次已不可用或不属于当前课程".into())
                })?;
                self.cached_video_detail(owner, course_id, jar.clone(), &cached.session, lesson_id)
                    .await
                    .map_err(|_| first_error)?
            }
        };
        let requested = track_code(track_kind)?;
        let track = detail
            .tracks
            .iter()
            .find(|track| track.view == requested)
            .ok_or_else(|| {
                let available = detail
                    .tracks
                    .iter()
                    .map(|track| track_label(track.view))
                    .collect::<Vec<_>>()
                    .join("、");
                AppError::NotFound(format!(
                    "当前讲次没有所选画面{}",
                    if available.is_empty() {
                        String::new()
                    } else {
                        format!("，可用画面：{available}")
                    }
                ))
            })?;
        resource::resolve_resource_track(&lesson, track, &self.config)
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
        let (session, lessons) = source_with_timeout(SOURCE_TIMEOUT, "课堂视频平台", async {
            let session = self.authorize(jar.clone(), course_id).await?;
            let lessons = self.resource_lessons(jar.clone(), &session).await?;
            Ok((session, lessons))
        })
        .await?;
        let cache_ttl = if lessons.is_empty() { 30 } else { 15 * 60 };
        let cached = CachedCourse {
            session,
            lessons,
            expires_at: Instant::now() + Duration::from_secs(cache_ttl),
        };
        self.cache.insert(key, cached.clone());
        Ok(cached)
    }

    /// Launches the course's 课堂视频 tool in Canvas and completes the LTI
    /// login with the video platform.
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
            |action| action.contains(resource::INITIATION_PATH),
            "未识别到新版课堂视频的授权入口：该课程可能还没有接入新版平台，请从 Canvas 官网核对课程的“课堂视频”入口",
        )?;
        self.authorize_resource_video(
            &client,
            &no_redirect,
            &initiation_action,
            &initiation_fields,
        )
        .await
    }

    async fn initiate_lti(
        &self,
        client: &Client,
        initiation_url: &Url,
        initiation_fields: &HashMap<String, String>,
    ) -> AppResult<reqwest::Response> {
        // Serialize only the short initiation exchange. If the shared video
        // gateway is down, the first caller performs the sole retry and opens
        // the circuit; concurrent course loads then fail fast instead of
        // amplifying the outage into one request pair per course.
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
        // a cookie mutation. Never replay later launch mutations here.
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

    /// The id of the course's 课堂视频 external tool, read from the course
    /// navigation; the school-wide default when the page cannot be read.
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
        external_tool_id_from_html(&html).unwrap_or_else(|| DEFAULT_TOOL_ID.into())
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
        let detail = self.resource_video_detail(jar, session, lesson_id).await?;
        self.detail_cache.insert(
            key,
            CachedDetail {
                detail: detail.clone(),
                expires_at: Instant::now() + Duration::from_secs(90),
            },
        );
        Ok(detail)
    }
}

fn find_lesson(course: &CachedCourse, lesson_id: &str) -> Option<Lesson> {
    course
        .lessons
        .iter()
        .find(|lesson| lesson.video_id == lesson_id && lesson.available)
        .cloned()
}

fn external_tool_id_from_html(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("a[href*='/external_tools/']").ok()?;
    document.select(&selector).find_map(|link| {
        let text = link.text().collect::<String>();
        let href = link.value().attr("href")?;
        if !text.contains("课堂视频") || text.contains("旧版") {
            return None;
        }
        href.split("/external_tools/")
            .nth(1)?
            .split(['?', '/'])
            .next()
            .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
            .map(str::to_string)
    })
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
        "课堂视频平台当前维护或过载，请稍后重试",
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

/// The OIDC launch form (`id_token` + `state`) that ends the LTI login.
struct LtiLaunch {
    url: Url,
    fields: HashMap<String, String>,
}

enum LtiFormStep {
    Launch(LtiLaunch),
    /// An intermediate Canvas form (`/api/lti/authorize`) to submit first.
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

/// Follows Canvas's LTI authorization pages after the OIDC initiation until
/// they produce the launch form for the video platform.
async fn resolve_lti_launch(
    service: &VideoService,
    client: &Client,
    config: &Config,
    mut response: reqwest::Response,
) -> AppResult<LtiLaunch> {
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
        let html = response.text().await?;
        match lti_form_step(&html, &page_url, config)? {
            Some(LtiFormStep::Launch(launch)) => return Ok(launch),
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

    let mut launches = forms.iter().filter(|form| is_lti_launch_form(**form));
    if let Some(form) = launches.next() {
        if launches.next().is_some() {
            return Err(AppError::Upstream(
                "课堂视频 LTI 鉴权页面包含多个授权表单".into(),
            ));
        }
        let action = form.value().attr("action").unwrap_or_default();
        let url = resolve_video_form_action(page_url, &config.video_lti_adapter, action)?;
        return Ok(Some(LtiFormStep::Launch(LtiLaunch {
            url,
            fields: form_inputs(*form),
        })));
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

fn is_lti_launch_form(form: ElementRef<'_>) -> bool {
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

/// `url` must be a public address on the configured service, at or below
/// the service's path.
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

/// A query parameter of `value`, looked up in the query and in a query
/// carried by the fragment (`#/route?key=value`, as single-page apps do).
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

/// The platform's view number for a track name the apps use.
fn track_code(value: &str) -> AppResult<i64> {
    match value {
        "teacher" | "0" | "教师" => Ok(0),
        "student1" | "1" | "学生1" => Ok(1),
        "student2" | "2" | "学生2" => Ok(2),
        "slides" | "ppt" | "3" | "PPT" => Ok(3),
        "composite" | "4" | "合成" => Ok(4),
        _ => Err(AppError::BadRequest("未知的视频画面".into())),
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

pub(crate) fn validate_resource_id(value: &str) -> AppResult<()> {
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

    pub(crate) fn lti_test_config() -> Config {
        Config::for_tests()
    }

    #[test]
    fn parses_parameters_from_fragment_routes() {
        let url = "https://v.sjtu.edu.cn/jy-application-resourcemanage-ui/#/lms/launch?jwt_token=hello%2Bworld";
        assert_eq!(
            redirect_parameter(url, "jwt_token").as_deref(),
            Some("hello+world")
        );
        assert_eq!(redirect_parameter(url, "other"), None);
    }

    #[test]
    fn canvas_launch_page_yields_the_adapter_initiation_form() {
        let html = r#"
            <form action="https://oc.sjtu.edu.cn/other" method="post"><input name="x" value="1"></form>
            <form action="https://v.sjtu.edu.cn/jy-lti-adapter/lti/canvas/oidc/login-initiation/canvas-record" method="post">
              <input type="hidden" name="iss" value="https://canvas.instructure.com">
              <input type="hidden" name="login_hint" value="hint">
              <input type="hidden" name="target_link_uri" value="https://v.sjtu.edu.cn/jy-lti-adapter/lti/canvas/launch/canvas-record">
            </form>
        "#;
        let (action, fields) = find_form(
            html,
            |action| action.contains(resource::INITIATION_PATH),
            "missing",
        )
        .unwrap();
        assert!(action.ends_with(resource::INITIATION_PATH));
        assert_eq!(fields["login_hint"], "hint");
        assert!(
            find_form(
                r#"<form action="https://v.sjtu.edu.cn/jy-application-canvas-sjtu/oidc/login_initiations"></form>"#,
                |action| action.contains(resource::INITIATION_PATH),
                "missing"
            )
            .is_err()
        );
    }

    #[test]
    fn course_navigation_names_the_new_video_tool() {
        let html = r#"
            <a href="/courses/1/external_tools/1234">课堂视频旧版</a>
            <a href="/courses/1/external_tools/9001?display=borderless">课堂视频</a>
        "#;
        assert_eq!(external_tool_id_from_html(html).as_deref(), Some("9001"));
        assert_eq!(
            external_tool_id_from_html(r#"<a href="/courses/1/external_tools/1234">课堂视频旧版</a>"#),
            None
        );
    }

    #[test]
    fn recognizes_the_adapter_launch_form_by_oidc_fields() {
        let config = lti_test_config();
        let page_url = Url::parse("https://oc.sjtu.edu.cn/api/lti/authorize").unwrap();
        let html = r#"
            <form action="https://v.sjtu.edu.cn/jy-lti-adapter/lti/canvas/launch/canvas-record" method="post">
              <input type="hidden" name="id_token" value="jwt" />
              <input type="hidden" name="state" value="state" />
            </form>
        "#;
        let Some(LtiFormStep::Launch(LtiLaunch { url, fields })) =
            lti_form_step(html, &page_url, &config).unwrap()
        else {
            panic!("expected launch form");
        };
        assert_eq!(
            url.as_str(),
            "https://v.sjtu.edu.cn/jy-lti-adapter/lti/canvas/launch/canvas-record"
        );
        assert_eq!(fields.get("id_token").map(String::as_str), Some("jwt"));
    }

    #[test]
    fn relative_launch_action_keeps_the_adapter_prefix() {
        let config = lti_test_config();
        let page_url = Url::parse("https://oc.sjtu.edu.cn/api/lti/authorize").unwrap();
        let html = r#"
            <form action="/lti/canvas/launch/canvas-record" method="post">
              <input type="hidden" name="id_token" value="jwt" />
              <input type="hidden" name="state" value="state" />
            </form>
        "#;
        let Some(LtiFormStep::Launch(LtiLaunch { url, .. })) =
            lti_form_step(html, &page_url, &config).unwrap()
        else {
            panic!("expected launch form");
        };
        assert_eq!(
            url.as_str(),
            "https://v.sjtu.edu.cn/jy-lti-adapter/lti/canvas/launch/canvas-record"
        );
        let url = absolute_action(
            "https://v.sjtu.edu.cn/jy-lti-adapter",
            "lti/canvas/oidc/login-initiation/canvas-record",
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://v.sjtu.edu.cn/jy-lti-adapter/lti/canvas/oidc/login-initiation/canvas-record"
        );
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
    fn rejects_launch_forms_on_unconfigured_origins() {
        let config = lti_test_config();
        let page_url = Url::parse("https://oc.sjtu.edu.cn/api/lti/authorize").unwrap();
        for action in [
            "https://example.com/lti/canvas/launch/canvas-record",
            "https://v.sjtu.edu.cn/jy-application-canvas-sjtu/lti3/launch",
        ] {
            let html = format!(
                r#"<form action="{action}" method="post">
                  <input type="hidden" name="id_token" value="jwt" />
                  <input type="hidden" name="state" value="state" />
                </form>"#
            );
            assert!(lti_form_step(&html, &page_url, &config).is_err());
        }
    }

    #[test]
    fn launch_form_requires_nonempty_hidden_fields_and_post() {
        let config = lti_test_config();
        let page_url = Url::parse("https://oc.sjtu.edu.cn/api/lti/authorize").unwrap();
        for html in [
            r#"<form action="https://v.sjtu.edu.cn/jy-lti-adapter/lti/canvas/launch/canvas-record" method="get">
                  <input type="hidden" name="id_token" value="jwt" />
                  <input type="hidden" name="state" value="state" />
                </form>"#,
            r#"<form action="https://v.sjtu.edu.cn/jy-lti-adapter/lti/canvas/launch/canvas-record" method="post">
                  <input type="text" name="id_token" value="jwt" />
                  <input type="hidden" name="state" value="state" />
                </form>"#,
            r#"<form action="https://v.sjtu.edu.cn/jy-lti-adapter/lti/canvas/launch/canvas-record" method="post">
                  <input type="hidden" name="id_token" value="" />
                  <input type="hidden" name="state" value="state" />
                </form>"#,
        ] {
            assert!(lti_form_step(html, &page_url, &config).unwrap().is_none());
        }
    }

    #[test]
    fn rejects_ambiguous_launch_forms() {
        let config = lti_test_config();
        let page_url = Url::parse("https://oc.sjtu.edu.cn/api/lti/authorize").unwrap();
        let html = r#"
            <form action="https://v.sjtu.edu.cn/jy-lti-adapter/lti/canvas/launch/canvas-record" method="post">
              <input type="hidden" name="id_token" value="jwt-a" />
              <input type="hidden" name="state" value="state-a" />
            </form>
            <form action="https://v.sjtu.edu.cn/jy-lti-adapter/lti/canvas/launch/other" method="post">
              <input type="hidden" name="id_token" value="jwt-b" />
              <input type="hidden" name="state" value="state-b" />
            </form>
        "#;
        assert!(lti_form_step(html, &page_url, &config).is_err());
    }

    #[test]
    fn video_form_action_must_stay_under_configured_path() {
        let configured = "https://v.sjtu.edu.cn/jy-lti-adapter";
        let allowed =
            Url::parse("https://v.sjtu.edu.cn/jy-lti-adapter/lti/canvas/launch/canvas-record")
                .unwrap();
        let outside = Url::parse("https://v.sjtu.edu.cn/other-app/lti/canvas/launch").unwrap();
        assert!(validate_video_action(&allowed, configured).is_ok());
        assert!(validate_video_action(&outside, configured).is_err());
        assert!(validate_video_action(&Url::parse("https://v.sjtu.edu.cn/jy-lti-adapter").unwrap(), configured).is_ok());
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
    fn track_names_map_to_platform_view_codes() {
        assert_eq!(track_code("teacher").unwrap(), 0);
        assert_eq!(track_code("slides").unwrap(), 3);
        assert_eq!(track_code("composite").unwrap(), 4);
        assert!(track_code("audio").is_err());
        assert_eq!(track_label(3), "PPT");
    }
}
