#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use tempfile::tempdir;

use musick_core::{
    AppPaths, AuthMode, CoverSize, ImageProtocol, Settings, Thumb, atomic_write_text,
    best_thumb_url, clean_media_title, format_duration,
};

#[test]
fn privacy_and_cache_defaults_match_policy() {
    let settings = Settings::default();
    assert_eq!(settings.privacy.auth_mode, AuthMode::Anonymous);
    assert!(!settings.privacy.save_history);
    assert!(!settings.privacy.report_playback_history);
    assert!(settings.privacy.remote_suggestions);

    assert!(settings.cache.enabled);
    assert_eq!(settings.cache.images_max_mib, 256);
    assert_eq!(settings.cache.images_max_files, 2048);
    assert_eq!(settings.cache.lyrics_max_mib, 32);
    assert_eq!(settings.cache.catalog_max_mib, 64);

    assert_eq!(settings.player.volume, 80);
    assert_eq!(settings.player.cache_secs, 30);
    assert!(!settings.player.use_mpv_config);
}

#[test]
fn legacy_settings_toml_ignores_unknown_keys_and_aliases() {
    let legacy = r#"
[privacy]
save_history = true
cache_images = false
cache_lyrics = false
remote_suggestions = false
legacy_unused_key = "ignored"

[display]
cover_size = "L"
image_protocol = "tgp"
region = "US"

[player]
volume = 250
cache_secs = 2
"#;

    let loaded = Settings::from_toml_str(legacy);
    assert_eq!(loaded.privacy.auth_mode, AuthMode::Anonymous);
    assert!(loaded.privacy.save_history);
    assert!(!loaded.privacy.remote_suggestions);
    assert!(!loaded.privacy.report_playback_history);
    assert_eq!(loaded.display.cover_size, CoverSize::L);
    assert_eq!(loaded.display.image_protocol, ImageProtocol::Kitty);
    assert_eq!(loaded.display.region, "US");
    assert_eq!(loaded.player.volume, 130);
    assert_eq!(loaded.player.cache_secs, 5);
}

#[test]
fn normalize_locks_telemetry_off_when_anonymous_and_clamps_bounds() {
    let mut settings = Settings::default();
    settings.privacy.auth_mode = AuthMode::Anonymous;
    settings.privacy.report_playback_history = true;
    settings.player.volume = 199;
    settings.player.cache_secs = 999;
    settings.display.region = "INVALID".into();
    settings.normalize();

    assert!(!settings.privacy.report_playback_history);
    assert_eq!(settings.player.volume, 130);
    assert_eq!(settings.player.cache_secs, 300);
    assert_eq!(settings.display.region, "ID");
}

