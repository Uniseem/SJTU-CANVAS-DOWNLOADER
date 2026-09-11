//! The Canvas account of this installation. Login cookies live in memory; on
//! disk they are encrypted (ChaCha20-Poly1305) with a key that the host app
//! keeps in the OS credential store (Windows Credential Manager / macOS
//! Keychain) and hands to the engine at start-up. Only the Canvas and
//! classroom-video sites' cookies are kept, never jAccount's.

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result, anyhow};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use chrono::Utc;
use reqwest::cookie::{CookieStore, Jar};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use url::Url;

use crate::{config::Config, models::CanvasProfile};

const FILE_VERSION: u8 = 1;
const AAD: &[u8] = b"sjtu-canvas-downloader/session/v1";

#[derive(Clone, Debug, Serialize)]
pub struct AccountView {
    pub authenticated: bool,
    pub profile: Option<CanvasProfile>,
    /// False when the host could not provide a credential-store key: the
    /// login then lasts only until the engine stops.
    pub persisted: bool,
}

struct Login {
    jar: Arc<Jar>,
    profile: CanvasProfile,
}

pub struct Account {
    login: RwLock<Option<Login>>,
    key: std::sync::RwLock<Option<[u8; 32]>>,
    path: PathBuf,
    origins: Vec<String>,
    persist_lock: Mutex<()>,
}

#[derive(Serialize, Deserialize)]
struct EncryptedFile {
    version: u8,
    nonce: String,
    ciphertext: String,
}

#[derive(Serialize, Deserialize)]
struct SavedLogin {
    profile: CanvasProfile,
    cookies: Vec<OriginCookies>,
    saved_at: String,
}

#[derive(Serialize, Deserialize)]
struct OriginCookies {
    origin: String,
    header: String,
}

impl Account {
    pub fn new(config: &Config) -> Self {
        let mut origins = Vec::new();
        for value in [
            &config.canvas_origin,
            &config.courses_origin,
            &config.video_api,
            &config.video_lti_adapter,
            &config.resource_video_api,
        ] {
            if let Some(origin) = origin_of(value)
                && !origins.contains(&origin)
            {
                origins.push(origin);
            }
        }
        Self {
            login: RwLock::new(None),
            key: std::sync::RwLock::new(None),
            path: config.session_path.clone(),
            origins,
            persist_lock: Mutex::new(()),
        }
    }

