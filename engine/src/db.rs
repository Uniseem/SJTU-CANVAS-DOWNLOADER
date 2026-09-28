use std::{path::Path, str::FromStr, time::Duration};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

/// Sortable UTC timestamp text, e.g. `2026-09-11T08:30:12.123Z`.
pub fn timestamp(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn now() -> String {
    timestamp(Utc::now())
}

pub async fn open(path: &Path) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str("sqlite://")?
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(15));
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .with_context(|| format!("无法打开下载记录数据库 {}", path.display()))?;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("下载记录数据库迁移失败")?;
    Ok(pool)
}

#[cfg(test)]
pub async fn open_memory() -> SqlitePool {
    let options = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
    // A single connection keeps the in-memory database alive and shared.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_sort_as_text() {
        let earlier = timestamp(
            DateTime::parse_from_rfc3339("2026-09-11T08:00:00Z")
                .unwrap()
                .into(),
        );
        let later = timestamp(
            DateTime::parse_from_rfc3339("2026-09-11T08:00:00.5Z")
                .unwrap()
                .into(),
        );
        assert_eq!(earlier, "2026-09-11T08:00:00.000Z");
        assert!(earlier < later);
    }
}
