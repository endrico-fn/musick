use reqwest::RequestBuilder;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::error::{CoreError, CoreResult};

pub const YTM_ORIGIN: &str = "https://music.youtube.com";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserCredentials {
    pub cookie: String,
    #[serde(default = "default_auth_user")]
    pub auth_user: String,
    #[serde(default)]
    pub page_id: Option<String>,
    #[serde(default = "default_origin")]
    pub origin: String,
    #[serde(default)]
    pub account_label: Option<String>,
}

fn default_auth_user() -> String {
    "0".to_string()
}

fn default_origin() -> String {
    YTM_ORIGIN.to_string()
}

impl BrowserCredentials {
    pub fn sapisid(&self) -> Option<String> {
        extract_sapisid(&self.cookie)
    }

    pub fn sapisidhash(&self, timestamp_secs: u64) -> CoreResult<String> {
        let sapisid = self.sapisid().ok_or_else(|| {
            CoreError::Auth("Browser cookie missing SAPISID or __Secure-3PAPISID".into())
        })?;
        Ok(compute_sapisidhash(&sapisid, &self.origin, timestamp_secs))
    }

    pub fn apply_headers(
        &self,
        mut req: RequestBuilder,
        timestamp_secs: u64,
    ) -> CoreResult<RequestBuilder> {
        let auth_header = self.sapisidhash(timestamp_secs)?;
        req = req
            .header("Cookie", &self.cookie)
            .header("Authorization", auth_header)
            .header("X-Origin", &self.origin)
            .header("Origin", &self.origin)
            .header("X-Goog-AuthUser", &self.auth_user);

        if let Some(page_id) = self.page_id.as_deref().filter(|s| !s.trim().is_empty()) {
            req = req.header("X-Goog-PageId", page_id.trim());
        }
        Ok(req)
    }
}

pub fn extract_sapisid(cookie_header: &str) -> Option<String> {
    let mut fallback_3p: Option<String> = None;
    for part in cookie_header.split(';') {
        let trimmed = part.trim();
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim();
        if val.is_empty() {
            continue;
        }
        if key == "SAPISID" {
            return Some(val.to_string());
        }
        if key == "__Secure-3PAPISID" {
            fallback_3p = Some(val.to_string());
        }
    }
    fallback_3p
}

pub fn compute_sapisidhash(sapisid: &str, origin: &str, timestamp_secs: u64) -> String {
    let input = format!("{timestamp_secs} {sapisid} {origin}");
    let mut hasher = Sha1::new();
    hasher.update(input.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("SAPISIDHASH {timestamp_secs}_{digest}")
}

/// Parse pasted browser DevTools request headers, cURL `-H` snippet, or a plain cookie string.
pub fn parse_browser_headers(raw: &str) -> CoreResult<BrowserCredentials> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidInput(
            "Empty browser headers/cookie input".into(),
        ));
    }

    let mut cookie: Option<String> = None;
    let mut auth_user = default_auth_user();
    let mut page_id: Option<String> = None;
    let mut origin = default_origin();

    for raw_line in trimmed.lines() {
        let mut line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        // Strip cURL -H / --header wrapper if user copied a cURL command.
        if let Some(rest) = line
            .strip_prefix("-H ")
            .or_else(|| line.strip_prefix("--header "))
        {
            line = rest.trim().trim_end_matches('\\').trim();
            line = line.trim_matches('\'').trim_matches('"');
        }
        if line.starts_with(':') {
            // HTTP/2 pseudo-header like `:authority:`
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let header_name = name.trim().to_ascii_lowercase();
        let header_val = value.trim().trim_matches('\'').trim_matches('"').trim();
        if header_val.is_empty() {
            continue;
        }
        match header_name.as_str() {
            "cookie" => cookie = Some(header_val.to_string()),
            "x-goog-authuser" => auth_user = header_val.to_string(),
            "x-goog-pageid" => page_id = Some(header_val.to_string()),
            "x-origin" | "origin" => origin = header_val.to_string(),
            _ => {}
        }
    }

    // If user pasted a plain cookie string (`SAPISID=...; __Secure-3PAPISID=...`) directly.
    let final_cookie = match cookie {
        Some(c) => c,
        None => {
            if extract_sapisid(trimmed).is_some() {
                trimmed.to_string()
            } else {
                return Err(CoreError::Auth(
                    "Could not find Cookie header containing SAPISID or __Secure-3PAPISID".into(),
                ));
            }
        }
    };

    if extract_sapisid(&final_cookie).is_none() {
        return Err(CoreError::Auth(
            "Cookie does not contain SAPISID or __Secure-3PAPISID".into(),
        ));
    }

    Ok(BrowserCredentials {
        cookie: final_cookie,
        auth_user,
        page_id,
        origin,
        account_label: None,
    })
}
