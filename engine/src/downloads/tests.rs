//! Transfers against a local media server (the demo school supplies the
//! tasks): completion, pause/resume with Range, cancel, retries, logout.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize},
    },
};

use axum::{
    Router,
    body::Body,
    extract::{Path as UrlPath, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use reqwest::cookie::Jar;
use tokio::{net::TcpListener, sync::Semaphore, task::JoinHandle};

use super::*;
use crate::{config::Config, fake};

fn byte_at(index: u64) -> u8 {
    ((index * 31 + 7) % 251) as u8
}

fn content(size: u64) -> Vec<u8> {
    (0..size).map(byte_at).collect()
}

/// Serves `size` bytes of a fixed pattern with Range support. The first
/// request from offset 0 can stall after `stall_after` bytes until the
/// transfer is dropped, so a test can pause or cancel at a known point.
#[derive(Clone, Default)]
struct MediaServer {
    stall_after: Option<u64>,
    ignore_range: bool,
    fail_first: Arc<AtomicBool>,
    stalled: Arc<AtomicBool>,
    ranges: Arc<std::sync::Mutex<Vec<Option<String>>>>,
    requests: Arc<AtomicUsize>,
}

async fn serve_media(
    State(server): State<MediaServer>,
    UrlPath(id): UrlPath<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    server
        .requests
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    server.ranges.lock().unwrap().push(range.clone());
    if server
        .fail_first
        .swap(false, std::sync::atomic::Ordering::SeqCst)
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let size: u64 = query["size"].parse().unwrap();
    let start = range
        .filter(|_| !server.ignore_range)
        .and_then(|value| {
            value
                .strip_prefix("bytes=")?
                .strip_suffix('-')?
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0);
    let stall = (start == 0)
        .then_some(server.stall_after)
        .flatten()
        .filter(|_| {
            !server
                .stalled
                .swap(true, std::sync::atomic::Ordering::SeqCst)
        });
    let chunks = futures_util::stream::unfold(start, move |offset| async move {
        if offset >= size {
            return None;
        }
        if stall.is_some_and(|limit| offset >= limit) {
            // Never released: the client drops the connection.
            let gate = Semaphore::new(0);
            let _ = gate.acquire().await;
        }
        let end = (offset + 64 * 1024)
            .min(size)
            .min(stall.filter(|limit| offset < *limit).unwrap_or(size));
        let bytes = (offset..end).map(byte_at).collect::<Vec<_>>();
        Some((Ok::<_, std::io::Error>(axum::body::Bytes::from(bytes)), end))
    });
    let mut response = Response::new(Body::from_stream(chunks));
    *response.status_mut() = if start > 0 {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("video/mp4"));
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(size - start));
    headers.insert(
        header::ETAG,
        HeaderValue::from_str(&format!("\"{id}-{size}\"")).unwrap(),
    );
    if start > 0 {
        headers.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{}/{size}", size - 1)).unwrap(),
        );
    }
    response
}

struct Fixture {
    state: Arc<AppState>,
    directory: tempfile::TempDir,
    server: MediaServer,
    upstream: JoinHandle<()>,
    scheduler: JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.upstream.abort();
        self.scheduler.abort();
    }
}