    /// Accepts the host's key (base64 of 32 bytes) and restores a saved login.
    /// A login saved under another key cannot be read and is discarded.
    pub async fn unlock(&self, key: Option<&str>) -> Result<()> {
        let key = match key.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => Some(decode_key(value)?),
            None => None,
        };
        *self.key.write().unwrap_or_else(|error| error.into_inner()) = key;
        let Some(key) = key else {
            return Ok(());
        };
        if self.login.read().await.is_some() {
            return Ok(());
        }
        match self.read_file(&key).await {
            Ok(Some(login)) => {
                *self.login.write().await = Some(login);
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(error = %format!("{error:#}"), "discarding unreadable saved login");
                let _ = tokio::fs::remove_file(&self.path).await;
            }
        }
        Ok(())
    }

    pub async fn current(&self) -> Option<(Arc<Jar>, CanvasProfile)> {
        self.login
            .read()
            .await
            .as_ref()
            .map(|login| (login.jar.clone(), login.profile.clone()))
    }

    pub async fn profile(&self) -> Option<CanvasProfile> {
        self.login
            .read()
            .await
            .as_ref()
            .map(|login| login.profile.clone())
    }

    pub async fn view(&self) -> AccountView {
        let profile = self.profile().await;
        AccountView {
            authenticated: profile.is_some(),
            profile,
            persisted: self
                .key
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .is_some(),
        }
    }

    /// True while `jar` is still the jar of the current login.
    pub async fn is_current(&self, jar: &Arc<Jar>) -> bool {
        self.login
            .read()
            .await
            .as_ref()
            .is_some_and(|login| Arc::ptr_eq(&login.jar, jar))
    }

    pub async fn complete_login(&self, jar: Arc<Jar>, profile: CanvasProfile) -> Result<()> {
        let sanitized = Arc::new(restore_cookies(&snapshot_cookies(&jar, &self.origins)));
        *self.login.write().await = Some(Login {
            jar: sanitized,
            profile,
        });
        self.persist().await
    }

    pub async fn logout(&self) -> Result<()> {
        *self.login.write().await = None;
        self.persist().await
    }

    /// Logs out only if `expected` is still the current login's jar, so a
    /// late failure from an old request never ends a newer login.
    pub async fn logout_if_current(&self, expected: &Arc<Jar>) -> Result<bool> {
        {
            let mut login = self.login.write().await;
            if !login
                .as_ref()
                .is_some_and(|login| Arc::ptr_eq(&login.jar, expected))
            {
                return Ok(false);
            }
            *login = None;
        }
        self.persist().await?;
        Ok(true)
    }

    async fn persist(&self) -> Result<()> {
        let _guard = self.persist_lock.lock().await;
        let key = *self.key.read().unwrap_or_else(|error| error.into_inner());
        let payload = self.login.read().await.as_ref().map(|login| SavedLogin {
            profile: login.profile.clone(),
            cookies: snapshot_cookies(&login.jar, &self.origins),
            saved_at: Utc::now().to_rfc3339(),
        });
        let (Some(key), Some(payload)) = (key, payload) else {
            // Logged out, or no key to protect the cookies with.
            match tokio::fs::remove_file(&self.path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("无法删除保存的登录状态"),
            }
            return Ok(());
        };
        let plaintext = serde_json::to_vec(&payload)?;
        let nonce: [u8; 12] = rand::random();
        let ciphertext = ChaCha20Poly1305::new((&key).into())
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: AAD,
                },
            )
            .map_err(|_| anyhow!("无法加密登录状态"))?;
        let document = serde_json::to_vec(&EncryptedFile {
            version: FILE_VERSION,
            nonce: URL_SAFE_NO_PAD.encode(nonce),
            ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
        })?;
        let temporary = self.path.with_extension("tmp");
        tokio::fs::write(&temporary, document)
            .await
            .context("无法保存登录状态")?;
        tokio::fs::rename(&temporary, &self.path)
            .await
            .context("无法保存登录状态")?;
        Ok(())
    }

    async fn read_file(&self, key: &[u8; 32]) -> Result<Option<Login>> {
        let bytes = match tokio::fs::read(&self.path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("无法读取保存的登录状态"),
        };
        let file: EncryptedFile = serde_json::from_slice(&bytes).context("登录状态文件已损坏")?;
        anyhow::ensure!(file.version == FILE_VERSION, "不支持的登录状态版本");
        let nonce = URL_SAFE_NO_PAD.decode(&file.nonce)?;
        anyhow::ensure!(nonce.len() == 12, "登录状态文件已损坏");
        let ciphertext = URL_SAFE_NO_PAD.decode(&file.ciphertext)?;
        let plaintext = ChaCha20Poly1305::new(key.into())
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: AAD,
                },
            )
            .map_err(|_| anyhow!("登录状态无法用当前密钥解密"))?;
        let payload: SavedLogin = serde_json::from_slice(&plaintext).context("登录状态格式无效")?;
        Ok(Some(Login {
            jar: Arc::new(restore_cookies(&payload.cookies)),
            profile: payload.profile,
        }))
    }
}

fn decode_key(value: &str) -> Result<[u8; 32]> {
    let bytes = STANDARD
        .decode(value)
        .or_else(|_| URL_SAFE_NO_PAD.decode(value))
        .context("登录状态密钥不是有效的 base64")?;
    bytes
        .try_into()
        .map_err(|_| anyhow!("登录状态密钥必须是 32 字节"))
}

fn snapshot_cookies(jar: &Jar, origins: &[String]) -> Vec<OriginCookies> {
    origins
        .iter()
        .filter_map(|origin| {
            let url = Url::parse(origin).ok()?;
            let header = jar.cookies(&url)?.to_str().ok()?.to_string();
            (!header.is_empty()).then(|| OriginCookies {
                origin: origin.clone(),
                header,
            })
        })
        .collect()
}

fn restore_cookies(cookies: &[OriginCookies]) -> Jar {
    let jar = Jar::default();
    for entry in cookies {
        let Ok(url) = Url::parse(&entry.origin) else {
            continue;
        };
        let secure = if url.scheme() == "https" {
            "; Secure"
        } else {
            ""
        };
        for pair in entry
            .header
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            jar.add_cookie_str(&format!("{pair}; Path=/; HttpOnly{secure}"), &url);
        }
    }
    jar
}

fn origin_of(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    Some(match url.port() {
        Some(port) => format!("{}://{}:{port}", url.scheme(), url.host_str()?),
        None => format!("{}://{}", url.scheme(), url.host_str()?),
    })
}

