//! SJTU Canvas Downloader engine: the shared core of the Windows and macOS
//! apps. It signs in to Canvas through 交我办 QR codes, lists courses, lesson
//! recordings and course files, and downloads them.
//!
//! The host app starts `sjtu-canvas-engine --data-dir <dir>` and talks
//! JSON-RPC over the process's stdin/stdout (see `rpc.rs`).

mod account;
mod auth;
mod canvas;
mod config;
mod db;
mod downloads;
mod error;
mod events;
mod fake;
mod guard;
mod http;
mod models;
mod rpc;
mod settings;
mod state;
mod video;

use std::{fs::OpenOptions, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use config::{Args, Config};
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    let args = Args::parse()?;
    if args.version {
        println!("sjtu-canvas-engine {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let config = Arc::new(Config::new(&args)?);
    for directory in [&config.data_root, &config.log_root] {
        std::fs::create_dir_all(directory)
            .with_context(|| format!("无法创建目录 {}", directory.display()))?;
    }
    init_logging(&config)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(config.data_root.join("engine.lock"))
        .context("无法创建引擎锁文件")?;
    if lock.try_lock().is_err() {
        anyhow::bail!("同一个数据目录已有一个 SJTU Canvas Downloader 引擎在运行");
    }
    for origin in config.test_origins() {
        http::allow_test_origin(&origin);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("sjtu-canvas-engine")
        .build()?;
    let result = runtime.block_on(run(config));
    if let Err(error) = &result {
        tracing::error!(error = %format!("{error:#}"), "engine stopped with an error");
    }
    runtime.shutdown_timeout(Duration::from_secs(5));
    drop(lock);
    result
}

async fn run(config: Arc<Config>) -> Result<()> {
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        data = %config.data_root.display(),
        test_mode = config.test_mode,
        fake_school = config.fake_school,
        "engine starting"
    );
    let pool = db::open(&config.database_path).await?;
    let state = state::AppState::new(config, pool);
    state
        .downloads
        .reset_interrupted()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let (outgoing, lines) = mpsc::unbounded_channel();
    events::install_sink(outgoing.clone());
    rpc::serve(state, outgoing, lines).await
}

fn init_logging(config: &Config) -> Result<()> {
    let path = config.log_root.join("engine.log");
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 8 * 1024 * 1024) {
        let _ = std::fs::rename(&path, config.log_root.join("engine.log.1"));
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("无法打开日志文件 {}", path.display()))?;
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("SJTU_CANVAS_LOG")
                .unwrap_or_else(|_| "sjtu_canvas_engine=info".into()),
        )
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file))
        .init();
    Ok(())
}

/// A random URL-safe identifier.
pub fn random_token(bytes: usize) -> String {
    let data = (0..bytes).map(|_| rand::random::<u8>()).collect::<Vec<_>>();
    URL_SAFE_NO_PAD.encode(data)
}
