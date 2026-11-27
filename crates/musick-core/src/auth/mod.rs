pub mod browser;
pub mod oauth;
pub mod secrets;

use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::RequestBuilder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::config::AuthMode;
use crate::error::{CoreError, CoreResult};

pub use browser::{
    BrowserCredentials, YTM_ORIGIN, compute_sapisidhash, extract_sapisid, parse_browser_headers,
};
pub use oauth::{
    GOOGLE_DEVICE_CODE_URL, GOOGLE_TOKEN_URL, HttpOAuthTransport, OAuthCredentials,
    OAuthDevicePrompt, OAuthPollResponse, OAuthPollStatus, OAuthTransport, TokenRefreshResponse,
    YOUTUBE_OAUTH_SCOPE,
};
pub use secrets::{KeyringSecretStore, MemorySecretStore, SecretStore};

const SECRET_CREDENTIALS_KEY: &str = "auth_credentials_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum StoredCredentials {
    OAuth(OAuthCredentials),
    Browser(BrowserCredentials),
}

#[derive(Debug, Clone, Default)]
enum AuthState {
    #[default]
    Anonymous,
    OAuth(OAuthCredentials),
    Browser(BrowserCredentials),
}

pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hash_account_key(prefix: &str, seed: &str, page_id: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(b":");
    hasher.update(seed.as_bytes());
    if let Some(pid) = page_id.filter(|s| !s.trim().is_empty()) {
        hasher.update(b":page:");
        hasher.update(pid.trim().as_bytes());
    }
    let hex_str = hex::encode(hasher.finalize());
    format!("ytm:{}", &hex_str[..16])
}

