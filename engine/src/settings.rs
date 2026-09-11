//! Preferences stored in the engine's database, shared by both apps.

use std::path::Path;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::http::ProxySettings;

const PREFERENCES: &str = "preferences_v1";

pub const DEFAULT_CONCURRENCY: usize = 3;
pub const MAX_CONCURRENCY: usize = 8;
/// Video tracks the user can download: 电脑屏幕, 教室摄像头, 合成画面.
pub const TRACKS: [&str; 3] = ["slides", "teacher", "composite"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    /// Where new downloads go (course folders are created inside it).
    pub download_dir: String,
    /// Ask for a folder before every download instead of using `download_dir`.
    pub ask_destination: bool,
    /// Files downloaded at the same time.
    pub concurrency: usize,
    /// Tracks preselected for lessons, a non-empty subset of `TRACKS`.
    pub default_tracks: Vec<String>,
    pub proxy: ProxySettings,
}

impl Preferences {
    pub fn defaults(download_dir: &Path) -> Self {
        Self {
            download_dir: download_dir.to_string_lossy().into_owned(),
            ask_destination: false,
            concurrency: DEFAULT_CONCURRENCY,
            default_tracks: vec!["slides".into(), "teacher".into()],
            proxy: ProxySettings::System,
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.download_dir.trim().is_empty()
                && Path::new(self.download_dir.trim()).is_absolute(),
            "保存位置必须是完整的文件夹路径"
        );
        ensure!(
            (1..=MAX_CONCURRENCY).contains(&self.concurrency),
            "同时下载数必须在 1–{MAX_CONCURRENCY} 之间"
        );
        ensure!(!self.default_tracks.is_empty(), "至少选择一个默认画面");
        for track in &self.default_tracks {
            ensure!(TRACKS.contains(&track.as_str()), "未知的视频画面：{track}");
        }
        self.proxy.validate()
    }

    fn normalized(mut self) -> Self {
        self.download_dir = self.download_dir.trim().to_string();
        let mut tracks = Vec::new();
        for track in TRACKS {
            if self.default_tracks.iter().any(|value| value == track) {
                tracks.push(track.to_string());
            }
        }
        self.default_tracks = tracks;
        self
    }
}

/// The saved preferences; an empty or invalid saved value means the defaults.
pub async fn load(pool: &SqlitePool, default_dir: &Path) -> Result<Preferences> {
    let defaults = Preferences::defaults(default_dir);
    let saved = sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key = $1")
        .bind(PREFERENCES)
        .fetch_optional(pool)
        .await?;
    let Some(saved) = saved else {
        return Ok(defaults);
    };
    let Ok(mut preferences) = serde_json::from_str::<Preferences>(&saved) else {
        return Ok(defaults);
    };
    // The folder is stored only when the user chose one; otherwise it follows
    // the platform's Downloads folder.
    if preferences.download_dir.trim().is_empty() {
        preferences.download_dir = defaults.download_dir.clone();
    }
    let preferences = preferences.normalized();
    Ok(if preferences.validate().is_ok() {
        preferences
    } else {
        defaults
    })
}

pub async fn save(
    pool: &SqlitePool,
    preferences: &Preferences,
    default_dir: &Path,
) -> Result<Preferences> {
    let preferences = preferences.clone().normalized();
    preferences.validate()?;
    let mut stored = preferences.clone();
    if Path::new(&stored.download_dir) == default_dir {
        stored.download_dir = String::new();
    }
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at) VALUES ($1, $2, $3) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(PREFERENCES)
    .bind(serde_json::to_string(&stored)?)
    .bind(crate::db::now())
    .execute(pool)
    .await?;
    Ok(preferences)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn folder(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[tokio::test]
    async fn preferences_persist_and_follow_the_default_folder() {
        let pool = crate::db::open_memory().await;
        let default_dir = folder("Downloads-A");
        assert_eq!(
            load(&pool, &default_dir).await.unwrap(),
            Preferences::defaults(&default_dir)
        );

        let mut changed = Preferences::defaults(&default_dir);
        changed.concurrency = 5;
        changed.default_tracks = vec!["composite".into(), "slides".into()];
        changed.proxy = ProxySettings::Direct;
        let saved = save(&pool, &changed, &default_dir).await.unwrap();
        // Tracks are kept in canonical order.
        assert_eq!(saved.default_tracks, vec!["slides", "composite"]);
        // The default folder is not pinned: it moves with the Downloads folder.
        let moved = folder("Downloads-B");
        assert_eq!(
            load(&pool, &moved).await.unwrap().download_dir,
            moved.to_string_lossy()
        );

        let custom = folder("Canvas-Custom");
        changed.download_dir = custom.to_string_lossy().into_owned();
        save(&pool, &changed, &default_dir).await.unwrap();
        assert_eq!(
            load(&pool, &moved).await.unwrap().download_dir,
            custom.to_string_lossy()
        );
    }

    #[test]
    fn invalid_preferences_are_rejected() {
        let base = Preferences::defaults(&folder("Downloads"));
        let mut relative = base.clone();
        relative.download_dir = "relative/path".into();
        assert!(relative.validate().is_err());
        let mut none = base.clone();
        none.default_tracks.clear();
        assert!(none.validate().is_err());
        let mut unknown = base.clone();
        unknown.default_tracks = vec!["ppt".into()];
        assert!(unknown.validate().is_err());
        let mut too_many = base.clone();
        too_many.concurrency = MAX_CONCURRENCY + 1;
        assert!(too_many.validate().is_err());
        base.validate().unwrap();
    }
}
