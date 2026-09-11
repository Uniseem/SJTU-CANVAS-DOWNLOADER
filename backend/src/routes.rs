use std::{convert::Infallible, sync::Arc, time::Duration};

use async_stream::stream;
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Extension, MatchedPath, Path, Query, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::{
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

use crate::{
    canvas::CanvasClient,
    downloads::{PrepareRequest, PrepareResponse},
    error::{AppError, AppResult},
    models::{
        Assignment, CanvasFile, Course, Dashboard, DownloadDescriptor, Lesson, SessionView,
        TodoItem,
    },
    session::{SessionId, random_token},
    state::AppState,
};

#[cfg(test)]
mod tests;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartLoginResponse {
    attempt_id: String,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/session", get(session_view).delete(logout))
        .route("/auth/qr", post(start_login))
        .route(
            "/auth/qr/{attempt_id}",
            get(login_status).delete(cancel_login),
        )
        .route("/auth/qr/{attempt_id}/events", get(login_events))
        .route("/auth/qr/{attempt_id}/refresh", post(refresh_login))
        .route("/dashboard", get(dashboard))
        .route("/courses", get(courses))
        .route("/courses/{course_id}/files", get(files))
        .route("/courses/{course_id}/lessons", get(lessons))
        .route(
            "/courses/{course_id}/lessons/{lesson_id}/sizes",
            get(lesson_sizes),
        )
        .route("/courses/{course_id}/assignments", get(assignments))
        .route("/downloads/prepare", post(prepare_downloads))
        .route("/downloads/proxy/{ticket_id}", get(proxy_download))
        .route("/demo/download/{id}", get(demo_download))
        .fallback(api_not_found)
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            api_security_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            session_middleware,
        ))
        .with_state(state.clone());

    let index = ServeFile::new(state.config.web_dist.join("index.html"));
    let static_files = ServeDir::new(&state.config.web_dist).fallback(index);
    Router::new()
        .nest("/api", api)
        .fallback_service(static_files)
        .layer(middleware::from_fn(security_headers))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn api_not_found() -> AppError {
    AppError::NotFound("API 路由不存在".into())
}

async fn session_view(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
) -> AppResult<Json<SessionView>> {
    let view = state.sessions.view(&session.0).await;
    if !view.authenticated || state.config.demo_mode {
        return Ok(Json(view));
    }
    let (_, jar, saved_profile) = state.sessions.authenticated(&session.0).await?;
    let canvas = CanvasClient::new(state.config.clone(), jar.clone())?;
    match canvas.profile().await {
        Ok(profile) if profile.id == saved_profile.id => Ok(Json(SessionView {
            authenticated: true,
            demo: false,
            profile: Some(profile),
        })),
        Ok(_) | Err(AppError::Unauthorized) => {
            if state.sessions.logout_if_current(&session.0, &jar).await? {
                state.videos.clear_owner(&session.0);
                state.downloads.clear_owner(&session.0);
            }
            Ok(Json(state.sessions.view(&session.0).await))
        }
        Err(_) => Ok(Json(view)),
    }
}

async fn start_login(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
) -> AppResult<impl IntoResponse> {
    if state.config.demo_mode {
        return Ok(Json(StartLoginResponse {
            attempt_id: "demo".into(),
        }));
    }
    let attempt = state.auth.start(&session.0).await?;
    Ok(Json(StartLoginResponse {
        attempt_id: attempt.id.clone(),
    }))
}

