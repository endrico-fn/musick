use std::fmt;

use reqwest::RequestBuilder;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::error::{CoreError, CoreResult};

pub const YTM_ORIGIN: &str = "https://music.youtube.com";
const ALLOWED_ORIGINS: &[&str] = &[
    "https://music.youtube.com",
    "https://www.youtube.com",
    "https://youtube.com",
];
const MAX_COOKIE_BYTES: usize = 32_768;
const MAX_HEADER_BYTES: usize = 512;

fn sanitize_origin(candidate: &str) -> String {
    let trimmed = candidate.trim().trim_end_matches('/');
    if ALLOWED_ORIGINS
        .iter()
        .any(|allowed| trimmed.eq_ignore_ascii_case(allowed))
    {
        trimmed.to_ascii_lowercase()
    } else {
        YTM_ORIGIN.to_string()
    }
}

fn sanitize_header_value(raw: &str, max_len: usize) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > max_len {
        return None;
    }
    if trimmed.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return None;
    }
    Some(trimmed.to_string())
}

fn sanitize_auth_user(raw: &str) -> String {
    let trimmed = raw.trim();
    if !trimmed.is_empty() && trimmed.len() <= 3 && trimmed.bytes().all(|b| b.is_ascii_digit()) {
        trimmed.to_string()
    } else {
        default_auth_user()
    }
}

fn sanitize_page_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if !trimmed.is_empty()
        && trimmed.len() <= 64
        && trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn validate_cookie_header(raw: &str) -> CoreResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CoreError::Auth("Empty Cookie header".into()));
    }
    if trimmed.len() > MAX_COOKIE_BYTES {
        return Err(CoreError::Auth(
            "Cookie header exceeds maximum allowed size".into(),
        ));
    }
    if trimmed.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Err(CoreError::Auth(
            "Cookie header contains invalid control characters or newlines".into(),
        ));
    }
    if extract_sapisid(trimmed).is_none() {
        return Err(CoreError::Auth(
            "Cookie does not contain SAPISID or __Secure-3PAPISID".into(),
        ));
    }
    Ok(trimmed.to_string())
}

