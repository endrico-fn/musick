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