async fn login_status(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
    Path(attempt_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let attempt = state.auth.get_owned(&attempt_id, &session.0)?;
    Ok(Json(attempt.latest().await))
}

async fn login_events(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
    Path(attempt_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let attempt = state.auth.get_owned(&attempt_id, &session.0)?;
    let receiver = attempt.subscribe();
    let initial = attempt.latest().await;
    let updates = BroadcastStream::new(receiver).filter_map(|message| message.ok());
    let events = stream! {
        yield Ok::<Event, Infallible>(Event::default().event("status").json_data(initial).unwrap_or_else(|_| Event::default()));
        tokio::pin!(updates);
        while let Some(update) = updates.next().await {
            yield Ok(Event::default().event("status").json_data(update).unwrap_or_else(|_| Event::default()));
        }
    };
    Ok(Sse::new(events).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    ))
}

async fn refresh_login(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
    Path(attempt_id): Path<String>,
) -> AppResult<StatusCode> {
    state.auth.refresh(&attempt_id, &session.0).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn cancel_login(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
    Path(attempt_id): Path<String>,
) -> AppResult<StatusCode> {
    state.auth.cancel(&attempt_id, &session.0).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn logout(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
) -> AppResult<StatusCode> {
    state.auth.cancel_owner(&session.0).await;
    state.videos.clear_owner(&session.0);
    state.downloads.clear_owner(&session.0);
    state.sessions.logout(&session.0).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn dashboard(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
) -> AppResult<Json<Dashboard>> {
    let (_, jar, profile) = state.sessions.authenticated(&session.0).await?;
    if state.config.demo_mode {
        return Ok(Json(Dashboard {
            profile,
            courses: demo_courses(),
            todos: demo_todos(),
        }));
    }
    let canvas = Arc::new(CanvasClient::new(state.config.clone(), jar)?);
    let (courses, todos) = tokio::try_join!(canvas.courses(), canvas.todos())?;
    Ok(Json(Dashboard {
        profile,
        courses,
        todos,
    }))
}

async fn courses(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
) -> AppResult<Json<Vec<Course>>> {
    let (_, jar, _) = state.sessions.authenticated(&session.0).await?;
    if state.config.demo_mode {
        return Ok(Json(demo_courses()));
    }
    Ok(Json(
        CanvasClient::new(state.config.clone(), jar)?
            .courses()
            .await?,
    ))
}

async fn files(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
    Path(course_id): Path<String>,
) -> AppResult<Json<Vec<CanvasFile>>> {
    let (_, jar, _) = state.sessions.authenticated(&session.0).await?;
    if state.config.demo_mode {
        return Ok(Json(demo_files()));
    }
    Ok(Json(
        CanvasClient::new(state.config.clone(), jar)?
            .files(&course_id)
            .await?,
    ))
}

async fn lessons(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
    Path(course_id): Path<String>,
) -> AppResult<Json<Vec<Lesson>>> {
    let (_, jar, _) = state.sessions.authenticated(&session.0).await?;
    if state.config.demo_mode {
        return Ok(Json(demo_lessons()));
    }
    Ok(Json(
        state.videos.lessons(&session.0, jar, &course_id).await?,
    ))
}

async fn assignments(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
    Path(course_id): Path<String>,
) -> AppResult<Json<Vec<Assignment>>> {
    let (_, jar, _) = state.sessions.authenticated(&session.0).await?;
    if state.config.demo_mode {
        return Ok(Json(demo_assignments()));
    }
    Ok(Json(
        CanvasClient::new(state.config.clone(), jar)?
            .assignments(&course_id)
            .await?,
    ))
}

#[derive(Deserialize)]
struct SizeQuery {
    #[serde(default = "default_size_tracks")]
    tracks: String,
    #[serde(default)]
    refresh: bool,
}
fn default_size_tracks() -> String {
    "slides,teacher".into()
}

async fn lesson_sizes(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
    Path((course_id, lesson_id)): Path<(String, String)>,
    Query(query): Query<SizeQuery>,
) -> AppResult<Json<crate::models::LessonSizes>> {
    let (_, jar, _) = state.sessions.authenticated(&session.0).await?;
    Ok(Json(
        state
            .videos
            .lesson_sizes(
                &session.0,
                jar,
                &course_id,
                &lesson_id,
                &query.tracks,
                query.refresh,
            )
            .await?,
    ))
}

async fn prepare_downloads(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
    Json(request): Json<PrepareRequest>,
) -> AppResult<PrepareResponse> {
    if request.items.is_empty() {
        return Err(AppError::BadRequest("请至少选择一个下载项目".into()));
    }
    if request.items.len() > 100 {
        return Err(AppError::BadRequest("单次最多准备 100 个下载项目".into()));
    }
    if state.config.demo_mode {
        let expires_at = (Utc::now() + chrono::Duration::minutes(15)).to_rfc3339();
        let items = request
            .items
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let id = random_token(12);
                DownloadDescriptor {
                    id: id.clone(),
                    filename: format!("Canvas-演示下载-{:02}.txt", index + 1),
                    size: Some(256 * 1024),
                    direct_url: None,
                    proxy_url: format!("/api/demo/download/{id}"),
                    expires_at: expires_at.clone(),
                    source: "demo".into(),
                    direct_supported: false,
                }
            })
            .collect();
        return Ok(PrepareResponse {
            items,
            failures: Vec::new(),
        });
    }
    state.downloads.prepare(&session.0, request).await
}

async fn proxy_download(
    State(state): State<AppState>,
    Extension(session): Extension<SessionId>,
    Path(ticket_id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Response> {
    state
        .downloads
        .proxy(&session.0, &ticket_id, &headers)
        .await
}

async fn demo_download(
    State(state): State<AppState>,
    Path(_id): Path<String>,
) -> AppResult<Response> {
    if !state.config.demo_mode {
        return Err(AppError::NotFound("资源不存在".into()));
    }
    let line = "Canvas Pocket 演示下载：实际部署时这里会流式传输课程文件或课堂录像。\n";
    let mut content = String::with_capacity(256 * 1024);
    while content.len() < 256 * 1024 {
        content.push_str(line);
    }
    let mut response = Response::new(Body::from(content.into_bytes()));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"canvas-pocket-demo.txt\""),
    );
    Ok(response)
}

async fn session_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    // Container health probes do not carry a cookie. Bypass session creation so a
    // long-running instance cannot accumulate one anonymous session per probe.
    if matches!(request.uri().path(), "/health" | "/api/health") {
        return next.run(request).await;
    }
    let incoming = request
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| cookie_value(cookies, state.config.session_cookie_name()));
    let (session, created) = state.sessions.ensure(incoming.as_deref()).await;
    request
        .extensions_mut()
        .insert(SessionId(session.id.clone()));
    let mut response = next.run(request).await;
    if created {
        let value = state
            .config
            .cookie_header(state.config.session_ttl)
            .replace("{sid}", &session.id);
        if let Ok(value) = HeaderValue::from_str(&value) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}

async fn api_security_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let session_id = request.extensions().get::<SessionId>().cloned();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned())
        .unwrap_or_else(|| "unknown".into());
    // Snapshot the authentication used by this request. Parallel 401s from
    // an old page must not log out a newer login or cancel its QR attempt.
    let request_auth = if let Some(session) = &session_id {
        state
            .sessions
            .authenticated(&session.0)
            .await
            .ok()
            .map(|(_, jar, profile)| (jar, profile.id))
    } else {
        None
    };
    if matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) {
        if request
            .headers()
            .get("sec-fetch-site")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value == "cross-site")
        {
            return AppError::Forbidden("拒绝跨站请求".into()).into_response();
        }
        if let (Some(public_url), Some(origin)) = (
            state.config.public_url.as_deref(),
            request
                .headers()
                .get(header::ORIGIN)
                .and_then(|value| value.to_str().ok()),
        ) {
            let expected = public_url.trim_end_matches('/');
            if origin.trim_end_matches('/') != expected {
                return AppError::Forbidden("请求来源与 PUBLIC_URL 不一致".into()).into_response();
            }
        }
    }
    let mut response = next.run(request).await;
    if response.status() == StatusCode::UNAUTHORIZED
        && let Some(session) = session_id
        && let Some((jar, profile_id)) = request_auth
    {
        response = resolve_unauthorized(&state, &session.0, jar, &profile_id, &route)
            .await
            .into_response();
    }
    response
        .headers_mut()
        .entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
    response
}