fn split_shell_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for ch in input.chars() {
        if escaped {
            cur.push(ch);
            escaped = false;
            continue;
        }
        if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                cur.push(ch);
            }
            continue;
        }
        if in_double {
            if ch == '"' {
                in_double = false;
            } else if ch == '\\' {
                escaped = true;
            } else {
                cur.push(ch);
            }
            continue;
        }
        match ch {
            '\'' => in_single = true,
            '"' => in_double = true,
            '\\' => escaped = true,
            c if c.is_whitespace() => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

struct HeaderAcc {
    cookie: Option<String>,
    auth_user: String,
    page_id: Option<String>,
    origin: String,
    visitor_id: Option<String>,
    user_agent: Option<String>,
}

fn apply_header_pair(raw_pair: &str, acc: &mut HeaderAcc) {
    let pair = raw_pair.trim();
    if pair.starts_with(':') {
        // http/2 pseudo-header like `:authority:`
        return;
    }
    let Some((name, value)) = pair.split_once(':') else {
        return;
    };
    let header_name = name.trim().to_ascii_lowercase();
    let header_val = value.trim().trim_matches('\'').trim_matches('"').trim();
    if header_val.is_empty() {
        return;
    }
    match header_name.as_str() {
        "cookie" => {
            if let Some(clean) = sanitize_header_value(header_val, MAX_COOKIE_BYTES) {
                acc.cookie = Some(clean);
            }
        }
        "x-goog-authuser" => acc.auth_user = sanitize_auth_user(header_val),
        "x-goog-pageid" => acc.page_id = sanitize_page_id(header_val),
        "x-goog-visitor-id" => {
            acc.visitor_id = sanitize_header_value(header_val, MAX_HEADER_BYTES);
        }
        "user-agent" => {
            acc.user_agent = sanitize_header_value(header_val, MAX_HEADER_BYTES);
        }
        "x-origin" | "origin" => acc.origin = sanitize_origin(header_val),
        _ => {}
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserCredentials {
    pub cookie: String,
    #[serde(default = "default_auth_user")]
    pub auth_user: String,
    #[serde(default)]
    pub page_id: Option<String>,
    #[serde(default = "default_origin")]
    pub origin: String,
    #[serde(default)]
    pub visitor_id: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub account_label: Option<String>,
}

impl fmt::Debug for BrowserCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserCredentials")
            .field("cookie", &"[REDACTED]")
            .field("auth_user", &self.auth_user)
            .field("page_id", &self.page_id)
            .field("origin", &self.origin)
            .field("has_visitor_id", &self.visitor_id.is_some())
            .field("has_user_agent", &self.user_agent.is_some())
            .finish()
    }
}

fn default_auth_user() -> String {
    "0".to_string()
}

fn default_origin() -> String {
    YTM_ORIGIN.to_string()
}

fn extract_named_cookie(cookie_header: &str, target: &str) -> Option<String> {
    for part in cookie_header.split(';') {
        let trimmed = part.trim();
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim();
        if key == target && !val.is_empty() {
            return Some(val.to_string());
        }
    }
    None
}

fn hash_scheme(scheme: &str, token: &str, origin: &str, timestamp_secs: u64) -> String {
    let input = format!("{timestamp_secs} {token} {origin}");
    let mut hasher = Sha1::new();
    hasher.update(input.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("{scheme} {timestamp_secs}_{digest}")
}

impl BrowserCredentials {
    pub fn sapisid(&self) -> Option<String> {
        extract_sapisid(&self.cookie)
    }

    pub fn sapisidhash(&self, timestamp_secs: u64) -> CoreResult<String> {
        let sapisid = self.sapisid().ok_or_else(|| {
            CoreError::Auth("Browser cookie missing SAPISID or __Secure-3PAPISID".into())
        })?;
        let origin = sanitize_origin(&self.origin);
        Ok(compute_sapisidhash(&sapisid, &origin, timestamp_secs))
    }

    pub fn authorization_header(&self, timestamp_secs: u64) -> CoreResult<String> {
        let origin = sanitize_origin(&self.origin);
        let primary = self.sapisid().ok_or_else(|| {
            CoreError::Auth("Browser cookie missing SAPISID or __Secure-3PAPISID".into())
        })?;
        let mut parts = vec![compute_sapisidhash(&primary, &origin, timestamp_secs)];
        if let Some(p1) = extract_named_cookie(&self.cookie, "__Secure-1PAPISID") {
            parts.push(hash_scheme("SAPISID1PHASH", &p1, &origin, timestamp_secs));
        }
        if let Some(p3) = extract_named_cookie(&self.cookie, "__Secure-3PAPISID") {
            parts.push(hash_scheme("SAPISID3PHASH", &p3, &origin, timestamp_secs));
        }
        Ok(parts.join(" "))
    }

    pub fn apply_headers(
        &self,
        mut req: RequestBuilder,
        timestamp_secs: u64,
    ) -> CoreResult<RequestBuilder> {
        let clean_cookie = validate_cookie_header(&self.cookie)?;
        let origin = sanitize_origin(&self.origin);
        let auth_user = sanitize_auth_user(&self.auth_user);
        let auth_header = self.authorization_header(timestamp_secs)?;
        req = req
            .header("Cookie", clean_cookie)
            .header("Authorization", auth_header)
            .header("X-Origin", &origin)
            .header("Origin", &origin)
            .header("X-Goog-AuthUser", auth_user)
            .header("X-Youtube-Bootstrap-Logged-In", "true");

        if let Some(page_id) = self.page_id.as_deref().and_then(sanitize_page_id) {
            req = req.header("X-Goog-PageId", page_id);
        }
        Ok(req)
    }
}

pub fn extract_sapisid(cookie_header: &str) -> Option<String> {
    extract_named_cookie(cookie_header, "SAPISID")
        .or_else(|| extract_named_cookie(cookie_header, "__Secure-3PAPISID"))
}

pub fn compute_sapisidhash(sapisid: &str, origin: &str, timestamp_secs: u64) -> String {
    hash_scheme("SAPISIDHASH", sapisid, origin, timestamp_secs)
}

/// parse pasted browser devtools request headers, curl `-H`/`-b`/`--cookie` snippet, or a plain cookie string
pub fn parse_browser_headers(raw: &str) -> CoreResult<BrowserCredentials> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidInput(
            "Empty browser headers/cookie input".into(),
        ));
    }

    let mut acc = HeaderAcc {
        cookie: None,
        auth_user: default_auth_user(),
        page_id: None,
        origin: default_origin(),
        visitor_id: None,
        user_agent: None,
    };

    for raw_line in trimmed.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let is_curl_line = line.starts_with("curl ")
            || line.starts_with("-H ")
            || line.starts_with("--header ")
            || line.starts_with("-b ")
            || line.starts_with("--cookie ");
        if is_curl_line {
            let tokens = split_shell_tokens(line.trim_end_matches('\\').trim());
            let mut i = 0;
            while i < tokens.len() {
                match tokens[i].as_str() {
                    "-b" | "--cookie" => {
                        if let Some(val) = tokens.get(i + 1)
                            && let Some(clean) = sanitize_header_value(val, MAX_COOKIE_BYTES)
                        {
                            acc.cookie = Some(clean);
                            i += 2;
                            continue;
                        }
                    }
                    "-H" | "--header" => {
                        if let Some(val) = tokens.get(i + 1) {
                            apply_header_pair(val, &mut acc);
                            i += 2;
                            continue;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            continue;
        }
        apply_header_pair(line, &mut acc);
    }

    let candidate_cookie = match acc.cookie {
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

    let final_cookie = validate_cookie_header(&candidate_cookie)?;

    Ok(BrowserCredentials {
        cookie: final_cookie,
        auth_user: acc.auth_user,
        page_id: acc.page_id,
        origin: acc.origin,
        visitor_id: acc.visitor_id,
        user_agent: acc.user_agent,
        account_label: None,
    })
}
