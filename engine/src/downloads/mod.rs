//! Download manager. Tasks are rows in SQLite; the scheduler starts up to
//! `concurrency` transfers for the logged-in account, and every transfer
//! resolves a fresh media URL right before it starts, so signed URLs never
//! expire in a long queue. Files are written as `<name>.part` and renamed
//! when complete; pausing keeps the partial file for a Range resume.

pub mod naming;
mod transfer;

pub use transfer::Media;

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use tokio::{
    sync::Notify,
    task::{Id, JoinSet},
};
use tokio_util::sync::CancellationToken;

use crate::{
    canvas::validate_canvas_id,
    db,
    error::{AppError, AppResult},
    events,
    settings::{DEFAULT_CONCURRENCY, TRACKS},
    state::AppState,
    video::validate_resource_id,
};

pub const MAX_ITEMS_PER_REQUEST: usize = 500;
const UNFINISHED: &str = "('queued', 'downloading', 'paused')";

#[derive(Debug, Clone, FromRow)]
pub struct DownloadRow {
    pub id: String,
    pub owner_id: String,
    pub kind: String,
    pub course_id: String,
    pub course_name: String,
    pub resource_id: String,
    pub track: Option<String>,
    pub title: String,
    pub begin_time: Option<String>,
    pub destination: String,
    pub file_path: Option<String>,
    pub status: String,
    pub received: i64,
    pub total: Option<i64>,
    pub etag: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

/// A task as the apps see it (the `download.changed` notification).
#[derive(Debug, Clone, Serialize)]
pub struct DownloadInfo {
    pub id: String,
    /// video | file
    pub kind: String,
    pub course_id: String,
    pub course_name: String,
    pub resource_id: String,
    pub track: Option<String>,
    pub title: String,
    /// "第74讲 · 电脑屏幕" for videos, the file name for course files.
    pub display_name: String,
    pub begin_time: Option<String>,
    pub destination: String,
    /// Final location once known (the file is `<file_path>.part` until done).
    pub file_path: Option<String>,
    /// queued | downloading | paused | completed | failed | cancelled
    pub status: String,
    pub received: i64,
    pub total: Option<i64>,
    /// Bytes per second while downloading.
    pub speed: f64,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Live {
    received: i64,
    total: Option<i64>,
    speed: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NewDownload {
    Video {
        course_id: String,
        course_name: String,
        lesson_id: String,
        title: String,
        #[serde(default)]
        begin_time: Option<String>,
        track: String,
        /// Known size, shown until the transfer reports one.
        #[serde(default)]
        size: Option<i64>,
    },
    File {
        course_id: String,
        course_name: String,
        file_id: String,
        title: String,
        #[serde(default)]
        size: Option<i64>,
    },
}

#[derive(Debug, Serialize)]
pub struct CreateResult {
    pub created: Vec<DownloadInfo>,
    pub skipped: Vec<Skipped>,
}

#[derive(Debug, Serialize)]
pub struct Skipped {
    pub index: usize,
    /// invalid | queued | downloaded
    pub reason: &'static str,
    pub message: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListInput {
    /// all | active | completed | failed
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Default, Serialize, FromRow)]
pub struct Counts {
    pub all: i64,
    /// Not finished: queued, downloading or paused.
    pub active: i64,
    /// Queued or downloading.
    pub running: i64,
    pub completed: i64,
    /// Failed or cancelled.
    pub failed: i64,
}

#[derive(Debug, Serialize)]
pub struct DownloadList {
    pub items: Vec<DownloadInfo>,
    pub counts: Counts,
}

pub struct DownloadService {
    pool: SqlitePool,
    running: Mutex<HashMap<String, CancellationToken>>,
    live: Mutex<HashMap<String, Live>>,
    wake: Notify,
    concurrency: AtomicUsize,
    /// Set when the engine shuts down: no new transfer starts.
    stopping: AtomicBool,
}

fn internal(error: impl Into<anyhow::Error>) -> AppError {
    AppError::internal(error)
}

impl DownloadService {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            running: Mutex::new(HashMap::new()),
            live: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            concurrency: AtomicUsize::new(DEFAULT_CONCURRENCY),
            stopping: AtomicBool::new(false),
        }
    }

    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub fn concurrency(&self) -> usize {
        self.concurrency.load(Ordering::Relaxed).max(1)
    }

    pub fn set_concurrency(&self, value: usize) {
        self.concurrency.store(value.max(1), Ordering::Relaxed);
        self.wake();
    }