async fn resolve_unauthorized(
    state: &AppState,
    session_id: &str,
    jar: Arc<reqwest::cookie::Jar>,
    profile_id: &str,
    route: &str,
) -> AppError {
    // Canvas also uses 401 for resource permission failures. Only the identity
    // endpoint can confirm whether the overall Canvas login has expired.
    let check = async {
        CanvasClient::new(state.config.clone(), jar.clone())?
            .profile()
            .await
    };
    let identity = tokio::time::timeout(Duration::from_secs(8), check).await;
    // A completed QR login may have replaced the jar while this request was
    // in flight. Do not send a stale 401 that would log out the new frontend.
    let still_current = state
        .sessions
        .authenticated(session_id)
        .await
        .is_ok_and(|(_, current, _)| Arc::ptr_eq(&jar, &current));
    if !still_current {
        return AppError::Conflict("登录状态已更新，请重试当前操作".into());
    }
    match identity {
        Ok(Ok(profile)) if profile.id == profile_id => {
            tracing::warn!(
                route,
                "resource denied but Canvas identity is valid; keeping login"
            );
            AppError::Forbidden(
                "当前账号无权访问此资源，课程可能尚未开放或未启用该功能；登录仍然有效".into(),
            )
        }
        Ok(Ok(_)) | Ok(Err(AppError::Unauthorized)) => {
            match state.sessions.logout_if_current(session_id, &jar).await {
                Ok(true) => {
                    tracing::info!(route, "Canvas identity no longer valid; login expired");
                    state.videos.clear_owner(session_id);
                    state.downloads.clear_owner(session_id);
                    AppError::Unauthorized
                }
                Ok(false) => AppError::Conflict("登录状态已更新，请重试当前操作".into()),
                Err(error) => error,
            }
        }
        Ok(Err(_)) | Err(_) => {
            tracing::warn!(route, "Canvas identity check unavailable; keeping login");
            AppError::upstream_unavailable("暂时无法核验 Canvas 登录状态，请稍后重试", 5)
        }
    }
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' data: https:; connect-src 'self' https:; style-src 'self' 'unsafe-inline'; font-src 'self' data:; script-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'",
        ),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    response
}

