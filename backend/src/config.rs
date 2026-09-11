use std::{env, net::SocketAddr, path::PathBuf, str::FromStr, time::Duration};

use anyhow::{Context, Result};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub web_dist: PathBuf,
    pub public_url: Option<String>,
    pub app_secret: Option<String>,
    pub cookie_secure: bool,
    pub session_ttl: Duration,
    pub ticket_ttl: Duration,
    pub canvas_origin: String,
    pub courses_origin: String,
    pub video_api: String,
    pub video_lti_adapter: String,
    pub resource_video_api: String,
    pub jaccount_origin: String,
    pub demo_mode: bool,
    pub proxy_concurrency: usize,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bind = env_string("BIND", "0.0.0.0:8080")
            .parse()
            .context("BIND 必须是 host:port")?;
        let public_url = env::var("PUBLIC_URL")
            .ok()
            .map(|value| value.trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty());
        let cookie_secure = env_bool(
            "COOKIE_SECURE",
            public_url
                .as_deref()
                .is_some_and(|value| value.starts_with("https://")),
        );

        Ok(Self {
            bind,
            data_dir: PathBuf::from(env_string("DATA_DIR", "./data")),
            web_dist: PathBuf::from(env_string("WEB_DIST", "./frontend/dist")),
            public_url,
            app_secret: env::var("APP_SECRET")
                .ok()
                .filter(|value| !value.is_empty()),
            cookie_secure,
            session_ttl: Duration::from_secs(env_u64("SESSION_TTL_DAYS", 7) * 86_400),
            ticket_ttl: Duration::from_secs(env_u64("DOWNLOAD_TICKET_TTL_MINUTES", 15) * 60),
            canvas_origin: env_string("CANVAS_ORIGIN", "https://oc.sjtu.edu.cn"),
            courses_origin: env_string("COURSES_ORIGIN", "https://courses.sjtu.edu.cn"),
            video_api: env_string(
                "VIDEO_API",
                "https://v.sjtu.edu.cn/jy-application-canvas-sjtu",
            ),
            video_lti_adapter: env_string(
                "VIDEO_LTI_ADAPTER",
                "https://v.sjtu.edu.cn/jy-lti-adapter",
            ),
            resource_video_api: env_string(
                "RESOURCE_VIDEO_API",
                "https://v.sjtu.edu.cn/jy-application-resourcemanage",
            ),
            jaccount_origin: env_string("JACCOUNT_ORIGIN", "https://jaccount.sjtu.edu.cn"),
            demo_mode: env_bool("DEMO_MODE", false),
            proxy_concurrency: env_usize("PROXY_CONCURRENCY", 16).clamp(1, 128),
        })
    }

    pub fn login_init_url(&self) -> String {
        format!(
            "{}/app/oauth/2.0/login?login_type=outer",
            self.courses_origin
        )
    }

    pub fn canvas_login_url(&self) -> String {
        format!("{}/login/openid_connect", self.canvas_origin)
    }

    pub fn cookie_header(&self, max_age: Duration) -> String {
        let secure = if self.cookie_secure { "; Secure" } else { "" };
        format!(
            "{}={{sid}}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
            self.session_cookie_name(),
            max_age.as_secs(),
            secure
        )
    }

    pub fn session_cookie_name(&self) -> &'static str {
        if self.cookie_secure {
            "__Host-canvas_sid"
        } else {
            "canvas_sid"
        }
    }
}

fn env_string(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_bool(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .and_then(|value| bool::from_str(value.trim()).ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