impl Fixture {
    async fn new(server: MediaServer) -> Self {
        let app = Router::new()
            .route("/media/{id}", get(serve_media))
            .with_state(server.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let upstream = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        crate::http::allow_test_origin(&origin);
        let directory = tempfile::tempdir().unwrap();
        let mut config = Config::school(directory.path().join("data"));
        config.fake_school = true;
        config.fake_media = Some(origin);
        let state = AppState::new(Arc::new(config), crate::db::open_memory().await);
        state
            .account
            .complete_login(Arc::new(Jar::default()), fake::profile())
            .await
            .unwrap();
        let scheduler = tokio::spawn(run_scheduler(state.clone()));
        Self {
            state,
            directory,
            server,
            upstream,
            scheduler,
        }
    }

    fn destination(&self) -> PathBuf {
        self.directory.path().join("下载")
    }

    async fn create(&self, items: Vec<NewDownload>) -> CreateResult {
        self.state
            .downloads
            .create(&fake::profile().id, &self.destination(), items)
            .await
            .unwrap()
    }

    async fn wait(
        &self,
        id: &str,
        what: &str,
        done: impl Fn(&DownloadInfo) -> bool,
    ) -> DownloadInfo {
        for _ in 0..600 {
            let info = self.state.downloads.get(id).await.unwrap();
            if done(&info) {
                return info;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!(
            "timed out waiting for {what}: {:?}",
            self.state.downloads.get(id).await.unwrap()
        );
    }
}

fn file(file_id: &str) -> NewDownload {
    NewDownload::File {
        course_id: "87954".into(),
        course_name: "机器学习与数据挖掘".into(),
        file_id: file_id.into(),
        title: "课件".into(),
        size: None,
    }
}

fn video(lesson: &str, track: &str) -> NewDownload {
    NewDownload::Video {
        course_id: "87954".into(),
        course_name: "机器学习与数据挖掘".into(),
        lesson_id: lesson.into(),
        title: "第 01 讲 · 课程导论".into(),
        begin_time: Some("2026-09-01 08:00:00".into()),
        track: track.into(),
        size: None,
    }
}

fn size_of_file(file_id: &str) -> u64 {
    fake::files("87954")
        .unwrap()
        .into_iter()
        .find(|file| file.id == file_id)
        .unwrap()
        .size
}

fn assert_content(path: &Path, size: u64) {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(bytes.len() as u64, size, "{}", path.display());
    assert!(
        bytes == content(size),
        "content of {} differs",
        path.display()
    );
    assert!(!naming::part_path(path).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn downloads_complete_into_course_folders() {
    let fixture = Fixture::new(MediaServer::default()).await;
    let result = fixture
        .create(vec![
            file("8795402"),
            file("8795401"),
            video("demo-87954-01", "slides"),
        ])
        .await;
    assert_eq!(result.created.len(), 3);
    assert!(result.skipped.is_empty());
    for info in &result.created {
        fixture
            .wait(&info.id, "completion", |info| info.status == "completed")
            .await;
    }
    let lecture = format!(
        "2026-09-01_08-00 第 01 讲 · 课程导论 [{}]",
        naming::short_id("demo-87954-01")
    );
    let course = fixture.destination().join("机器学习与数据挖掘 [87954]");
    assert_content(
        &course.join("课程文件").join("第 01 讲 课件.pdf"),
        size_of_file("8795402"),
    );
    assert_content(
        &course.join("课程文件").join("课程大纲.pdf"),
        size_of_file("8795401"),
    );
    let video_path = course.join("课堂录像").join(lecture).join("电脑屏幕.mp4");
    let video_size = std::fs::metadata(&video_path).unwrap().len();
    assert_content(&video_path, video_size);

    let info = fixture
        .state
        .downloads
        .get(&result.created[0].id)
        .await
        .unwrap();
    assert_eq!(info.received, info.total.unwrap());
    assert!(info.completed_at.is_some());
    let counts = fixture.state.downloads.counts().await.unwrap();
    assert_eq!((counts.all, counts.completed, counts.active), (3, 3, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pause_keeps_the_partial_file_and_resume_continues_with_range() {
    let fixture = Fixture::new(MediaServer {
        stall_after: Some(1_000_000),
        ..MediaServer::default()
    })
    .await;
    let id = fixture.create(vec![file("8795402")]).await.created[0]
        .id
        .clone();
    fixture
        .wait(&id, "the stall", |info| info.received >= 1_000_000)
        .await;
    let paused = fixture.state.downloads.pause(&id).await.unwrap();
    assert_eq!(paused.status, "paused");
    let path = PathBuf::from(
        fixture
            .wait(&id, "the file path", |info| info.file_path.is_some())
            .await
            .file_path
            .unwrap(),
    );
    // The transfer stops and flushes what it received.
    for _ in 0..200 {
        if std::fs::metadata(naming::part_path(&path))
            .map(|meta| meta.len())
            .unwrap_or(0)
            == 1_000_000
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        std::fs::metadata(naming::part_path(&path)).unwrap().len(),
        1_000_000
    );

    fixture.state.downloads.resume(&id).await.unwrap();
    fixture
        .wait(&id, "completion", |info| info.status == "completed")
        .await;
    assert_content(&path, size_of_file("8795402"));
    let ranges = fixture.server.ranges.lock().unwrap().clone();
    assert_eq!(ranges.last().unwrap().as_deref(), Some("bytes=1000000-"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_ignores_range_restarts_the_file() {
    let fixture = Fixture::new(MediaServer {
        stall_after: Some(700_000),
        ignore_range: true,
        ..MediaServer::default()
    })
    .await;
    let id = fixture.create(vec![file("8795402")]).await.created[0]
        .id
        .clone();
    fixture
        .wait(&id, "the stall", |info| info.received >= 700_000)
        .await;
    fixture.state.downloads.pause(&id).await.unwrap();
    fixture
        .wait(&id, "the pause", |info| info.status == "paused")
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    fixture.state.downloads.resume(&id).await.unwrap();
    let info = fixture
        .wait(&id, "completion", |info| info.status == "completed")
        .await;
    assert_content(
        Path::new(info.file_path.as_deref().unwrap()),
        size_of_file("8795402"),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_deletes_the_partial_file_and_retry_starts_over() {
    let fixture = Fixture::new(MediaServer {
        stall_after: Some(500_000),
        ..MediaServer::default()
    })
    .await;
    let id = fixture.create(vec![file("8795403")]).await.created[0]
        .id
        .clone();
    let path = PathBuf::from(
        fixture
            .wait(&id, "the stall", |info| {
                info.received >= 500_000 && info.file_path.is_some()
            })
            .await
            .file_path
            .unwrap(),
    );
    let cancelled = fixture.state.downloads.cancel(&id).await.unwrap();
    assert_eq!(cancelled.status, "cancelled");
    assert!(cancelled.file_path.is_none());
    for _ in 0..200 {
        if !naming::part_path(&path).exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!naming::part_path(&path).exists());
    assert!(!path.exists());
    for _ in 0..200 {
        if !fixture.state.downloads.is_running(&id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // The stopping transfer does not write its progress back.
    let stopped = fixture.state.downloads.get(&id).await.unwrap();
    assert_eq!(
        (stopped.status.as_str(), stopped.received),
        ("cancelled", 0)
    );

    fixture.state.downloads.retry(&id).await.unwrap();
    let info = fixture
        .wait(&id, "completion", |info| info.status == "completed")
        .await;
    assert_eq!(
        info.file_path.as_deref(),
        Some(path.to_string_lossy().as_ref())
    );
    assert_content(&path, size_of_file("8795403"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shutdown_starts_no_new_transfers() {
    let fixture = Fixture::new(MediaServer {
        stall_after: Some(300_000),
        ..MediaServer::default()
    })
    .await;
    fixture.state.downloads.set_concurrency(1);
    let created = fixture
        .create(vec![file("8795401"), file("8795402"), file("8795403")])
        .await
        .created;
    let first = fixture
        .wait(&created[0].id, "the stall", |info| info.received >= 300_000)
        .await;
    assert_eq!(first.status, "downloading");
    fixture.state.downloads.stop_all().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    // The stopped transfer stays "downloading" for the next start; the
    // queue behind it was not started while the engine was shutting down.
    for (item, status) in created.iter().zip(["downloading", "queued", "queued"]) {
        let info = fixture.state.downloads.get(&item.id).await.unwrap();
        assert_eq!(info.status, status, "{}", info.title);
    }
    assert!(naming::part_path(Path::new(&first.file_path.unwrap())).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancel_retried_at_once_leaves_no_partial_file() {
    let fixture = Fixture::new(MediaServer {
        stall_after: Some(500_000),
        ..MediaServer::default()
    })
    .await;
    let id = fixture.create(vec![file("8795402")]).await.created[0]
        .id
        .clone();
    let path = PathBuf::from(
        fixture
            .wait(&id, "the stall", |info| {
                info.received >= 500_000 && info.file_path.is_some()
            })
            .await
            .file_path
            .unwrap(),
    );
    // Retried before the old transfer has stopped: it waits for it.
    fixture.state.downloads.cancel(&id).await.unwrap();
    fixture.state.downloads.retry(&id).await.unwrap();
    let info = fixture
        .wait(&id, "completion", |info| info.status == "completed")
        .await;
    assert_eq!(info.file_path.as_deref(), path.to_str());
    assert_content(&path, size_of_file("8795402"));
    let leftovers = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "part"))
        .count();
    assert_eq!(leftovers, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn temporary_server_errors_are_retried() {
    let fixture = Fixture::new(MediaServer {
        fail_first: Arc::new(AtomicBool::new(true)),
        ..MediaServer::default()
    })
    .await;
    let id = fixture.create(vec![file("8795404")]).await.created[0]
        .id
        .clone();
    let info = fixture
        .wait(&id, "completion", |info| info.status == "completed")
        .await;
    assert_content(
        Path::new(info.file_path.as_deref().unwrap()),
        size_of_file("8795404"),
    );
    assert!(
        fixture
            .server
            .requests
            .load(std::sync::atomic::Ordering::SeqCst)
            >= 2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ended_login_holds_transfers_until_the_next_login() {
    let fixture = Fixture::new(MediaServer {
        stall_after: Some(300_000),
        ..MediaServer::default()
    })
    .await;
    let id = fixture.create(vec![file("8795402")]).await.created[0]
        .id
        .clone();
    fixture
        .wait(&id, "the stall", |info| info.received >= 300_000)
        .await;
    let profile = fake::profile();
    fixture.state.account.logout().await.unwrap();
    fixture.state.after_logout(Some(&profile.id)).await;
    let held = fixture
        .wait(&id, "the hold", |info| info.status == "queued")
        .await;
    assert!(held.error.unwrap().contains("重新登录"));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        fixture.state.downloads.get(&id).await.unwrap().status,
        "queued"
    );

    fixture
        .state
        .account
        .complete_login(Arc::new(Jar::default()), profile)
        .await
        .unwrap();
    fixture.state.downloads.wake();
    let info = fixture
        .wait(&id, "completion", |info| info.status == "completed")
        .await;
    assert_content(
        Path::new(info.file_path.as_deref().unwrap()),
        size_of_file("8795402"),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicates_are_skipped_and_finished_tasks_can_be_removed() {
    let fixture = Fixture::new(MediaServer::default()).await;
    let result = fixture
        .create(vec![
            file("8795401"),
            file("8795401"),
            video("bad id!", "slides"),
        ])
        .await;
    assert_eq!(result.created.len(), 1);
    assert_eq!(result.skipped.len(), 2);
    assert_eq!(result.skipped[0].reason, "queued");
    assert!(result.skipped[0].message.contains("已在下载队列中"));
    assert_eq!(result.skipped[1].reason, "invalid");
    let id = result.created[0].id.clone();
    let info = fixture
        .wait(&id, "completion", |info| info.status == "completed")
        .await;
    let path = PathBuf::from(info.file_path.unwrap());

    // A finished download whose file is still there is not repeated…
    let again = fixture.create(vec![file("8795401")]).await;
    assert!(again.created.is_empty());
    assert_eq!(again.skipped[0].reason, "downloaded");

    // …unless the file is gone: then it is downloaded again under its name.
    std::fs::remove_file(&path).unwrap();
    let again = fixture.create(vec![file("8795401")]).await.created[0]
        .id
        .clone();
    let second = fixture
        .wait(&again, "completion", |info| info.status == "completed")
        .await;
    assert_eq!(second.file_path.as_deref(), path.to_str());

    // Removing a finished task keeps its file; a new copy gets a numbered name.
    assert!(fixture.state.downloads.remove(&id).await.is_ok());
    assert!(fixture.state.downloads.remove(&again).await.is_ok());
    assert!(path.exists());
    let third = fixture.create(vec![file("8795401")]).await.created[0]
        .id
        .clone();
    let third = fixture
        .wait(&third, "completion", |info| info.status == "completed")
        .await;
    assert!(third.file_path.unwrap().ends_with("课程大纲 (2).pdf"));

    assert_eq!(fixture.state.downloads.clear_completed().await.unwrap(), 1);
    assert_eq!(fixture.state.downloads.counts().await.unwrap().all, 0);
}
