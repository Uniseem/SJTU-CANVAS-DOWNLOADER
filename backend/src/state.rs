use std::sync::Arc;

use anyhow::Result;

use crate::{
    auth::AuthManager, config::Config, downloads::DownloadService, session::SessionStore,
    video::VideoService,
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub sessions: Arc<SessionStore>,
    pub auth: Arc<AuthManager>,
    pub videos: Arc<VideoService>,
    pub downloads: Arc<DownloadService>,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self> {
        let config = Arc::new(config);
        let sessions = Arc::new(SessionStore::open(&config).await?);
        let videos = Arc::new(VideoService::new(config.clone()));
        let auth = Arc::new(AuthManager::new(config.clone(), sessions.clone()));
        let downloads = Arc::new(DownloadService::new(
            config.clone(),
            sessions.clone(),
            videos.clone(),
        ));
        Ok(Self {
            config,
            sessions,
            auth,
            videos,
            downloads,
        })
    }
}