pub struct AuthManager {
    store: Arc<dyn SecretStore>,
    oauth_transport: Arc<dyn OAuthTransport>,
    state: RwLock<AuthState>,
    refresh_lock: Mutex<()>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl AuthManager {
    pub fn new_anonymous(store: Arc<dyn SecretStore>, http: reqwest::Client) -> Self {
        Self {
            store,
            oauth_transport: Arc::new(HttpOAuthTransport::new(http)),
            state: RwLock::new(AuthState::Anonymous),
            refresh_lock: Mutex::new(()),
            clock: Arc::new(now_unix_secs),
        }
    }

    pub fn with_transport(
        store: Arc<dyn SecretStore>,
        oauth_transport: Arc<dyn OAuthTransport>,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Self {
        Self {
            store,
            oauth_transport,
            state: RwLock::new(AuthState::Anonymous),
            refresh_lock: Mutex::new(()),
            clock,
        }
    }

    /// load persisted credentials only when oauth or browser mode is explicitly configured
    pub fn load_for_mode(&self, mode: AuthMode) -> CoreResult<AuthMode> {
        if mode == AuthMode::Anonymous {
            let mut guard = self
                .state
                .write()
                .map_err(|_| CoreError::Auth("AuthManager state lock poisoned".into()))?;
            *guard = AuthState::Anonymous;
            return Ok(AuthMode::Anonymous);
        }

        let Some(raw) = self.store.get(SECRET_CREDENTIALS_KEY)? else {
            return Ok(AuthMode::Anonymous);
        };

        let parsed: StoredCredentials = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => return Ok(AuthMode::Anonymous),
        };

        let mut guard = self
            .state
            .write()
            .map_err(|_| CoreError::Auth("AuthManager state lock poisoned".into()))?;
        match (mode, parsed) {
            (AuthMode::OAuth, StoredCredentials::OAuth(creds)) => {
                *guard = AuthState::OAuth(creds);
                Ok(AuthMode::OAuth)
            }
            (AuthMode::Browser, StoredCredentials::Browser(creds)) => {
                *guard = AuthState::Browser(creds);
                Ok(AuthMode::Browser)
            }
            _ => {
                *guard = AuthState::Anonymous;
                Ok(AuthMode::Anonymous)
            }
        }
    }

    pub fn mode(&self) -> AuthMode {
        match self.state.read() {
            Ok(guard) => match &*guard {
                AuthState::Anonymous => AuthMode::Anonymous,
                AuthState::OAuth(_) => AuthMode::OAuth,
                AuthState::Browser(_) => AuthMode::Browser,
            },
            Err(_) => AuthMode::Anonymous,
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.mode().is_authenticated()
    }

    pub fn account_key(&self) -> String {
        let Ok(guard) = self.state.read() else {
            return "local".to_string();
        };
        match &*guard {
            AuthState::Anonymous => "local".to_string(),
            AuthState::OAuth(c) => {
                let seed = if !c.refresh_token.is_empty() {
                    c.refresh_token.as_str()
                } else {
                    c.access_token.as_str()
                };
                hash_account_key("oauth", seed, c.page_id.as_deref())
            }
            AuthState::Browser(c) => {
                let seed = c.sapisid().unwrap_or_else(|| c.cookie.clone());
                let full_seed = format!("{}:{}", c.auth_user, seed);
                hash_account_key("browser", &full_seed, c.page_id.as_deref())
            }
        }
    }

    pub fn account_label(&self) -> Option<String> {
        let guard = self.state.read().ok()?;
        match &*guard {
            AuthState::Anonymous => None,
            AuthState::OAuth(c) => c.account_label.clone(),
            AuthState::Browser(c) => c.account_label.clone(),
        }
    }

    /// returns brand account `onBehalfOfUser` / `X-Goog-PageId` in both oauth and browser modes
    pub fn on_behalf_of_user(&self) -> Option<String> {
        let guard = self.state.read().ok()?;
        match &*guard {
            AuthState::Anonymous => None,
            AuthState::OAuth(c) => c
                .page_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            AuthState::Browser(c) => c
                .page_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        }
    }

    pub fn set_oauth_credentials(&self, creds: OAuthCredentials) -> CoreResult<()> {
        let serialized = serde_json::to_string(&StoredCredentials::OAuth(creds.clone()))?;
        self.store.set(SECRET_CREDENTIALS_KEY, &serialized)?;
        let mut guard = self
            .state
            .write()
            .map_err(|_| CoreError::Auth("AuthManager state lock poisoned".into()))?;
        *guard = AuthState::OAuth(creds);
        Ok(())
    }

    pub fn import_browser_headers(
        &self,
        raw: &str,
        page_id_override: Option<String>,
    ) -> CoreResult<BrowserCredentials> {
        let mut creds = parse_browser_headers(raw)?;
        if let Some(pid) = page_id_override
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            creds.page_id = Some(pid.to_string());
        }
        let serialized = serde_json::to_string(&StoredCredentials::Browser(creds.clone()))?;
        self.store.set(SECRET_CREDENTIALS_KEY, &serialized)?;
        let mut guard = self
            .state
            .write()
            .map_err(|_| CoreError::Auth("AuthManager state lock poisoned".into()))?;
        *guard = AuthState::Browser(creds.clone());
        Ok(creds)
    }

    pub async fn start_oauth_device_flow(&self, client_id: &str) -> CoreResult<OAuthDevicePrompt> {
        if client_id.trim().is_empty() {
            return Err(CoreError::InvalidInput(
                "OAuth client_id cannot be empty".into(),
            ));
        }
        self.oauth_transport
            .request_device_code(client_id.trim(), YOUTUBE_OAUTH_SCOPE)
            .await
    }

    pub async fn poll_oauth_device_token(
        &self,
        client_id: &str,
        client_secret: &str,
        device_code: &str,
        page_id: Option<String>,
    ) -> CoreResult<OAuthPollStatus> {
        match self
            .oauth_transport
            .poll_device_token(client_id.trim(), client_secret.trim(), device_code.trim())
            .await?
        {
            OAuthPollResponse::Pending => Ok(OAuthPollStatus::Pending),
            OAuthPollResponse::SlowDown => Ok(OAuthPollStatus::SlowDown),
            OAuthPollResponse::Token(tok) => {
                let now = (self.clock)();
                let creds = OAuthCredentials {
                    client_id: client_id.trim().to_string(),
                    client_secret: client_secret.trim().to_string(),
                    access_token: tok.access_token,
                    refresh_token: tok.refresh_token.unwrap_or_default(),
                    expires_at: now.saturating_add(tok.expires_in),
                    scope: tok.scope.unwrap_or_else(|| YOUTUBE_OAUTH_SCOPE.to_string()),
                    token_type: tok.token_type.unwrap_or_else(|| "Bearer".to_string()),
                    page_id: page_id
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                    account_label: None,
                };
                self.set_oauth_credentials(creds.clone())?;
                Ok(OAuthPollStatus::Authorized(creds))
            }
        }
    }

    /// apply auth headers to outgoing innertube requests (strictly no-op in anonymous mode)
    pub async fn apply_headers(&self, req: RequestBuilder) -> CoreResult<RequestBuilder> {
        let snapshot = {
            let guard = self
                .state
                .read()
                .map_err(|_| CoreError::Auth("AuthManager state lock poisoned".into()))?;
            guard.clone()
        };

        match snapshot {
            AuthState::Anonymous => Ok(req),
            AuthState::Browser(creds) => creds.apply_headers(req, (self.clock)()),
            AuthState::OAuth(creds) => {
                let fresh = self.ensure_fresh_oauth(creds).await?;
                let mut builder =
                    req.header("Authorization", format!("Bearer {}", fresh.access_token));
                if let Some(pid) = fresh
                    .page_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    builder = builder.header("X-Goog-PageId", pid);
                }
                Ok(builder)
            }
        }
    }

