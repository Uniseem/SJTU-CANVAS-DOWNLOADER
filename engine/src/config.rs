//! Command line, data folders and the SJTU endpoints the engine talks to.

use std::{
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};

/// Name of the app's data folder.
pub const APP_NAME: &str = "SJTU Canvas Downloader";
/// Folder created inside the user's Downloads folder by default.
pub const DOWNLOAD_FOLDER_NAME: &str = "SJTU Canvas";

/// Command line accepted from the host application.
#[derive(Debug, Default)]
pub struct Args {
    pub data_dir: Option<PathBuf>,
    pub version: bool,
}

impl Args {
    pub fn parse() -> Result<Self> {
        let mut args = Self::default();
        let mut iter = env::args_os().skip(1);
        while let Some(arg) = iter.next() {
            match arg.to_str() {
                Some("--data-dir") => {
                    args.data_dir = Some(iter.next().context("--data-dir 缺少路径")?.into());
                }
                Some("--version") => args.version = true,
                _ => anyhow::bail!("未知参数：{}", arg.to_string_lossy()),
            }
        }
        Ok(args)
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub data_root: PathBuf,
    pub log_root: PathBuf,
    pub database_path: PathBuf,
    /// The encrypted Canvas login (cookies and profile).
    pub session_path: PathBuf,
    pub canvas_origin: String,
    pub courses_origin: String,
    pub video_api: String,
    pub video_lti_adapter: String,
    pub resource_video_api: String,
    pub jaccount_origin: String,
    /// Development aid honoured by debug builds only (SJTU_CANVAS_TEST_MODE=1):
    /// the school endpoints may be overridden and loopback test servers are
    /// accepted as upstreams. Release builds ignore it.
    pub test_mode: bool,
    /// Demo courses, lessons and files instead of the school, for UI tests
    /// (test mode plus SJTU_CANVAS_FAKE_SCHOOL=1). Media then comes from
    /// `fake_media` (SJTU_CANVAS_FAKE_MEDIA, see engine/tests/mock_media.py).
    pub fake_school: bool,
    pub fake_media: Option<String>,
    /// How long the demo login shows its QR code (SJTU_CANVAS_FAKE_LOGIN_SECONDS).
    pub fake_login_delay: Duration,
}

impl Config {
    pub fn new(args: &Args) -> Result<Self> {
        let data_root = match &args.data_dir {
            Some(path) => absolute(path)?,
            None => default_data_dir()?,
        };
        let test_mode = cfg!(debug_assertions) && env_flag("SJTU_CANVAS_TEST_MODE");
        let endpoint = |name: &str, default: &str| {
            test_mode
                .then(|| env::var(name).ok())
                .flatten()
                .filter(|value| !value.trim().is_empty())
                .map(|value| value.trim().trim_end_matches('/').to_string())
                .unwrap_or_else(|| default.to_string())
        };
        let mut config = Self::school(data_root);
        config.canvas_origin = endpoint("SJTU_CANVAS_CANVAS_ORIGIN", &config.canvas_origin);
        config.courses_origin = endpoint("SJTU_CANVAS_COURSES_ORIGIN", &config.courses_origin);
        config.video_api = endpoint("SJTU_CANVAS_VIDEO_API", &config.video_api);
        config.video_lti_adapter =
            endpoint("SJTU_CANVAS_VIDEO_LTI_ADAPTER", &config.video_lti_adapter);
        config.resource_video_api =
            endpoint("SJTU_CANVAS_RESOURCE_VIDEO_API", &config.resource_video_api);
        config.jaccount_origin = endpoint("SJTU_CANVAS_JACCOUNT_ORIGIN", &config.jaccount_origin);
        config.test_mode = test_mode;
        config.fake_school = test_mode && env_flag("SJTU_CANVAS_FAKE_SCHOOL");
        config.fake_media = test_mode
            .then(|| env::var("SJTU_CANVAS_FAKE_MEDIA").ok())
            .flatten()
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty());
        if let Some(seconds) = env::var("SJTU_CANVAS_FAKE_LOGIN_SECONDS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
        {
            config.fake_login_delay = Duration::from_secs(seconds);
        }
        Ok(config)
    }

    /// The real SJTU endpoints with the data folders under `data_root`.
    pub fn school(data_root: PathBuf) -> Self {
        Self {
            log_root: data_root.join("logs"),
            database_path: data_root.join("downloads.db"),
            session_path: data_root.join("session.bin"),
            data_root,
            canvas_origin: "https://oc.sjtu.edu.cn".into(),
            courses_origin: "https://courses.sjtu.edu.cn".into(),
            video_api: "https://v.sjtu.edu.cn/jy-application-canvas-sjtu".into(),
            video_lti_adapter: "https://v.sjtu.edu.cn/jy-lti-adapter".into(),
            resource_video_api: "https://v.sjtu.edu.cn/jy-application-resourcemanage".into(),
            jaccount_origin: "https://jaccount.sjtu.edu.cn".into(),
            test_mode: false,
            fake_school: false,
            fake_media: None,
            fake_login_delay: Duration::from_secs(4),
        }
    }

    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self::school(std::env::temp_dir().join(format!(
            "sjtu-canvas-test-{}",
            uuid::Uuid::new_v4().simple()
        )))
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

    /// Loopback servers a debug build may talk to in test mode.
    pub fn test_origins(&self) -> Vec<String> {
        if !self.test_mode {
            return Vec::new();
        }
        let mut origins = [
            &self.canvas_origin,
            &self.courses_origin,
            &self.video_api,
            &self.video_lti_adapter,
            &self.resource_video_api,
            &self.jaccount_origin,
        ]
        .into_iter()
        .cloned()
        .chain(self.fake_media.clone())
        .collect::<Vec<_>>();
        if let Ok(extra) = env::var("SJTU_CANVAS_TEST_ORIGINS") {
            origins.extend(extra.split(',').map(str::trim).map(str::to_string));
        }
        origins
    }
}

fn env_flag(name: &str) -> bool {
    env::var(name).is_ok_and(|value| value.trim() == "1")
}

fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

fn default_data_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    let base = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
    });
    #[cfg(not(any(windows, target_os = "macos")))]
    let base = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    Ok(base
        .context("无法确定默认数据目录，请传入 --data-dir")?
        .join(APP_NAME))
}

/// `<Downloads>/SJTU Canvas`, used when the host does not name a folder.
pub fn fallback_download_dir(downloads_folder: Option<&Path>) -> PathBuf {
    let downloads = downloads_folder.map(Path::to_path_buf).or_else(|| {
        #[cfg(windows)]
        let home = env::var_os("USERPROFILE");
        #[cfg(not(windows))]
        let home = env::var_os("HOME");
        home.map(|home| PathBuf::from(home).join("Downloads"))
    });
    downloads
        .unwrap_or_else(|| PathBuf::from("."))
        .join(DOWNLOAD_FOLDER_NAME)
}