    /// Transfers cut off by the last shutdown continue where they stopped.
    pub async fn reset_interrupted(&self) -> AppResult<()> {
        sqlx::query(
            "UPDATE downloads SET status = 'queued', updated_at = $1 WHERE status = 'downloading'",
        )
        .bind(db::now())
        .execute(&self.pool)
        .await
        .map_err(internal)?;
        Ok(())
    }

    pub async fn create(
        &self,
        owner_id: &str,
        destination: &Path,
        items: Vec<NewDownload>,
    ) -> AppResult<CreateResult> {
        if items.is_empty() {
            return Err(AppError::BadRequest("请至少选择一个下载项目".into()));
        }
        if items.len() > MAX_ITEMS_PER_REQUEST {
            return Err(AppError::BadRequest(format!(
                "一次最多添加 {MAX_ITEMS_PER_REQUEST} 个下载任务"
            )));
        }
        let destination_text = destination.to_string_lossy().into_owned();
        let queued = sqlx::query_as::<_, (String, String, String, Option<String>)>(&format!(
            "SELECT kind, course_id, resource_id, track FROM downloads \
             WHERE destination = $1 AND status IN {UNFINISHED}"
        ))
        .bind(&destination_text)
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?;
        let mut taken = queued.into_iter().collect::<HashSet<_>>();
        // Finished downloads whose file is still there are not repeated.
        let finished = sqlx::query_as::<_, (String, String, String, Option<String>, String)>(
            "SELECT kind, course_id, resource_id, track, file_path FROM downloads \
             WHERE destination = $1 AND status = 'completed' AND file_path IS NOT NULL",
        )
        .bind(&destination_text)
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|(kind, course, resource, track, path)| ((kind, course, resource, track), path))
        .collect::<HashMap<_, _>>();
        let created_at = db::now();
        let mut created = Vec::new();
        let mut skipped = Vec::new();
        for (index, item) in items.into_iter().enumerate() {
            let row = match new_row(owner_id, &destination_text, &created_at, item) {
                Ok(row) => row,
                Err(error) => {
                    skipped.push(Skipped {
                        index,
                        reason: "invalid",
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let key = (
                row.kind.clone(),
                row.course_id.clone(),
                row.resource_id.clone(),
                row.track.clone(),
            );
            if let Some(path) = finished.get(&key)
                && tokio::fs::try_exists(path).await.unwrap_or(false)
            {
                skipped.push(Skipped {
                    index,
                    reason: "downloaded",
                    message: format!("“{}”已经下载过", display_name(&row)),
                });
                continue;
            }
            if !taken.insert(key) {
                skipped.push(Skipped {
                    index,
                    reason: "queued",
                    message: format!("“{}”已在下载队列中", display_name(&row)),
                });
                continue;
            }
            sqlx::query(
                "INSERT INTO downloads (id, owner_id, kind, course_id, course_name, resource_id, track, title, \
                 begin_time, destination, status, received, total, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'queued', 0, $11, $12, $12)",
            )
            .bind(&row.id)
            .bind(&row.owner_id)
            .bind(&row.kind)
            .bind(&row.course_id)
            .bind(&row.course_name)
            .bind(&row.resource_id)
            .bind(&row.track)
            .bind(&row.title)
            .bind(&row.begin_time)
            .bind(&row.destination)
            .bind(row.total)
            .bind(&created_at)
            .execute(&self.pool)
            .await
            .map_err(internal)?;
            created.push(self.info(row));
        }
        for info in &created {
            events::notify("download.changed", info);
        }
        self.wake();
        Ok(CreateResult { created, skipped })
    }

    pub async fn list(&self, input: ListInput) -> AppResult<DownloadList> {
        let filter = match input.filter.as_deref().unwrap_or("all") {
            "all" => "1 = 1".to_string(),
            "active" => format!("status IN {UNFINISHED}"),
            "completed" => "status = 'completed'".to_string(),
            "failed" => "status IN ('failed', 'cancelled')".to_string(),
            other => return Err(AppError::BadRequest(format!("未知的筛选条件：{other}"))),
        };
        let query = input.query.unwrap_or_default().trim().to_string();
        let pattern = format!(
            "%{}%",
            query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let limit = input.limit.unwrap_or(2_000).clamp(1, 5_000);
        let rows = sqlx::query_as::<_, DownloadRow>(&format!(
            "SELECT * FROM downloads WHERE {filter} AND ($1 = '' OR title LIKE $2 ESCAPE '\\' \
             OR course_name LIKE $2 ESCAPE '\\' OR IFNULL(file_path, '') LIKE $2 ESCAPE '\\') \
             ORDER BY created_at DESC, rowid ASC LIMIT $3"
        ))
        .bind(&query)
        .bind(&pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?;
        Ok(DownloadList {
            items: rows.into_iter().map(|row| self.info(row)).collect(),
            counts: self.counts().await?,
        })
    }

    pub async fn counts(&self) -> AppResult<Counts> {
        sqlx::query_as::<_, Counts>(
            "SELECT COUNT(*) AS \"all\", \
             COALESCE(SUM(status IN ('queued', 'downloading', 'paused')), 0) AS active, \
             COALESCE(SUM(status IN ('queued', 'downloading')), 0) AS running, \
             COALESCE(SUM(status = 'completed'), 0) AS completed, \
             COALESCE(SUM(status IN ('failed', 'cancelled')), 0) AS failed FROM downloads",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(internal)
    }

    pub async fn get(&self, id: &str) -> AppResult<DownloadInfo> {
        Ok(self.info(self.row(id).await?))
    }

    async fn row(&self, id: &str) -> AppResult<DownloadRow> {
        sqlx::query_as::<_, DownloadRow>("SELECT * FROM downloads WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(internal)?
            .ok_or_else(|| AppError::NotFound("下载任务不存在".into()))
    }

    pub async fn pause(&self, id: &str) -> AppResult<DownloadInfo> {
        self.transition(id, "paused", "('queued', 'downloading')", None)
            .await?;
        self.stop_transfer(id);
        self.changed(id).await
    }

    pub async fn resume(&self, id: &str) -> AppResult<DownloadInfo> {
        self.transition(id, "queued", "('paused')", None).await?;
        self.wake();
        self.changed(id).await
    }

    pub async fn retry(&self, id: &str) -> AppResult<DownloadInfo> {
        self.transition(id, "queued", "('failed', 'cancelled')", None)
            .await?;
        self.wake();
        self.changed(id).await
    }

    /// Stops the task and deletes its partial file; the name is freed.
    pub async fn cancel(&self, id: &str) -> AppResult<DownloadInfo> {
        let row = self.row(id).await?;
        let updated = sqlx::query(
            "UPDATE downloads SET status = 'cancelled', error = NULL, file_path = NULL, received = 0, \
             updated_at = $2 WHERE id = $1 AND status IN ('queued', 'downloading', 'paused', 'failed')",
        )
        .bind(id)
        .bind(db::now())
        .execute(&self.pool)
        .await
        .map_err(internal)?;
        if updated.rows_affected() > 0 && !self.stop_transfer(id) {
            // Not running: nobody else holds the partial file.
            remove_part(row.file_path.as_deref()).await;
        }
        self.changed(id).await
    }

    /// Removes a finished task from the list. Downloaded files stay; the
    /// partial file of a failed task is deleted.
    pub async fn remove(&self, id: &str) -> AppResult<()> {
        let row = self.row(id).await?;
        if !matches!(row.status.as_str(), "completed" | "failed" | "cancelled") {
            return Err(AppError::Conflict("请先暂停或取消这个下载任务".into()));
        }
        sqlx::query("DELETE FROM downloads WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(internal)?;
        if row.status == "failed" {
            remove_part(row.file_path.as_deref()).await;
        }
        events::notify("download.removed", serde_json::json!({ "id": id }));
        Ok(())
    }

    pub async fn pause_all(&self) -> AppResult<usize> {
        let ids = self
            .bulk("UPDATE downloads SET status = 'paused', updated_at = $1 WHERE status IN ('queued', 'downloading') RETURNING id")
            .await?;
        for id in &ids {
            self.stop_transfer(id);
            let _ = self.changed(id).await;
        }
        Ok(ids.len())
    }

    pub async fn resume_all(&self) -> AppResult<usize> {
        let ids = self
            .bulk("UPDATE downloads SET status = 'queued', error = NULL, updated_at = $1 WHERE status = 'paused' RETURNING id")
            .await?;
        for id in &ids {
            let _ = self.changed(id).await;
        }
        self.wake();
        Ok(ids.len())
    }

    pub async fn clear_completed(&self) -> AppResult<usize> {
        let ids = sqlx::query_scalar::<_, String>(
            "DELETE FROM downloads WHERE status = 'completed' RETURNING id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?;
        for id in &ids {
            events::notify("download.removed", serde_json::json!({ "id": id }));
        }
        Ok(ids.len())
    }

    /// Puts running transfers back in the queue with a note, e.g. when the
    /// login ended; they continue after the next login.
    pub async fn hold_running(&self, note: &str) {
        let ids = sqlx::query_scalar::<_, String>(
            "UPDATE downloads SET status = 'queued', error = $2, updated_at = $1 \
             WHERE status = 'downloading' RETURNING id",
        )
        .bind(db::now())
        .bind(note)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        for id in &ids {
            self.stop_transfer(id);
            let _ = self.changed(id).await;
        }
    }

    /// Stops every transfer (engine shutdown). Their rows stay `downloading`
    /// and are queued again on the next start.
    pub async fn stop_all(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        let tokens = self
            .running
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for token in &tokens {
            token.cancel();
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline
            && !self
                .running
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn bulk(&self, statement: &str) -> AppResult<Vec<String>> {
        sqlx::query_scalar::<_, String>(statement)
            .bind(db::now())
            .fetch_all(&self.pool)
            .await
            .map_err(internal)
    }

    async fn transition(
        &self,
        id: &str,
        status: &str,
        from: &str,
        error: Option<&str>,
    ) -> AppResult<()> {
        let row = self.row(id).await?;
        let updated = sqlx::query(&format!(
            "UPDATE downloads SET status = $2, error = $3, updated_at = $4 WHERE id = $1 AND status IN {from}"
        ))
        .bind(id)
        .bind(status)
        .bind(error)
        .bind(db::now())
        .execute(&self.pool)
        .await
        .map_err(internal)?;
        if updated.rows_affected() == 0 && row.status != status {
            return Err(AppError::Conflict(format!(
                "任务当前{}，不能执行这个操作",
                status_label(&row.status)
            )));
        }
        Ok(())
    }

    /// Cancels a running transfer's token; false if it was not running.
    fn stop_transfer(&self, id: &str) -> bool {
        match self
            .running
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
        {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    fn is_running(&self, id: &str) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(id)
    }

    /// Sends `download.changed` with the current state of the task.
    async fn changed(&self, id: &str) -> AppResult<DownloadInfo> {
        let info = self.get(id).await?;
        events::notify("download.changed", &info);
        Ok(info)
    }

    /// The next queued task of `owner_id`, marked as downloading. A task whose
    /// previous transfer is still stopping is skipped until it has stopped.
    async fn claim(&self, owner_id: &str) -> AppResult<Option<DownloadRow>> {
        if self.stopping.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let candidates = sqlx::query_scalar::<_, String>(
            "SELECT id FROM downloads WHERE status = 'queued' AND owner_id = $1 \
             ORDER BY created_at, rowid LIMIT 64",
        )
        .bind(owner_id)
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?;
        for id in candidates {
            if self.is_running(&id) {
                continue;
            }
            let updated = sqlx::query(
                "UPDATE downloads SET status = 'downloading', error = NULL, updated_at = $2 \
                 WHERE id = $1 AND status = 'queued'",
            )
            .bind(&id)
            .bind(db::now())
            .execute(&self.pool)
            .await
            .map_err(internal)?;
            if updated.rows_affected() == 1 {
                return self.row(&id).await.map(Some);
            }
        }
        Ok(None)
    }

    fn info(&self, row: DownloadRow) -> DownloadInfo {
        let live = (row.status == "downloading")
            .then(|| {
                self.live
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .get(&row.id)
                    .copied()
            })
            .flatten();
        DownloadInfo {
            display_name: display_name(&row),
            received: live.map(|live| live.received).unwrap_or(row.received),
            total: live.and_then(|live| live.total).or(row.total),
            speed: live.map(|live| live.speed).unwrap_or(0.0),
            id: row.id,
            kind: row.kind,
            course_id: row.course_id,
            course_name: row.course_name,
            resource_id: row.resource_id,
            track: row.track,
            title: row.title,
            begin_time: row.begin_time,
            destination: row.destination,
            file_path: row.file_path,
            status: row.status,
            error: row.error,
            created_at: row.created_at,
            updated_at: row.updated_at,
            completed_at: row.completed_at,
        }
    }

    fn set_live(&self, id: &str, live: Live) {
        self.live
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id.to_string(), live);
    }
}

fn new_row(
    owner_id: &str,
    destination: &str,
    created_at: &str,
    item: NewDownload,
) -> AppResult<DownloadRow> {
    let (kind, course_id, course_name, resource_id, track, title, begin_time, size) = match item {
        NewDownload::Video {
            course_id,
            course_name,
            lesson_id,
            title,
            begin_time,
            track,
            size,
        } => {
            validate_resource_id(&lesson_id)?;
            if !TRACKS.contains(&track.as_str()) {
                return Err(AppError::BadRequest("未知的视频画面".into()));
            }
            (
                "video",
                course_id,
                course_name,
                lesson_id,
                Some(track),
                title,
                begin_time,
                size,
            )
        }
        NewDownload::File {
            course_id,
            course_name,
            file_id,
            title,
            size,
        } => {
            validate_canvas_id(&file_id)?;
            (
                "file",
                course_id,
                course_name,
                file_id,
                None,
                title,
                None,
                size,
            )
        }
    };
    validate_canvas_id(&course_id)?;
    let limit = |value: String, max: usize| value.trim().chars().take(max).collect::<String>();
    Ok(DownloadRow {
        id: crate::random_token(12),
        owner_id: owner_id.to_string(),
        kind: kind.to_string(),
        course_id,
        course_name: limit(course_name, 200),
        resource_id,
        track,
        title: limit(title, 300),
        begin_time: begin_time
            .map(|value| limit(value, 64))
            .filter(|value| !value.is_empty()),
        destination: destination.to_string(),
        file_path: None,
        status: "queued".into(),
        received: 0,
        total: size.filter(|size| *size > 0),
        etag: None,
        error: None,
        created_at: created_at.to_string(),
        updated_at: created_at.to_string(),
        completed_at: None,
    })
}

fn display_name(row: &DownloadRow) -> String {
    match row.track.as_deref() {
        Some(track) => format!("{} · {}", row.title, naming::track_label(track)),
        None => row.title.clone(),
    }
}

fn status_label(status: &str) -> &'static str {
    match status {
        "queued" => "正在排队",
        "downloading" => "正在下载",
        "paused" => "已暂停",
        "completed" => "已完成",
        "failed" => "已失败",
        "cancelled" => "已取消",
        _ => "状态未知",
    }
}

async fn remove_part(file_path: Option<&str>) {
    if let Some(path) = file_path {
        let _ = tokio::fs::remove_file(naming::part_path(&PathBuf::from(path))).await;
    }
}

/// Starts queued transfers for the logged-in account, up to the concurrency.
pub async fn run_scheduler(state: Arc<AppState>) {
    let mut transfers: JoinSet<()> = JoinSet::new();
    let mut tasks: HashMap<Id, String> = HashMap::new();
    tracing::info!("download scheduler started");
    loop {
        if let Some(profile) = state.account.profile().await {
            while transfers.len() < state.downloads.concurrency() {
                match state.downloads.claim(&profile.id).await {
                    Ok(Some(row)) => {
                        let token = CancellationToken::new();
                        state
                            .downloads
                            .running
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .insert(row.id.clone(), token.clone());
                        let _ = state.downloads.changed(&row.id).await;
                        let job = state.clone();
                        let id = row.id.clone();
                        let handle =
                            transfers.spawn(async move { transfer::run(job, id, token).await });
                        tasks.insert(handle.id(), row.id);
                    }
                    Ok(None) => break,
                    Err(error) => {
                        tracing::error!(%error, "could not claim a download");
                        break;
                    }
                }
            }
        }
        tokio::select! {
            _ = state.downloads.wake.notified() => {}
            joined = transfers.join_next_with_id(), if !transfers.is_empty() => {
                let (task, panicked) = match &joined {
                    Some(Ok((task, ()))) => (Some(*task), false),
                    Some(Err(error)) => (Some(error.id()), error.is_panic()),
                    None => (None, false),
                };
                if let Some(id) = task.and_then(|task| tasks.remove(&task)) {
                    state.downloads.running.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
                    state.downloads.live.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
                    if panicked {
                        let _ = sqlx::query("UPDATE downloads SET status = 'failed', error = '下载过程中发生内部错误', updated_at = $2 WHERE id = $1 AND status = 'downloading'")
                            .bind(&id)
                            .bind(db::now())
                            .execute(&state.pool)
                            .await;
                        let _ = state.downloads.changed(&id).await;
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(15)) => {}
        }
    }
}

#[cfg(test)]
mod tests;
