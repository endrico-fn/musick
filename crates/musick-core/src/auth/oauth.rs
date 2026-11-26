use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

pub const GOOGLE_DEVICE_CODE_URL: &str = "https://oauth2.googleapis.com/device/code";
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const YOUTUBE_OAUTH_SCOPE: &str = "https://www.googleapis.com/auth/youtube";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default = "default_token_type")]
    pub token_type: String,
    #[serde(default)]
    pub page_id: Option<String>,
    #[serde(default)]
    pub account_label: Option<String>,
}

fn default_scope() -> String {
    YOUTUBE_OAUTH_SCOPE.to_string()
}

fn default_token_type() -> String {
    "Bearer".to_string()
}

impl OAuthCredentials {
    pub fn is_expired(&self, now_secs: u64) -> bool {
        now_secs.saturating_add(90) >= self.expires_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthDevicePrompt {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthPollStatus {
    Pending,
    SlowDown,
    Authorized(OAuthCredentials),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRefreshResponse {
    pub access_token: String,
    pub expires_in: u64,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
}

#[async_trait]
pub trait OAuthTransport: Send + Sync {
    async fn request_device_code(
        &self,
        client_id: &str,
        scope: &str,
    ) -> CoreResult<OAuthDevicePrompt>;

    async fn poll_device_token(
        &self,
        client_id: &str,
        client_secret: &str,
        device_code: &str,
    ) -> CoreResult<OAuthPollResponse>;

    async fn refresh_access_token(
        &self,
        client_id: &str,
        client_secret: &str,
        refresh_token: &str,
    ) -> CoreResult<TokenRefreshResponse>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthPollResponse {
    Pending,
    SlowDown,
    Token(TokenRefreshResponse),
}

pub struct HttpOAuthTransport {
    client: reqwest::Client,
}

impl HttpOAuthTransport {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[derive(Debug, Deserialize)]
struct RawDeviceCodeResponse {
    device_code: String,
    user_code: String,
    #[serde(alias = "verification_uri")]
    verification_url: String,
    expires_in: u64,
    #[serde(default = "default_interval")]
    interval: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Debug, Deserialize)]
struct RawErrorPayload {
    error: Option<String>,
    error_description: Option<String>,
}

#[async_trait]
impl OAuthTransport for HttpOAuthTransport {
    async fn request_device_code(
        &self,
        client_id: &str,
        scope: &str,
    ) -> CoreResult<OAuthDevicePrompt> {
        let resp = self
            .client
            .post(GOOGLE_DEVICE_CODE_URL)
            .form(&[("client_id", client_id), ("scope", scope)])
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(CoreError::Auth(format!(
                "OAuth device code request failed ({status}): {text}"
            )));
        }

        let raw: RawDeviceCodeResponse = resp.json().await?;
        Ok(OAuthDevicePrompt {
            device_code: raw.device_code,
            user_code: raw.user_code,
            verification_url: raw.verification_url,
            expires_in: raw.expires_in,
            interval: raw.interval.max(1),
        })
    }

    async fn poll_device_token(
        &self,
        client_id: &str,
        client_secret: &str,
        device_code: &str,
    ) -> CoreResult<OAuthPollResponse> {
        let resp = self
            .client
            .post(GOOGLE_TOKEN_URL)
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if status.is_success() {
            let token: TokenRefreshResponse = serde_json::from_str(&body)?;
            return Ok(OAuthPollResponse::Token(token));
        }

        if let Ok(err) = serde_json::from_str::<RawErrorPayload>(&body) {
            match err.error.as_deref() {
                Some("authorization_pending") => return Ok(OAuthPollResponse::Pending),
                Some("slow_down") => return Ok(OAuthPollResponse::SlowDown),
                Some(code) => {
                    let desc = err.error_description.unwrap_or_default();
                    return Err(CoreError::Auth(format!("OAuth error {code}: {desc}")));
                }
                None => {}
            }
        }

        Err(CoreError::Auth(format!(
            "OAuth token poll failed ({status}): {body}"
        )))
    }

    async fn refresh_access_token(
        &self,
        client_id: &str,
        client_secret: &str,
        refresh_token: &str,
    ) -> CoreResult<TokenRefreshResponse> {
        let resp = self
            .client
            .post(GOOGLE_TOKEN_URL)
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("refresh_token", refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(CoreError::Auth(format!(
                "OAuth token refresh failed ({status}): {text}"
            )));
        }

        let token: TokenRefreshResponse = resp.json().await?;
        Ok(token)
    }
}
