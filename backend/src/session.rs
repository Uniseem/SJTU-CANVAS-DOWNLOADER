use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use chrono::Utc;
use reqwest::cookie::{CookieStore, Jar};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    fs,
    sync::{Mutex, RwLock},
};
use url::Url;

use crate::{
    config::Config,
    error::{AppError, AppResult},
    models::{CanvasProfile, SessionView},
};

const REGISTRY_VERSION: u8 = 1;
const SECRET_FILE: &str = "secret.key";
const SESSION_FILE: &str = "sessions.json";
const MAX_ANONYMOUS_SESSIONS: usize = 1024;
const ANONYMOUS_IDLE_SECONDS: i64 = 30 * 60;

#[derive(Clone, Debug)]
pub struct SessionId(pub String);

pub struct UserSession {
    pub id: String,
    jar: RwLock<Arc<Jar>>,
    profile: RwLock<Option<CanvasProfile>>,
    authenticated: AtomicBool,
    auth_gate: Mutex<()>,
    last_seen: AtomicI64,
}

impl UserSession {
    fn new(id: String, jar: Arc<Jar>, profile: Option<CanvasProfile>, last_seen: i64) -> Self {
        Self {
            id,
            jar: RwLock::new(jar),
            authenticated: AtomicBool::new(profile.is_some()),
            auth_gate: Mutex::new(()),
            profile: RwLock::new(profile),
            last_seen: AtomicI64::new(last_seen),
        }
    }

    pub fn touch(&self) {
        self.touch_at(Utc::now().timestamp());
    }

    fn touch_at(&self, now: i64) {
        self.last_seen.store(now, Ordering::Relaxed);
    }

    fn last_seen(&self) -> i64 {
        self.last_seen.load(Ordering::Relaxed)
    }

    fn is_authenticated(&self) -> bool {
        self.authenticated.load(Ordering::Acquire)
    }

    pub async fn jar(&self) -> Arc<Jar> {
        self.jar.read().await.clone()
    }

    pub async fn profile(&self) -> Option<CanvasProfile> {
        self.profile.read().await.clone()
    }

    async fn replace_auth(&self, jar: Arc<Jar>, profile: CanvasProfile) {
        let _guard = self.auth_gate.lock().await;
        *self.jar.write().await = jar;
        *self.profile.write().await = Some(profile);
        self.authenticated.store(true, Ordering::Release);
        self.touch();
    }

    async fn clear_auth(&self) {
        let _guard = self.auth_gate.lock().await;
        self.clear_auth_unlocked().await;
    }

    async fn clear_auth_if_current(&self, expected: &Arc<Jar>) -> bool {
        let _guard = self.auth_gate.lock().await;
        if !self.is_authenticated() || !Arc::ptr_eq(&self.jar().await, expected) {
            return false;
        }
        self.clear_auth_unlocked().await;
        true
    }