fn cookie_value(cookies: &str, name: &str) -> Option<String> {
    cookies.split(';').find_map(|cookie| {
        let (key, value) = cookie.trim().split_once('=')?;
        (key == name).then(|| value.to_string())
    })
}

fn demo_courses() -> Vec<Course> {
    vec![
        Course {
            id: "87954".into(),
            name: "机器学习与数据挖掘（2026 春）".into(),
            course_code: "CS3611".into(),
            start_at: Some("2026-02-23T00:00:00Z".into()),
            end_at: Some("2026-07-05T00:00:00Z".into()),
            term: Some("2026 春季学期".into()),
            teacher: Some("陈老师".into()),
            enrollment_state: "active".into(),
        },
        Course {
            id: "88148".into(),
            name: "无线通信原理".into(),
            course_code: "EE3402".into(),
            start_at: Some("2026-02-23T00:00:00Z".into()),
            end_at: Some("2026-07-05T00:00:00Z".into()),
            term: Some("2026 春季学期".into()),
            teacher: Some("何老师".into()),
            enrollment_state: "active".into(),
        },
        Course {
            id: "76231".into(),
            name: "大学物理（荣誉）".into(),
            course_code: "PHYS1201".into(),
            start_at: Some("2025-09-08T00:00:00Z".into()),
            end_at: Some("2026-01-18T00:00:00Z".into()),
            term: Some("2025 秋季学期".into()),
            teacher: Some("周老师".into()),
            enrollment_state: "completed".into(),
        },
    ]
}

fn demo_todos() -> Vec<TodoItem> {
    vec![
        TodoItem {
            id: "99001".into(),
            title: "作业 4：卷积网络".into(),
            course_name: "机器学习与数据挖掘".into(),
            due_at: Some("2026-08-22T15:59:00Z".into()),
            points_possible: Some(20.0),
            submitted: false,
        },
        TodoItem {
            id: "99002".into(),
            title: "Lab 6 报告".into(),
            course_name: "无线通信原理".into(),
            due_at: Some("2026-08-25T15:59:00Z".into()),
            points_possible: Some(100.0),
            submitted: true,
        },
    ]
}

fn demo_files() -> Vec<CanvasFile> {
    vec![
        CanvasFile {
            id: "50001".into(),
            display_name: "第 08 讲 · 卷积神经网络.pdf".into(),
            filename: "lecture-08-cnn.pdf".into(),
            size: 8_431_616,
            content_type: Some("application/pdf".into()),
            updated_at: Some("2026-08-18T08:20:00Z".into()),
            url: None,
        },
        CanvasFile {
            id: "50002".into(),
            display_name: "课程资料与数据集.zip".into(),
            filename: "course-assets.zip".into(),
            size: 128_761_233,
            content_type: Some("application/zip".into()),
            updated_at: Some("2026-08-16T06:10:00Z".into()),
            url: None,
        },
    ]
}

fn demo_lessons() -> Vec<Lesson> {
    (1..=8)
        .map(|index| Lesson {
            video_id: format!("demo-video-{index:02}"),
            title: format!(
                "第 {index:02} 讲 · {}",
                [
                    "课程导论",
                    "线性模型",
                    "反向传播",
                    "优化方法",
                    "卷积网络",
                    "循环网络",
                    "注意力机制",
                    "课程总结"
                ][(index - 1) as usize]
            ),
            begin_time: format!("2026-08-{:02} 08:00", index + 1),
            end_time: format!("2026-08-{:02} 09:40", index + 1),
            classroom: "东中院 4-202".into(),
            audit_status: 3,
            available: true,
            source: None,
        })
        .collect()
}

fn demo_assignments() -> Vec<Assignment> {
    vec![
        Assignment {
            id: "60001".into(),
            name: "作业 4：卷积网络".into(),
            due_at: Some("2026-08-22T15:59:00Z".into()),
            points_possible: Some(20.0),
            submission_state: "unsubmitted".into(),
        },
        Assignment {
            id: "60002".into(),
            name: "期中项目".into(),
            due_at: Some("2026-09-05T15:59:00Z".into()),
            points_possible: Some(100.0),
            submission_state: "submitted".into(),
        },
    ]
}