#[test]
fn settings_roundtrip_and_atomic_write_permissions() {
    let dir = tempdir().unwrap();
    let paths = AppPaths::from_roots(
        dir.path().join("cfg"),
        dir.path().join("data"),
        dir.path().join("cache"),
        dir.path().join("rt"),
    );

    let mut s = Settings::default();
    s.player.volume = 115;
    s.lyrics.translate_to_id = true;
    s.save(&paths.settings_file()).unwrap();

    let reloaded = Settings::load(&paths.settings_file());
    assert_eq!(reloaded.player.volume, 115);
    assert!(reloaded.lyrics.translate_to_id);

    let secret_file = paths.secrets_fallback_file();
    atomic_write_text(&secret_file, "{\"secret\":true}").unwrap();
    assert_eq!(fs::read_to_string(&secret_file).unwrap(), "{\"secret\":true}");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&secret_file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn best_thumb_url_upscales_google_cdn_and_preserves_crop() {
    let thumbs = vec![
        Thumb {
            url: "https://lh3.googleusercontent.com/abc=w60-h60-l90-rj".into(),
            width: 60,
            height: 60,
        },
        Thumb {
            url: "https://lh3.googleusercontent.com/abc=w226-h226-p-l90-rj".into(),
            width: 226,
            height: 226,
        },
    ];
    let picked = best_thumb_url(&thumbs, 300).unwrap();
    assert_eq!(
        picked,
        "https://lh3.googleusercontent.com/abc=w300-h300-p-l90-rj"
    );

    assert_eq!(best_thumb_url(&[], 300), None);
}

#[test]
fn clean_media_title_and_duration_formatting() {
    assert_eq!(
        clean_media_title("Starboy (feat. Daft Punk) [Official Video]", true),
        "Starboy"
    );
    assert_eq!(
        clean_media_title("Starboy (feat. Daft Punk) [Official Video]", false),
        "Starboy (feat. Daft Punk)"
    );
    assert_eq!(clean_media_title("(Official Audio)", true), "(Official Audio)");
    assert_eq!(format_duration(None), "--:--");
    assert_eq!(format_duration(Some(0)), "--:--");
    assert_eq!(format_duration(Some(65)), "1:05");
    assert_eq!(format_duration(Some(3661)), "1:01:01");
}

mod auth_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use sha1::{Digest, Sha1};
    use tempfile::tempdir;

    use musick_core::auth::{
        AuthManager, KeyringSecretStore, MemorySecretStore, OAuthCredentials, OAuthDevicePrompt,
        OAuthPollResponse, OAuthPollStatus, OAuthTransport, SecretStore, TokenRefreshResponse,
        YTM_ORIGIN, compute_sapisidhash, parse_browser_headers,
    };
    use musick_core::{AuthMode, CoreResult};

    struct MockOAuthTransport {
        refresh_calls: AtomicUsize,
    }

    impl MockOAuthTransport {
        fn new() -> Self {
            Self {
                refresh_calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl OAuthTransport for MockOAuthTransport {
        async fn request_device_code(
            &self,
            _client_id: &str,
            _scope: &str,
        ) -> CoreResult<OAuthDevicePrompt> {
            Ok(OAuthDevicePrompt {
                device_code: "dev-123".into(),
                user_code: "ABCD-EFGH".into(),
                verification_url: "https://www.google.com/device".into(),
                expires_in: 1800,
                interval: 5,
            })
        }

        async fn poll_device_token(
            &self,
            _client_id: &str,
            _client_secret: &str,
            _device_code: &str,
        ) -> CoreResult<OAuthPollResponse> {
            Ok(OAuthPollResponse::Token(TokenRefreshResponse {
                access_token: "fresh-access".into(),
                expires_in: 3600,
                refresh_token: Some("fresh-refresh".into()),
                scope: None,
                token_type: Some("Bearer".into()),
            }))
        }

        async fn refresh_access_token(
            &self,
            _client_id: &str,
            _client_secret: &str,
            _refresh_token: &str,
        ) -> CoreResult<TokenRefreshResponse> {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            let n = self.refresh_calls.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(TokenRefreshResponse {
                access_token: format!("refreshed-token-{n}"),
                expires_in: 3600,
                refresh_token: None,
                scope: None,
                token_type: Some("Bearer".into()),
            })
        }
    }

    #[test]
    fn sapisidhash_matches_sha1_specification() {
        let ts = 1_700_000_000u64;
        let sapisid = "TEST_SAPISID_VALUE";
        let got = compute_sapisidhash(sapisid, YTM_ORIGIN, ts);

        let mut hasher = Sha1::new();
        hasher.update(format!("{ts} {sapisid} {YTM_ORIGIN}").as_bytes());
        let expected = format!("SAPISIDHASH {ts}_{}", hex::encode(hasher.finalize()));
        assert_eq!(got, expected);
    }

    #[test]
    fn parse_browser_headers_supports_devtools_curl_and_plain_cookie() {
        let devtools = "\
:authority: music.youtube.com
cookie: VISITOR_INFO1_LIVE=xyz; __Secure-3PAPISID=3p_secret_abc; SID=123
x-goog-authuser: 2
x-goog-pageid: 112233445566778899001
origin: https://music.youtube.com
";
        let creds = parse_browser_headers(devtools).unwrap();
        assert_eq!(creds.sapisid().as_deref(), Some("3p_secret_abc"));
        assert_eq!(creds.auth_user, "2");
        assert_eq!(creds.page_id.as_deref(), Some("112233445566778899001"));

        let plain = "HSID=aaa; SAPISID=direct_sapisid_99; SSID=bbb";
        let creds2 = parse_browser_headers(plain).unwrap();
        assert_eq!(creds2.sapisid().as_deref(), Some("direct_sapisid_99"));
        assert_eq!(creds2.auth_user, "0");
        assert!(creds2.page_id.is_none());

        assert!(parse_browser_headers("VISITOR_INFO1_LIVE=no_sapisid").is_err());
    }

    #[tokio::test]
    async fn anonymous_mode_attaches_zero_auth_or_cookie_headers() {
        let store = Arc::new(MemorySecretStore::new());
        let http = reqwest::Client::new();
        let auth = AuthManager::new_anonymous(store, http.clone());

        assert_eq!(auth.mode(), AuthMode::Anonymous);
        assert_eq!(auth.account_key(), "local");
        assert!(auth.on_behalf_of_user().is_none());

        let req = auth
            .apply_headers(http.post("https://music.youtube.com/youtubei/v1/browse"))
            .await
            .unwrap()
            .build()
            .unwrap();
        assert!(req.headers().get("authorization").is_none());
        assert!(req.headers().get("cookie").is_none());
        assert!(req.headers().get("x-goog-pageid").is_none());
    }

    #[tokio::test]
    async fn brand_account_headers_and_single_flight_oauth_refresh_and_logout() {
        let store = Arc::new(MemorySecretStore::new());
        let transport = Arc::new(MockOAuthTransport::new());
        let auth = Arc::new(AuthManager::with_transport(
            store.clone(),
            transport.clone(),
            Arc::new(|| 10_000),
        ));

        // 1. Browser mode with Brand Account page_id
        auth.import_browser_headers(
            "cookie: SAPISID=browser_tok\nx-goog-pageid: 987654321",
            None,
        )
        .unwrap();
        assert_eq!(auth.mode(), AuthMode::Browser);
        assert_eq!(auth.on_behalf_of_user().as_deref(), Some("987654321"));
        assert!(auth.account_key().starts_with("ytm:"));

        let http = reqwest::Client::new();
        let req = auth
            .apply_headers(http.post("https://music.youtube.com/youtubei/v1/browse"))
            .await
            .unwrap()
            .build()
            .unwrap();
        assert!(
            req.headers()
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("SAPISIDHASH 10000_")
        );
        assert_eq!(
            req.headers().get("x-goog-pageid").unwrap().to_str().unwrap(),
            "987654321"
        );
        assert!(req.headers().get("cookie").is_some());

        // 2. OAuth device poll + Brand Account page_id
        let poll = auth
            .poll_oauth_device_token("cid", "csec", "dev-123", Some("111222333".into()))
            .await
            .unwrap();
        assert!(matches!(poll, OAuthPollStatus::Authorized(_)));
        assert_eq!(auth.mode(), AuthMode::OAuth);
        assert_eq!(auth.on_behalf_of_user().as_deref(), Some("111222333"));

        // 3. Expired OAuth triggers exactly 1 refresh across 8 concurrent requests
        auth.set_oauth_credentials(OAuthCredentials {
            client_id: "cid".into(),
            client_secret: "csec".into(),
            access_token: "stale".into(),
            refresh_token: "ref".into(),
            expires_at: 9_999, // expired at now=10_000
            scope: "https://www.googleapis.com/auth/youtube".into(),
            token_type: "Bearer".into(),
            page_id: Some("111222333".into()),
            account_label: Some("Brand".into()),
        })
        .unwrap();

        let mut handles = Vec::new();
        for _ in 0..8 {
            let a = auth.clone();
            let c = http.clone();
            handles.push(tokio::spawn(async move {
                a.apply_headers(c.post("https://music.youtube.com/youtubei/v1/browse"))
                    .await
                    .unwrap()
                    .build()
                    .unwrap()
            }));
        }

        for h in handles {
            let built = h.await.unwrap();
            assert_eq!(
                built.headers().get("authorization").unwrap().to_str().unwrap(),
                "Bearer refreshed-token-1"
            );
            assert_eq!(
                built.headers().get("x-goog-pageid").unwrap().to_str().unwrap(),
                "111222333"
            );
            assert!(built.headers().get("cookie").is_none());
        }
        assert_eq!(transport.refresh_calls.load(Ordering::SeqCst), 1);

        // 4. Logout wipes secrets & resets cleanly to Anonymous
        auth.logout().unwrap();
        assert_eq!(auth.mode(), AuthMode::Anonymous);
        assert_eq!(auth.account_key(), "local");
        assert!(auth.on_behalf_of_user().is_none());
        assert!(store.get("auth_credentials_v1").unwrap().is_none());
    }

    #[test]
    fn file_fallback_secret_store_sets_0600_and_cleans_up_on_delete() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("credentials.json");
        let store = KeyringSecretStore::file_only(file.clone());
        store.set("k", "secret_val").unwrap();
        assert_eq!(store.get("k").unwrap().as_deref(), Some("secret_val"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        store.delete("k").unwrap();
        assert_eq!(store.get("k").unwrap(), None);
        assert!(!file.exists());
    }
}