    async fn clear_auth_unlocked(&self) {
        *self.jar.write().await = Arc::new(Jar::default());
        *self.profile.write().await = None;
        self.authenticated.store(false, Ordering::Release);
        self.touch();
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedRegistry {
    version: u8,
    sessions: Vec<EncryptedSession>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EncryptedSession {
    id: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionPayload {
    profile: CanvasProfile,
    cookies: Vec<OriginCookies>,
    last_seen: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OriginCookies {
    origin: String,
    header: String,
}

pub struct SessionStore {
    sessions: RwLock<HashMap<String, Arc<UserSession>>>,
    cipher: ChaCha20Poly1305,
    registry_path: std::path::PathBuf,
    origins: Vec<String>,
    ttl_seconds: i64,
    demo_mode: bool,
    persist_lock: Mutex<()>,
}

impl SessionStore {
    pub async fn open(config: &Config) -> Result<Self> {
        fs::create_dir_all(&config.data_dir)
            .await
            .with_context(|| format!("无法创建数据目录 {}", config.data_dir.display()))?;
        let key = load_or_create_key(&config.data_dir, config.app_secret.as_deref()).await?;
        let cipher = ChaCha20Poly1305::new((&key).into());
        let registry_path = config.data_dir.join(SESSION_FILE);
        let origins = vec![
            config.canvas_origin.clone(),
            config.courses_origin.clone(),
            origin_of(&config.video_api).unwrap_or_else(|| config.video_api.clone()),
        ];
        let store = Self {
            sessions: RwLock::new(HashMap::new()),
            cipher,
            registry_path,
            origins,
            ttl_seconds: config.session_ttl.as_secs() as i64,
            demo_mode: config.demo_mode,
            persist_lock: Mutex::new(()),
        };
        store.load().await?;
        Ok(store)
    }

    pub async fn ensure(&self, incoming: Option<&str>) -> (Arc<UserSession>, bool) {
        let now = Utc::now().timestamp();
        let mut sessions = self.sessions.write().await;
        if let Some(id) = incoming.filter(|value| valid_session_id(value))
            && let Some(session) = sessions.get(id).cloned()
        {
            if !session_expired(session.last_seen(), now, self.ttl_seconds) {
                session.touch_at(now);
                return (session, false);
            }
            sessions.remove(id);
        }

        prune_sessions(&mut sessions, now, self.ttl_seconds);
        let id = random_token(32);
        let profile = self.demo_mode.then(demo_profile);
        let session = Arc::new(UserSession::new(
            id.clone(),
            Arc::new(Jar::default()),
            profile,
            now,
        ));
        sessions.insert(id, session.clone());
        (session, true)
    }

    pub async fn get(&self, id: &str) -> AppResult<Arc<UserSession>> {
        let session = self
            .sessions
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or(AppError::Unauthorized)?;
        if !session_expired(
            session.last_seen(),
            Utc::now().timestamp(),
            self.ttl_seconds,
        ) {
            return Ok(session);
        }
        let mut sessions = self.sessions.write().await;
        if sessions.get(id).is_some_and(|current| {
            Arc::ptr_eq(current, &session)
                && session_expired(
                    current.last_seen(),
                    Utc::now().timestamp(),
                    self.ttl_seconds,
                )
        }) {
            sessions.remove(id);
        }
        Err(AppError::Unauthorized)
    }

    pub async fn authenticated(
        &self,
        id: &str,
    ) -> AppResult<(Arc<UserSession>, Arc<Jar>, CanvasProfile)> {
        let session = self.get(id).await?;
        let profile = session.profile().await.ok_or(AppError::Unauthorized)?;
        let jar = session.jar().await;
        Ok((session, jar, profile))
    }

    pub async fn view(&self, id: &str) -> SessionView {
        let profile = match self.get(id).await {
            Ok(session) => session.profile().await,
            Err(_) => None,
        };
        SessionView {
            authenticated: profile.is_some(),
            demo: self.demo_mode,
            profile,
        }
    }

    pub async fn complete_login(
        &self,
        id: &str,
        jar: Arc<Jar>,
        profile: CanvasProfile,
    ) -> AppResult<()> {
        let session = self.get(id).await?;
        let sanitized_jar = Arc::new(restore_cookies(&snapshot_cookies(&jar, &self.origins)));
        session.replace_auth(sanitized_jar, profile).await;
        self.persist().await.map_err(AppError::internal)
    }

    pub async fn logout(&self, id: &str) -> AppResult<()> {
        let session = self.get(id).await?;
        session.clear_auth().await;
        self.persist().await.map_err(AppError::internal)
    }

    pub async fn logout_if_current(&self, id: &str, expected: &Arc<Jar>) -> AppResult<bool> {
        let session = self.get(id).await?;
        if !session.clear_auth_if_current(expected).await {
            return Ok(false);
        }
        self.persist().await.map_err(AppError::internal)?;
        Ok(true)
    }

    pub async fn persist(&self) -> Result<()> {
        if self.demo_mode {
            return Ok(());
        }
        let _persist_guard = self.persist_lock.lock().await;
        let sessions = self
            .sessions
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let cutoff = Utc::now().timestamp() - self.ttl_seconds;
        let mut encrypted = Vec::new();

        for session in sessions {
            let last_seen = session.last_seen.load(Ordering::Relaxed);
            if last_seen < cutoff {
                continue;
            }
            let Some(profile) = session.profile().await else {
                continue;
            };
            let jar = session.jar().await;
            let cookies = snapshot_cookies(&jar, &self.origins);
            let payload = SessionPayload {
                profile,
                cookies,
                last_seen,
            };
            let plaintext = serde_json::to_vec(&payload)?;
            let nonce_bytes: [u8; 12] = rand::random();
            let ciphertext = self
                .cipher
                .encrypt(
                    Nonce::from_slice(&nonce_bytes),
                    Payload {
                        msg: &plaintext,
                        aad: session.id.as_bytes(),
                    },
                )
                .map_err(|_| anyhow!("无法加密会话"))?;
            encrypted.push(EncryptedSession {
                id: session.id.clone(),
                nonce: URL_SAFE_NO_PAD.encode(nonce_bytes),
                ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
            });
        }

        let document = serde_json::to_vec_pretty(&PersistedRegistry {
            version: REGISTRY_VERSION,
            sessions: encrypted,
        })?;
        fs::write(&self.registry_path, document).await?;
        Ok(())
    }

    async fn load(&self) -> Result<()> {
        if self.demo_mode || !fs::try_exists(&self.registry_path).await? {
            return Ok(());
        }
        let bytes = fs::read(&self.registry_path).await?;
        let registry: PersistedRegistry = serde_json::from_slice(&bytes)
            .context("会话文件格式损坏，可删除 data/sessions.json 后重新登录")?;
        if registry.version != REGISTRY_VERSION {
            tracing::warn!(
                version = registry.version,
                "ignoring unsupported session registry"
            );
            return Ok(());
        }
        let cutoff = Utc::now().timestamp() - self.ttl_seconds;
        let mut restored = self.sessions.write().await;
        for encrypted in registry.sessions {
            if !valid_session_id(&encrypted.id) {
                continue;
            }
            let Ok(nonce) = URL_SAFE_NO_PAD.decode(&encrypted.nonce) else {
                continue;
            };
            let Ok(ciphertext) = URL_SAFE_NO_PAD.decode(&encrypted.ciphertext) else {
                continue;
            };
            if nonce.len() != 12 {
                continue;
            }
            let Ok(plaintext) = self.cipher.decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: encrypted.id.as_bytes(),
                },
            ) else {
                tracing::warn!(session = %encrypted.id, "failed to decrypt saved session");
                continue;
            };
            let Ok(payload) = serde_json::from_slice::<SessionPayload>(&plaintext) else {
                continue;
            };
            if payload.last_seen < cutoff {
                continue;
            }
            let jar = restore_cookies(&payload.cookies);
            restored.insert(
                encrypted.id.clone(),
                Arc::new(UserSession::new(
                    encrypted.id,
                    Arc::new(jar),
                    Some(payload.profile),
                    payload.last_seen,
                )),
            );
        }
        Ok(())
    }
}

fn session_expired(last_seen: i64, now: i64, ttl_seconds: i64) -> bool {
    ttl_seconds <= 0 || now.saturating_sub(last_seen) >= ttl_seconds
}

fn prune_sessions(sessions: &mut HashMap<String, Arc<UserSession>>, now: i64, ttl_seconds: i64) {
    sessions.retain(|_, session| {
        !session_expired(session.last_seen(), now, ttl_seconds)
            && (session.is_authenticated()
                || now.saturating_sub(session.last_seen()) < ANONYMOUS_IDLE_SECONDS)
    });

    let mut anonymous = sessions
        .iter()
        .filter(|(_, session)| !session.is_authenticated())
        .map(|(id, session)| (id.clone(), session.last_seen()))
        .collect::<Vec<_>>();
    if anonymous.len() < MAX_ANONYMOUS_SESSIONS {
        return;
    }
    anonymous.sort_by_key(|(_, last_seen)| *last_seen);
    let remove_count = anonymous.len() - MAX_ANONYMOUS_SESSIONS + 1;
    for (id, _) in anonymous.into_iter().take(remove_count) {
        sessions.remove(&id);
    }
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
        for pair in entry
            .header
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            jar.add_cookie_str(&format!("{pair}; Path=/; Secure; HttpOnly"), &url);
        }
    }
    jar
}

async fn load_or_create_key(data_dir: &Path, configured: Option<&str>) -> Result<[u8; 32]> {
    if let Some(secret) = configured {
        return Ok(Sha256::digest(secret.as_bytes()).into());
    }

    let path = data_dir.join(SECRET_FILE);
    if fs::try_exists(&path).await? {
        let encoded = fs::read_to_string(&path).await?;
        let bytes = URL_SAFE_NO_PAD
            .decode(encoded.trim())
            .context("data/secret.key 格式无效")?;
        return bytes
            .try_into()
            .map_err(|_| anyhow!("data/secret.key 长度无效"));
    }

    let key: [u8; 32] = rand::random();
    fs::write(&path, URL_SAFE_NO_PAD.encode(key)).await?;
    tracing::warn!(path = %path.display(), "APP_SECRET not set; generated a local encryption key");
    Ok(key)
}

pub fn random_token(bytes: usize) -> String {
    let data = (0..bytes).map(|_| rand::random::<u8>()).collect::<Vec<_>>();
    URL_SAFE_NO_PAD.encode(data)
}

fn valid_session_id(value: &str) -> bool {
    (40..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn origin_of(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    Some(format!("{}://{}", url.scheme(), url.host_str()?))
}

fn demo_profile() -> CanvasProfile {
    CanvasProfile {
        id: "20260001".to_string(),
        name: "演示用户".to_string(),
        short_name: "演示用户".to_string(),
        avatar_url: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::SocketAddr, time::Duration};

    #[tokio::test]
    async fn late_unauthorized_cannot_clear_new_login_or_repeat_invalidation() {
        let old_jar = Arc::new(Jar::default());
        let session = UserSession::new("test".into(), old_jar.clone(), Some(demo_profile()), 0);
        assert!(session.clear_auth_if_current(&old_jar).await);
        assert!(!session.clear_auth_if_current(&old_jar).await);
        let new_jar = Arc::new(Jar::default());
        session.replace_auth(new_jar.clone(), demo_profile()).await;
        assert!(!session.clear_auth_if_current(&old_jar).await);
        assert!(session.is_authenticated());
        assert!(Arc::ptr_eq(&session.jar().await, &new_jar));
    }

    fn test_config(directory: &Path, session_ttl: Duration) -> Config {
        Config {
            bind: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            data_dir: directory.to_path_buf(),
            web_dist: directory.to_path_buf(),
            public_url: None,
            app_secret: Some("test-only-secret".into()),
            cookie_secure: false,
            session_ttl,
            ticket_ttl: Duration::from_secs(900),
            canvas_origin: "https://oc.sjtu.edu.cn".into(),
            courses_origin: "https://courses.sjtu.edu.cn".into(),
            video_api: "https://v.sjtu.edu.cn/jy-application-canvas-sjtu".into(),
            video_lti_adapter: "https://v.sjtu.edu.cn/jy-lti-adapter".into(),
            resource_video_api: "https://v.sjtu.edu.cn/jy-application-resourcemanage".into(),
            jaccount_origin: "https://jaccount.sjtu.edu.cn".into(),
            demo_mode: false,
            proxy_concurrency: 2,
        }
    }

    #[test]
    fn generated_session_ids_are_cookie_safe() {
        let id = random_token(32);
        assert!(valid_session_id(&id));
        assert!(!id.contains('='));
    }

    #[test]
    fn cookie_snapshot_round_trip() {
        let jar = Jar::default();
        let origin = Url::parse("https://oc.sjtu.edu.cn").unwrap();
        jar.add_cookie_str("canvas_session=secret; Path=/; Secure; HttpOnly", &origin);
        let snapshot = snapshot_cookies(&jar, &[origin.to_string()]);
        let restored = restore_cookies(&snapshot);
        let header = restored.cookies(&origin).unwrap();
        assert!(header.to_str().unwrap().contains("canvas_session=secret"));
    }

    #[tokio::test]
    async fn encrypted_sessions_survive_restart_without_jaccount_cookies() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path(), Duration::from_secs(86_400));
        let store = SessionStore::open(&config).await.unwrap();
        let (session, _) = store.ensure(None).await;
        let jar = Arc::new(Jar::default());
        let canvas_url = Url::parse(&config.canvas_origin).unwrap();
        let jaccount_url = Url::parse(&config.jaccount_origin).unwrap();
        jar.add_cookie_str("canvas_session=canvas-secret; Path=/; Secure", &canvas_url);
        jar.add_cookie_str("JSESSIONID=jaccount-secret; Path=/; Secure", &jaccount_url);
        store
            .complete_login(
                &session.id,
                jar,
                CanvasProfile {
                    id: "42".into(),
                    name: "测试用户".into(),
                    short_name: "测试".into(),
                    avatar_url: None,
                },
            )
            .await
            .unwrap();
        drop(store);

        let restored = SessionStore::open(&config).await.unwrap();
        let (_, jar, profile) = restored.authenticated(&session.id).await.unwrap();
        assert_eq!(profile.id, "42");
        assert!(
            jar.cookies(&canvas_url)
                .unwrap()
                .to_str()
                .unwrap()
                .contains("canvas_session=canvas-secret")
        );
        assert!(jar.cookies(&jaccount_url).is_none());
    }

    #[tokio::test]
    async fn expired_session_id_is_rotated_and_cannot_be_replayed() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path(), Duration::from_secs(60));
        let store = SessionStore::open(&config).await.unwrap();
        let (expired, _) = store.ensure(None).await;
        expired
            .replace_auth(Arc::new(Jar::default()), demo_profile())
            .await;
        expired
            .last_seen
            .store(Utc::now().timestamp() - 61, Ordering::Relaxed);

        let (replacement, created) = store.ensure(Some(&expired.id)).await;

        assert!(created);
        assert_ne!(replacement.id, expired.id);
        assert!(matches!(
            store.get(&expired.id).await,
            Err(AppError::Unauthorized)
        ));
    }

    #[tokio::test]
    async fn anonymous_capacity_never_evicts_an_authenticated_session() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path(), Duration::from_secs(86_400));
        let store = SessionStore::open(&config).await.unwrap();
        let (authenticated, _) = store.ensure(None).await;
        authenticated
            .replace_auth(Arc::new(Jar::default()), demo_profile())
            .await;

        for _ in 0..(MAX_ANONYMOUS_SESSIONS + 32) {
            store.ensure(None).await;
        }

        let sessions = store.sessions.read().await;
        let anonymous_count = sessions
            .values()
            .filter(|session| !session.is_authenticated())
            .count();
        assert_eq!(anonymous_count, MAX_ANONYMOUS_SESSIONS);
        assert!(sessions.contains_key(&authenticated.id));
        assert!(authenticated.is_authenticated());
    }
}
