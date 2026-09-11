//! JSON-RPC notifications for the host application. The engine has exactly
//! one host connection, so a process-wide sink keeps the rest of the code
//! independent of the transport.

use std::sync::OnceLock;

use serde::Serialize;
use serde_json::json;
use tokio::sync::mpsc;

static SINK: OnceLock<mpsc::UnboundedSender<String>> = OnceLock::new();

pub fn install_sink(sender: mpsc::UnboundedSender<String>) {
    let _ = SINK.set(sender);
}

pub fn notify(method: &str, params: impl Serialize) {
    let Some(sink) = SINK.get() else {
        return;
    };
    match serde_json::to_string(&json!({ "method": method, "params": params })) {
        Ok(line) => {
            let _ = sink.send(line);
        }
        Err(error) => tracing::warn!(%error, method, "无法序列化通知"),
    }
}
