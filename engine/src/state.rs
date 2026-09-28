use std::{
    path::PathBuf,
    sync::{Arc, RwLock, atomic::AtomicBool},
};

use sqlx::SqlitePool;

use crate::{
    account::Account,
    auth::AuthManager,
    config::{self, Config},
    downloads::DownloadService,
    events,
    settings::Preferences,
    video::VideoService,
};

pub struct AppState {
    pub config: Arc<Config>,
    pub pool: SqlitePool,
    pub account: Arc<Account>,
    pub auth: Arc<AuthManager>,
    pub videos: Arc<VideoService>,
    pub downloads: Arc<DownloadService>,
    preferences: RwLock<Option<Preferences>>,
    default_download_dir: RwLock<PathBuf>,
    pub scheduler_started: AtomicBool,
}

impl AppState {
    pub fn new(config: Arc<Config>, pool: SqlitePool) -> Arc<Self> {
        let account = Arc::new(Account::new(&config));
        let downloads = Arc::new(DownloadService::new(pool.clone()));
        let scheduler = downloads.clone();
        // A new login lets queued downloads continue.
        let auth = Arc::new(AuthManager::new(
            config.clone(),
            account.clone(),
            Box::new(move || scheduler.wake()),
        ));
        Arc::new(Self {
            videos: Arc::new(VideoService::new(config.clone())),
            default_download_dir: RwLock::new(config::fallback_download_dir(None)),
            config,
            pool,
            account,
            auth,
            downloads,
            preferences: RwLock::new(None),
            scheduler_started: AtomicBool::new(false),
        })
    }

    pub fn preferences(&self) -> Preferences {
        self.preferences
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .unwrap_or_else(|| Preferences::defaults(&self.default_download_dir()))
    }

    pub fn set_preferences(&self, preferences: Preferences) {
        crate::http::set_proxy(preferences.proxy.clone());
        self.downloads.set_concurrency(preferences.concurrency);
        *self
            .preferences
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Some(preferences);
    }

    pub fn default_download_dir(&self) -> PathBuf {
        self.default_download_dir
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn set_default_download_dir(&self, path: PathBuf) {
        *self
            .default_download_dir
            .write()
            .unwrap_or_else(|error| error.into_inner()) = path;
    }

    /// After the login ended: forget the account's cached school tokens, hold
    /// its transfers until the next login, and tell the app.
    pub async fn after_logout(&self, profile_id: Option<&str>) {
        if let Some(id) = profile_id {
            self.videos.clear_owner(id);
        }
        self.downloads
            .hold_running("登录已失效，重新登录后会自动继续")
            .await;
        events::notify("account.changed", self.account.view().await);
    }
}
