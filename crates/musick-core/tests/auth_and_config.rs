#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use tempfile::tempdir;

use musick_core::{
    AppPaths, AuthMode, CacheManager, CacheSettings, CavaAudioSource, ImageProtocol, Settings,
    Thumb, atomic_write_text, best_thumb_url, clean_media_title, format_duration,
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
cava_audio_source = "monitor"
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
    assert_eq!(loaded.display.image_protocol, ImageProtocol::Kitty);
    assert_eq!(
        loaded.display.cava_audio_source,
        CavaAudioSource::PulseMonitor
    );
    assert_eq!(loaded.display.region, "US");
    assert_eq!(loaded.player.volume, 130);
    assert_eq!(loaded.player.cache_secs, 5);

    // legacy cava_audio_source = "off" falls back cleanly to Auto
    let legacy_off = Settings::from_toml_str("[display]\ncava_audio_source = \"off\"\n");
    assert_eq!(legacy_off.display.cava_audio_source, CavaAudioSource::Auto);
}

#[test]
fn disabling_cache_bypasses_and_clears_l1_in_memory_cache() {
    let dir = tempdir().unwrap();
    let paths = AppPaths::from_roots(
        dir.path().join("cfg"),
        dir.path().join("data"),
        dir.path().join("cache"),
        dir.path().join("rt"),
    );
    let cache = CacheManager::open(&paths, CacheSettings::default()).unwrap();
    cache.put_catalog("k1", &"hello".to_string(), None);
    assert_eq!(cache.get_catalog::<String>("k1").as_deref(), Some("hello"));

    let disabled = CacheSettings {
        enabled: false,
        ..CacheSettings::default()
    };
    cache.update_settings(disabled);
    assert_eq!(cache.get_catalog::<String>("k1"), None);
    cache.put_catalog("k2", &"world".to_string(), None);
    assert_eq!(cache.get_catalog::<String>("k2"), None);
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
    assert_eq!(
        fs::read_to_string(&secret_file).unwrap(),
        "{\"secret\":true}"
    );

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
    assert_eq!(
        clean_media_title("(Official Audio)", true),
        "(Official Audio)"
    );
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
x-goog-visitor-id: CgtVISITOR_TEST_123
origin: https://music.youtube.com
";
        let creds = parse_browser_headers(devtools).unwrap();
        assert_eq!(creds.sapisid().as_deref(), Some("3p_secret_abc"));
        assert_eq!(creds.auth_user, "2");
        assert_eq!(creds.page_id.as_deref(), Some("112233445566778899001"));
        assert_eq!(creds.visitor_id.as_deref(), Some("CgtVISITOR_TEST_123"));

        let plain = "HSID=aaa; SAPISID=direct_sapisid_99; SSID=bbb";
        let creds2 = parse_browser_headers(plain).unwrap();
        assert_eq!(creds2.sapisid().as_deref(), Some("direct_sapisid_99"));
        assert_eq!(creds2.auth_user, "0");
        assert!(creds2.page_id.is_none());

        // Chrome "Copy as cURL" uses `-b '...'` / `--cookie '...'`, and untrusted origins are sanitized
        let chrome_curl = "\
curl 'https://music.youtube.com/youtubei/v1/browse' \\
  -H 'origin: https://evil.example' \\
  -b 'HSID=1; __Secure-3PAPISID=chrome_b_tok; SID=2' \\
  -H 'x-goog-authuser: 1'
";
        let creds3 = parse_browser_headers(chrome_curl).unwrap();
        assert_eq!(creds3.sapisid().as_deref(), Some("chrome_b_tok"));
        assert_eq!(creds3.auth_user, "1");
        assert_eq!(creds3.origin, YTM_ORIGIN);

        let single_line = "curl 'https://music.youtube.com/' --cookie 'SAPISID=single_tok; SID=x' -H 'origin: https://www.youtube.com'";
        let creds4 = parse_browser_headers(single_line).unwrap();
        assert_eq!(creds4.sapisid().as_deref(), Some("single_tok"));
        assert_eq!(creds4.origin, "https://www.youtube.com");

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
            req.headers()
                .get("x-goog-pageid")
                .unwrap()
                .to_str()
                .unwrap(),
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
                built
                    .headers()
                    .get("authorization")
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "Bearer refreshed-token-1"
            );
            assert_eq!(
                built
                    .headers()
                    .get("x-goog-pageid")
                    .unwrap()
                    .to_str()
                    .unwrap(),
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

    #[test]
    fn theme_accent_defaults_to_teal_and_legacy_cava_palette_toml_loads_cleanly() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("settings.toml");
        std::fs::write(
            &file,
            "[display]\ncava_palette = \"aurora\"\naccent = \"magenta\"\n",
        )
        .unwrap();
        let loaded = Settings::load(&file);
        assert_eq!(loaded.display.accent, musick_core::ThemeAccent::Magenta);
        assert_eq!(
            loaded.display.accent.toggle(),
            musick_core::ThemeAccent::Teal
        );
        assert_eq!(
            Settings::default().display.accent,
            musick_core::ThemeAccent::Teal
        );
    }

    #[test]
    fn credentials_redact_secrets_in_debug_and_reject_header_injection() {
        let raw = "cookie: SAPISID=top_secret_val_999\nuser-agent: Mozilla/5.0 CustomChrome/134.0";
        let creds = parse_browser_headers(raw).unwrap();
        assert_eq!(
            creds.user_agent.as_deref(),
            Some("Mozilla/5.0 CustomChrome/134.0")
        );
        let dbg_browser = format!("{creds:?}");
        assert!(!dbg_browser.contains("top_secret_val_999"));
        assert!(dbg_browser.contains("[REDACTED]"));

        let oauth = OAuthCredentials {
            client_id: "cid".into(),
            client_secret: "super_secret_client".into(),
            access_token: "super_secret_access".into(),
            refresh_token: "super_secret_refresh".into(),
            expires_at: 99999,
            scope: "yt".into(),
            token_type: "Bearer".into(),
            page_id: None,
            account_label: None,
        };
        let dbg_oauth = format!("{oauth:?}");
        assert!(!dbg_oauth.contains("super_secret_client"));
        assert!(!dbg_oauth.contains("super_secret_access"));
        assert!(!dbg_oauth.contains("super_secret_refresh"));

        let prompt = OAuthDevicePrompt {
            device_code: "secret_device_code".into(),
            user_code: "ABCD-EFGH".into(),
            verification_url: "https://www.google.com/device".into(),
            expires_in: 600,
            interval: 5,
        };
        let dbg_prompt = format!("{prompt:?}");
        assert!(!dbg_prompt.contains("secret_device_code"));
        assert!(dbg_prompt.contains("ABCD-EFGH"));

        // Reject control characters and oversized cookie headers
        assert!(parse_browser_headers("cookie: SAPISID=abc\x07def").is_err());
        let huge = format!("cookie: SAPISID={}", "a".repeat(40_000));
        assert!(parse_browser_headers(&huge).is_err());

        // Trusted Google URL validator blocks SSRF / lookalikes / HTTP
        use musick_core::innertube::endpoints::is_trusted_google_url;
        assert!(is_trusted_google_url(
            "https://s.youtube.com/api/stats/playback?docid=abc"
        ));
        assert!(is_trusted_google_url(
            "https://music.youtube.com/youtubei/v1/browse"
        ));
        assert!(!is_trusted_google_url(
            "http://s.youtube.com/api/stats/playback"
        ));
        assert!(!is_trusted_google_url("https://youtube.com@evil.com/steal"));
        assert!(!is_trusted_google_url("https://evil-youtube.com/steal"));
        assert!(!is_trusted_google_url(
            "https://evil.com/?https://youtube.com"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn file_fallback_secret_store_rejects_symlinks() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("real.txt");
        std::fs::write(&target, "hi").unwrap();
        let link = dir.path().join("credentials.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let store = KeyringSecretStore::file_only(link);
        assert!(store.set("k", "v").is_err());
        assert!(store.get("k").is_err());
    }
}