/// A new random key for hosts that have none yet (base64, 32 bytes).
#[cfg(test)]
pub fn new_key() -> String {
    STANDARD.encode(rand::random::<[u8; 32]>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str) -> CanvasProfile {
        CanvasProfile {
            id: id.into(),
            name: "测试用户".into(),
            short_name: "测试".into(),
            avatar_url: None,
        }
    }

    fn account(directory: &std::path::Path) -> Account {
        Account::new(&Config::school(directory.to_path_buf()))
    }

    #[test]
    fn cookie_snapshot_round_trip() {
        let jar = Jar::default();
        let origin = Url::parse("https://oc.sjtu.edu.cn").unwrap();
        jar.add_cookie_str("canvas_session=secret; Path=/; Secure; HttpOnly", &origin);
        let snapshot = snapshot_cookies(&jar, &["https://oc.sjtu.edu.cn".to_string()]);
        let restored = restore_cookies(&snapshot);
        let header = restored.cookies(&origin).unwrap();
        assert!(header.to_str().unwrap().contains("canvas_session=secret"));
    }

    #[tokio::test]
    async fn encrypted_login_survives_restart_without_jaccount_cookies() {
        let directory = tempfile::tempdir().unwrap();
        let key = new_key();
        let store = account(directory.path());
        store.unlock(Some(&key)).await.unwrap();
        let jar = Arc::new(Jar::default());
        let canvas = Url::parse("https://oc.sjtu.edu.cn").unwrap();
        let jaccount = Url::parse("https://jaccount.sjtu.edu.cn").unwrap();
        jar.add_cookie_str("canvas_session=canvas-secret; Path=/; Secure", &canvas);
        jar.add_cookie_str("JSESSIONID=jaccount-secret; Path=/; Secure", &jaccount);
        store.complete_login(jar, profile("42")).await.unwrap();
        let saved = std::fs::read_to_string(directory.path().join("session.bin")).unwrap();
        assert!(!saved.contains("canvas-secret"));
        drop(store);

        let restored = account(directory.path());
        restored.unlock(Some(&key)).await.unwrap();
        let (jar, profile) = restored.current().await.unwrap();
        assert_eq!(profile.id, "42");
        assert!(
            jar.cookies(&canvas)
                .unwrap()
                .to_str()
                .unwrap()
                .contains("canvas_session=canvas-secret")
        );
        assert!(jar.cookies(&jaccount).is_none());
        assert!(restored.view().await.persisted);
    }

    #[tokio::test]
    async fn another_key_cannot_read_the_login_and_logout_removes_it() {
        let directory = tempfile::tempdir().unwrap();
        let store = account(directory.path());
        store.unlock(Some(&new_key())).await.unwrap();
        store
            .complete_login(Arc::new(Jar::default()), profile("42"))
            .await
            .unwrap();
        let other = account(directory.path());
        other.unlock(Some(&new_key())).await.unwrap();
        assert!(other.current().await.is_none());
        assert!(!directory.path().join("session.bin").exists());

        store.logout().await.unwrap();
        assert!(!directory.path().join("session.bin").exists());
        assert!(!store.view().await.authenticated);
    }

    #[tokio::test]
    async fn without_a_key_the_login_is_kept_in_memory_only() {
        let directory = tempfile::tempdir().unwrap();
        let store = account(directory.path());
        store.unlock(None).await.unwrap();
        store
            .complete_login(Arc::new(Jar::default()), profile("42"))
            .await
            .unwrap();
        assert!(store.view().await.authenticated);
        assert!(!store.view().await.persisted);
        assert!(!directory.path().join("session.bin").exists());
        assert!(store.unlock(Some("too-short")).await.is_err());
    }

    #[tokio::test]
    async fn late_unauthorized_cannot_clear_a_new_login() {
        let directory = tempfile::tempdir().unwrap();
        let store = account(directory.path());
        let old_jar = Arc::new(Jar::default());
        store
            .complete_login(old_jar.clone(), profile("42"))
            .await
            .unwrap();
        // complete_login stores a sanitized copy of the jar.
        let (current_jar, _) = store.current().await.unwrap();
        assert!(!store.logout_if_current(&old_jar).await.unwrap());
        store
            .complete_login(Arc::new(Jar::default()), profile("42"))
            .await
            .unwrap();
        assert!(!store.logout_if_current(&current_jar).await.unwrap());
        assert!(store.view().await.authenticated);
        let (newest, _) = store.current().await.unwrap();
        assert!(store.logout_if_current(&newest).await.unwrap());
        assert!(!store.view().await.authenticated);
    }
}
