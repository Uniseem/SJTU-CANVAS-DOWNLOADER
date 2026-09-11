//! Outbound HTTP policy shared by every client the engine builds: the user's
//! proxy choice, and the loopback test servers a debug build may accept.

use std::sync::RwLock;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProxySettings {
    /// Environment variables, then the Windows / macOS system proxy.
    #[default]
    System,
    /// Direct connections only (useful when a system-wide proxy breaks the
    /// campus services).
    Direct,
    /// An explicit `http://`, `https://` or `socks5://` proxy.
    Custom { url: String },
}

impl ProxySettings {
    pub fn validate(&self) -> Result<()> {
        if let Self::Custom { url } = self {
            let parsed = Url::parse(url.trim()).context("代理地址格式不正确")?;
            anyhow::ensure!(
                matches!(parsed.scheme(), "http" | "https" | "socks5" | "socks5h"),
                "代理地址只支持 http、https 或 socks5"
            );
            anyhow::ensure!(parsed.host_str().is_some(), "代理地址缺少主机名");
        }
        Ok(())
    }
}

static PROXY: RwLock<ProxySettings> = RwLock::new(ProxySettings::System);
static TEST_ORIGINS: RwLock<Vec<String>> = RwLock::new(Vec::new());

pub fn proxy() -> ProxySettings {
    PROXY
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

pub fn set_proxy(proxy: ProxySettings) {
    *PROXY.write().unwrap_or_else(|error| error.into_inner()) = proxy;
}

/// Applies the current proxy policy to a client builder.
pub fn apply(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    match proxy() {
        ProxySettings::System => builder,
        ProxySettings::Direct => builder.no_proxy(),
        ProxySettings::Custom { url } => match reqwest::Proxy::all(url.trim()) {
            Ok(proxy) => builder.proxy(proxy),
            // Validated when saved; an unusable value falls back to direct.
            Err(_) => builder.no_proxy(),
        },
    }
}

pub fn user_agent() -> String {
    format!("SJTUCanvasDownloader/{}", env!("CARGO_PKG_VERSION"))
}

/// Lets a debug build (or a unit test) use a loopback test server as an
/// upstream. Release builds never accept one.
pub fn allow_test_origin(origin: &str) {
    if !cfg!(any(test, debug_assertions)) {
        return;
    }
    if let Some(origin) = Url::parse(origin).ok().and_then(|url| origin_of(&url))
        && is_loopback(&origin)
    {
        let mut origins = TEST_ORIGINS
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if !origins.contains(&origin) {
            origins.push(origin);
        }
    }
}

pub fn is_test_origin(url: &Url) -> bool {
    cfg!(any(test, debug_assertions))
        && origin_of(url).is_some_and(|origin| {
            TEST_ORIGINS
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .contains(&origin)
        })
}

fn origin_of(url: &Url) -> Option<String> {
    Some(format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str()?,
        url.port_or_known_default()?
    ))
}

fn is_loopback(origin: &str) -> bool {
    Url::parse(origin)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .is_some_and(|host| matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_settings_round_trip_and_reject_unknown_schemes() {
        for proxy in [
            ProxySettings::System,
            ProxySettings::Direct,
            ProxySettings::Custom {
                url: "http://127.0.0.1:7890".into(),
            },
        ] {
            let json = serde_json::to_string(&proxy).unwrap();
            assert_eq!(serde_json::from_str::<ProxySettings>(&json).unwrap(), proxy);
            proxy.validate().unwrap();
        }
        assert_eq!(
            serde_json::to_value(ProxySettings::Direct).unwrap(),
            serde_json::json!({"mode": "direct"})
        );
        for url in ["ftp://proxy", "not a url"] {
            assert!(
                ProxySettings::Custom { url: url.into() }
                    .validate()
                    .is_err()
            );
        }
    }

    #[test]
    fn only_registered_loopback_origins_are_test_origins() {
        allow_test_origin("http://127.0.0.1:45871");
        allow_test_origin("https://example.com");
        assert!(is_test_origin(
            &Url::parse("http://127.0.0.1:45871/media/a.mp4").unwrap()
        ));
        assert!(!is_test_origin(
            &Url::parse("http://127.0.0.1:45872/").unwrap()
        ));
        assert!(!is_test_origin(
            &Url::parse("https://127.0.0.1/secret").unwrap()
        ));
        assert!(!is_test_origin(
            &Url::parse("https://example.com/").unwrap()
        ));
    }
}
