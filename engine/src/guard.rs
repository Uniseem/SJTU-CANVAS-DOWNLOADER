//! Canvas answers 401 both when the login expired and when one resource is
//! not open to this account. Only the identity endpoint tells them apart, so
//! a 401 is confirmed there before the app is told to log in again.

use std::{sync::Arc, time::Duration};

use reqwest::cookie::Jar;

use crate::{
    canvas::CanvasClient,
    error::{AppError, AppResult},
    models::CanvasProfile,
    state::AppState,
};

/// Runs `call` with the current login and confirms any 401 it returns.
pub async fn with_login<T, F, Fut>(state: &AppState, call: F) -> AppResult<T>
where
    F: FnOnce(Arc<Jar>, CanvasProfile) -> Fut,
    Fut: std::future::Future<Output = AppResult<T>>,
{
    let (jar, profile) = state
        .account
        .current()
        .await
        .ok_or(AppError::Unauthorized)?;
    match call(jar.clone(), profile.clone()).await {
        Err(AppError::Unauthorized) => Err(confirm_unauthorized(state, &jar, &profile.id).await),
        other => other,
    }
}

/// The error to report for a 401 seen with `jar`: a permission problem while
/// the login is valid, a real logout, or a temporary failure to find out.
pub async fn confirm_unauthorized(state: &AppState, jar: &Arc<Jar>, profile_id: &str) -> AppError {
    let check = async {
        CanvasClient::new(state.config.clone(), jar.clone())?
            .profile()
            .await
    };
    let identity = tokio::time::timeout(Duration::from_secs(8), check).await;
    // A new QR login may have replaced the jar while this request was in
    // flight; its result must not end the newer login.
    if !state.account.is_current(jar).await {
        return AppError::Conflict("登录状态已更新，请重试当前操作".into());
    }
    match identity {
        Ok(Ok(profile)) if profile.id == profile_id => {
            tracing::warn!("resource denied but Canvas identity is valid; keeping login");
            AppError::Forbidden(
                "当前账号无权访问此资源，课程可能尚未开放或未启用该功能；登录仍然有效".into(),
            )
        }
        Ok(Ok(_)) | Ok(Err(AppError::Unauthorized)) => {
            match state.account.logout_if_current(jar).await {
                Ok(true) => {
                    tracing::info!("Canvas identity no longer valid; login expired");
                    state.after_logout(Some(profile_id)).await;
                    AppError::Unauthorized
                }
                Ok(false) => AppError::Conflict("登录状态已更新，请重试当前操作".into()),
                Err(error) => AppError::internal(error),
            }
        }
        Ok(Err(_)) | Err(_) => {
            tracing::warn!("Canvas identity check unavailable; keeping login");
            AppError::upstream_unavailable("暂时无法核验 Canvas 登录状态，请稍后重试", 5)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        http::StatusCode,
        response::{Html, Redirect},
        routing::get,
    };
    use serde_json::json;
    use tokio::{net::TcpListener, sync::Semaphore, task::JoinHandle};

    struct Fixture {
        state: Arc<AppState>,
        upstream: JoinHandle<()>,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.upstream.abort();
        }
    }

    fn profile() -> CanvasProfile {
        CanvasProfile {
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
            let mut config = crate::config::Config::for_tests();
            config.canvas_origin = format!("http://{address}");
            let pool = crate::db::open_memory().await;
            let state = AppState::new(Arc::new(config), pool);
            state
                .account
                .complete_login(Arc::new(Jar::default()), profile())
                .await
                .unwrap();
            Self { state, upstream }
        }

        async fn files(&self) -> AppResult<Vec<crate::models::CanvasFile>> {
            let config = self.state.config.clone();
            with_login(&self.state, |jar, _| async move {
                CanvasClient::new(config, jar)?.files("94198").await
            })
            .await
        }

        async fn courses(&self) -> AppResult<Vec<crate::models::Course>> {
            let config = self.state.config.clone();
            with_login(&self.state, |jar, _| async move {
                CanvasClient::new(config, jar)?.courses().await
            })
            .await
        }

        async fn authenticated(&self) -> bool {
            self.state.account.view().await.authenticated
        }
    }

    fn identity(handler: axum::routing::MethodRouter) -> Router {
        Router::new().route("/api/v1/users/self/profile", handler)
    }

    #[tokio::test]
    async fn course_401_after_login_keeps_login_and_other_courses_usable() {
        let fixture = Fixture::new(identity(get(|| async { Json(profile()) }))).await;
        assert!(matches!(fixture.files().await, Err(AppError::Forbidden(_))));
        assert!(fixture.authenticated().await);
        assert!(fixture.courses().await.is_ok());
    }

    #[tokio::test]
    async fn course_401_with_confirmed_expired_identity_logs_out() {
        let fixture = Fixture::new(identity(get(|| async { StatusCode::UNAUTHORIZED }))).await;
        assert!(matches!(fixture.files().await, Err(AppError::Unauthorized)));
        assert!(!fixture.authenticated().await);
    }

    #[tokio::test]
    async fn course_401_with_different_identity_logs_out() {
        let fixture = Fixture::new(identity(get(|| async {
            let mut changed = profile();
            changed.id = "43".into();
            Json(changed)
        })))
        .await;
        assert!(matches!(fixture.files().await, Err(AppError::Unauthorized)));
        assert!(!fixture.authenticated().await);
    }

    #[tokio::test]
    async fn temporary_identity_outage_does_not_clear_login() {
        let fixture =
            Fixture::new(identity(get(|| async { StatusCode::SERVICE_UNAVAILABLE }))).await;
        let error = fixture.files().await.unwrap_err();
        assert_eq!(error.retry_after(), Some(5));
        assert!(fixture.authenticated().await);
    }

    #[tokio::test]
    async fn maintenance_html_does_not_clear_login() {
        let fixture = Fixture::new(identity(get(|| async { Html("<h1>Maintenance</h1>") }))).await;
        assert!(matches!(
            fixture.files().await,
            Err(AppError::UpstreamUnavailable { .. })
        ));
        assert!(fixture.authenticated().await);
    }

    #[tokio::test]
    async fn login_redirect_confirms_expired_identity() {
        let fixture = Fixture::new(
            identity(get(|| async { Redirect::temporary("/login") }))
                .route("/login", get(|| async { Html("<h1>Login</h1>") })),
        )
        .await;
        assert!(matches!(fixture.files().await, Err(AppError::Unauthorized)));
        assert!(!fixture.authenticated().await);
    }

    #[tokio::test]
    async fn late_identity_failure_cannot_invalidate_a_replacement_login() {
        let started = Arc::new(Semaphore::new(0));
        let proceed = Arc::new(Semaphore::new(0));
        let fixture = Fixture::new(identity(get({
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
        })))
        .await;
        let state = fixture.state.clone();
        let config = state.config.clone();
        let pending = tokio::spawn(async move {
            with_login(&state, |jar, _| async move {
                CanvasClient::new(config, jar)?.files("94198").await
            })
            .await
        });
        tokio::time::timeout(Duration::from_secs(5), started.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        fixture
            .state
            .account
            .complete_login(Arc::new(Jar::default()), profile())
            .await
            .unwrap();
        proceed.add_permits(1);
        assert!(matches!(pending.await.unwrap(), Err(AppError::Conflict(_))));
        assert!(fixture.authenticated().await);
    }
}
