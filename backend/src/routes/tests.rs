use super::*;
use axum::{body::to_bytes, response::Html};
use reqwest::cookie::Jar;
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::Semaphore, task::JoinHandle};
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    app: Router,
    session_id: String,
    upstream: JoinHandle<()>,
    _directory: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.upstream.abort();
    }
}

fn profile() -> crate::models::CanvasProfile {
    crate::models::CanvasProfile {
        id: "42".into(),
        name: "测试用户".into(),
        short_name: "测试".into(),
        avatar_url: None,
    }
}

impl Fixture {
    async fn new(identity: Router) -> Self {
        // A course can appear in the list while its content is permission denied.
        let upstream_app = identity
            .route(
                "/api/v1/courses/94198/files",
                get(|| async { StatusCode::UNAUTHORIZED }),
            )
            .route("/api/v1/courses", get(|| async { Json(json!([])) }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let upstream = tokio::spawn(async move {
            axum::serve(listener, upstream_app).await.unwrap();
        });
        let directory = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::from_env().unwrap();
        config.canvas_origin = format!("http://{address}");
        config.data_dir = directory.path().to_path_buf();
        config.web_dist = directory.path().to_path_buf();
        config.app_secret = Some("integration-test-only".into());
        config.public_url = None;
        config.cookie_secure = false;
        config.demo_mode = false;
        config.session_ttl = Duration::from_secs(3600);
        let state = AppState::new(config).await.unwrap();
        let (session, _) = state.sessions.ensure(None).await;
        state
            .sessions
            .complete_login(&session.id, Arc::new(Jar::default()), profile())
            .await
            .unwrap();
        Self {
            app: router(state.clone()),
            state,
            session_id: session.id.clone(),
            upstream,
            _directory: directory,
        }
    }

    fn request(&self, path: &str) -> Request {
        Request::builder()
            .uri(path)
            .header(header::COOKIE, format!("canvas_sid={}", self.session_id))
            .body(Body::empty())
            .unwrap()
    }

    async fn get(&self, path: &str) -> Response {
        self.app.clone().oneshot(self.request(path)).await.unwrap()
    }
}

async fn json_body(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap()
}

#[tokio::test]
async fn course_401_after_login_keeps_session_and_other_courses_usable() {
    let fixture = Fixture::new(Router::new().route(
        "/api/v1/users/self/profile",
        get(|| async { Json(profile()) }),
    ))
    .await;
    assert_eq!(
        json_body(fixture.get("/api/session").await).await["authenticated"],
        true
    );
    let denied = fixture.get("/api/courses/94198/files").await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_body(denied).await["error"]["code"], "forbidden");
    assert!(
        fixture
            .state
            .sessions
            .view(&fixture.session_id)
            .await
            .authenticated
    );
    assert_eq!(
        json_body(fixture.get("/api/session").await).await["authenticated"],
        true
    );
    assert_eq!(fixture.get("/api/courses").await.status(), StatusCode::OK);
}

#[tokio::test]
async fn course_401_with_confirmed_expired_identity_logs_out() {
    let fixture = Fixture::new(Router::new().route(
        "/api/v1/users/self/profile",
        get(|| async { StatusCode::UNAUTHORIZED }),
    ))
    .await;
    assert_eq!(
        fixture.get("/api/courses/94198/files").await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(
        !fixture
            .state
            .sessions
            .view(&fixture.session_id)
            .await
            .authenticated
    );
}

#[tokio::test]
async fn course_401_with_different_identity_logs_out() {
    let fixture = Fixture::new(Router::new().route(
        "/api/v1/users/self/profile",
        get(|| async {
            let mut changed = profile();
            changed.id = "43".into();
            Json(changed)
        }),
    ))
    .await;
    assert_eq!(
        fixture.get("/api/courses/94198/files").await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(
        !fixture
            .state
            .sessions
            .view(&fixture.session_id)
            .await
            .authenticated
    );
}

#[tokio::test]
async fn temporary_identity_outage_does_not_clear_login() {
    let fixture = Fixture::new(Router::new().route(
        "/api/v1/users/self/profile",
        get(|| async { StatusCode::SERVICE_UNAVAILABLE }),
    ))
    .await;
    let response = fixture.get("/api/courses/94198/files").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[header::RETRY_AFTER], "5");
    assert!(
        fixture
            .state
            .sessions
            .view(&fixture.session_id)
            .await
            .authenticated
    );
}

#[tokio::test]
async fn maintenance_html_does_not_clear_login() {
    let fixture = Fixture::new(Router::new().route(
        "/api/v1/users/self/profile",
        get(|| async { Html("<h1>Maintenance</h1>") }),
    ))
    .await;
    assert_eq!(
        fixture.get("/api/courses/94198/files").await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        json_body(fixture.get("/api/session").await).await["authenticated"],
        true
    );
}

#[tokio::test]
async fn login_redirect_confirms_expired_identity() {
    let fixture = Fixture::new(
        Router::new()
            .route(
                "/api/v1/users/self/profile",
                get(|| async { axum::response::Redirect::temporary("/login") }),
            )
            .route("/login", get(|| async { Html("<h1>Login</h1>") })),
    )
    .await;
    assert_eq!(
        fixture.get("/api/courses/94198/files").await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(
        !fixture
            .state
            .sessions
            .view(&fixture.session_id)
            .await
            .authenticated
    );
}

#[tokio::test]
async fn late_identity_failure_cannot_invalidate_a_replacement_login() {
    let started = Arc::new(Semaphore::new(0));
    let proceed = Arc::new(Semaphore::new(0));
    let fixture = Fixture::new(Router::new().route(
        "/api/v1/users/self/profile",
        get({
            let started = started.clone();
            let proceed = proceed.clone();
            move || {
                let started = started.clone();
                let proceed = proceed.clone();
                async move {
                    started.add_permits(1);
                    proceed.acquire().await.unwrap().forget();
                    StatusCode::UNAUTHORIZED
                }
            }
        }),
    ))
    .await;
    let pending = tokio::spawn(
        fixture
            .app
            .clone()
            .oneshot(fixture.request("/api/courses/94198/files")),
    );
    tokio::time::timeout(Duration::from_secs(5), started.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    fixture
        .state
        .sessions
        .complete_login(&fixture.session_id, Arc::new(Jar::default()), profile())
        .await
        .unwrap();
    proceed.add_permits(1);
    assert_eq!(
        pending.await.unwrap().unwrap().status(),
        StatusCode::CONFLICT
    );
    assert!(
        fixture
            .state
            .sessions
            .view(&fixture.session_id)
            .await
            .authenticated
    );
}