    async fn ensure_fresh_oauth(&self, current: OAuthCredentials) -> CoreResult<OAuthCredentials> {
        let now = (self.clock)();
        if !current.is_expired(now) {
            return Ok(current);
        }

        // Single-flight refresh lock so concurrent API calls only refresh once.
        let _refresh_guard = self.refresh_lock.lock().await;

        let latest = {
            let guard = self
                .state
                .read()
                .map_err(|_| CoreError::Auth("AuthManager state lock poisoned".into()))?;
            match &*guard {
                AuthState::OAuth(c) => c.clone(),
                _ => {
                    return Err(CoreError::Auth(
                        "Auth mode changed while refreshing OAuth token".into(),
                    ));
                }
            }
        };

        let now_after_lock = (self.clock)();
        if !latest.is_expired(now_after_lock) {
            return Ok(latest);
        }

        if latest.refresh_token.is_empty() {
            return Err(CoreError::Auth(
                "OAuth access token expired and no refresh_token is stored".into(),
            ));
        }

        let refreshed = self
            .oauth_transport
            .refresh_access_token(
                &latest.client_id,
                &latest.client_secret,
                &latest.refresh_token,
            )
            .await?;

        let updated = OAuthCredentials {
            access_token: refreshed.access_token,
            refresh_token: refreshed.refresh_token.unwrap_or(latest.refresh_token),
            expires_at: now_after_lock.saturating_add(refreshed.expires_in),
            scope: refreshed.scope.unwrap_or(latest.scope),
            token_type: refreshed.token_type.unwrap_or(latest.token_type),
            ..latest
        };

        self.set_oauth_credentials(updated.clone())?;
        Ok(updated)
    }

    pub fn logout(&self) -> CoreResult<()> {
        self.store.delete(SECRET_CREDENTIALS_KEY)?;
        let mut guard = self
            .state
            .write()
            .map_err(|_| CoreError::Auth("AuthManager state lock poisoned".into()))?;
        *guard = AuthState::Anonymous;
        Ok(())
    }
}
