use musick_core::AuthMode;

use crate::text::format_bytes;
use crate::views::RenderCtx;

pub(super) const AUTO_SYNC_STEPS: &[u64] = &[60, 180, 300, 600, 900];
pub(super) const CATALOG_TTL_STEPS: &[u64] = &[300, 600, 1800, 3600];
pub(super) const IMAGE_CAP_MIB_STEPS: &[u64] = &[64, 128, 256, 512, 1024];
pub(super) const CACHE_SECS_STEPS: &[u32] = &[15, 30, 60, 120];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsSection {
    #[default]
    All,
    Account,
    Privacy,
    Display,
    Player,
    Lyrics,
    Cache,
}

impl SettingsSection {
    pub const ALL: &'static [Self] = &[
        Self::All,
        Self::Account,
        Self::Privacy,
        Self::Display,
        Self::Player,
        Self::Lyrics,
        Self::Cache,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Account => "Account",
            Self::Privacy => "Privacy",
            Self::Display => "Display",
            Self::Player => "Player",
            Self::Lyrics => "Lyrics",
            Self::Cache => "Cache",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Account => "Acct",
            Self::Privacy => "Priv",
            Self::Display => "UI",
            Self::Player => "Play",
            Self::Lyrics => "Lyr",
            Self::Cache => "Cache",
        }
    }

    pub fn matches(self, row: SettingRow) -> bool {
        match self {
            Self::All => true,
            Self::Account => matches!(row.group(), "Account" | "Sync"),
            Self::Privacy => row.group() == "Privacy",
            Self::Display => matches!(row.group(), "Display" | "Cava"),
            Self::Player => row.group() == "Player",
            Self::Lyrics => row.group() == "Lyrics",
            Self::Cache => row.group() == "Cache",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingRow {
    AuthOAuth,
    AuthBrowser,
    Logout,
    SyncNow,
    SyncOnStartup,
    AutoSyncInterval,
    SaveHistory,
    ReportPlaybackHistory,
    RemoteSuggestions,
    ClearHistory,
    ThemeAccent,
    Region,
    IconMode,
    ImageProtocol,
    ReducedMotion,
    CavaEnabled,
    CavaSmoothing,
    CavaAudioSource,
    AudioQuality,
    NormalizeAudio,
    Autoplay,
    Gapless,
    Prefetch,
    CacheSecs,
    UseMpvConfig,
    MprisEnabled,
    LyricsEnabled,
    LyricsSyncedOnly,
    LyricsSource,
    LyricsTranslateId,
    CacheEnabled,
    CatalogTtl,
    ImageCacheMaxMib,
    PruneCache,
    ClearCache,
}

pub(super) const ALL_ROWS: &[SettingRow] = &[
    SettingRow::AuthOAuth,
    SettingRow::AuthBrowser,
    SettingRow::Logout,
    SettingRow::SyncNow,
    SettingRow::SyncOnStartup,
    SettingRow::AutoSyncInterval,
    SettingRow::SaveHistory,
    SettingRow::ReportPlaybackHistory,
    SettingRow::RemoteSuggestions,
    SettingRow::ClearHistory,
    SettingRow::ThemeAccent,
    SettingRow::Region,
    SettingRow::IconMode,
    SettingRow::ImageProtocol,
    SettingRow::ReducedMotion,
    SettingRow::CavaEnabled,
    SettingRow::CavaSmoothing,
    SettingRow::CavaAudioSource,
    SettingRow::AudioQuality,
    SettingRow::NormalizeAudio,
    SettingRow::Autoplay,
    SettingRow::Gapless,
    SettingRow::Prefetch,
    SettingRow::CacheSecs,
    SettingRow::UseMpvConfig,
    SettingRow::MprisEnabled,
    SettingRow::LyricsEnabled,
    SettingRow::LyricsSyncedOnly,
    SettingRow::LyricsSource,
    SettingRow::LyricsTranslateId,
    SettingRow::CacheEnabled,
    SettingRow::CatalogTtl,
    SettingRow::ImageCacheMaxMib,
    SettingRow::PruneCache,
    SettingRow::ClearCache,
];

impl SettingRow {
    pub(super) fn group(self) -> &'static str {
        match self {
            Self::AuthOAuth | Self::AuthBrowser | Self::Logout => "Account",
            Self::SyncNow | Self::SyncOnStartup | Self::AutoSyncInterval => "Sync",
            Self::SaveHistory
            | Self::ReportPlaybackHistory
            | Self::RemoteSuggestions
            | Self::ClearHistory => "Privacy",
            Self::ThemeAccent
            | Self::Region
            | Self::IconMode
            | Self::ImageProtocol
            | Self::ReducedMotion => "Display",
            Self::CavaEnabled | Self::CavaSmoothing | Self::CavaAudioSource => "Cava",
            Self::AudioQuality
            | Self::NormalizeAudio
            | Self::Autoplay
            | Self::Gapless
            | Self::Prefetch
            | Self::CacheSecs
            | Self::UseMpvConfig
            | Self::MprisEnabled => "Player",
            Self::LyricsEnabled
            | Self::LyricsSyncedOnly
            | Self::LyricsSource
            | Self::LyricsTranslateId => "Lyrics",
            Self::CacheEnabled
            | Self::CatalogTtl
            | Self::ImageCacheMaxMib
            | Self::PruneCache
            | Self::ClearCache => "Cache",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::AuthOAuth => "OAuth2 Device Login (TV Flow)",
            Self::AuthBrowser => "Browser Auth Import (SAPISIDHASH)",
            Self::Logout => "Logout & Reset to Anonymous (no cookies)",
            Self::SyncNow => "Sync Playlists & Library Now",
            Self::SyncOnStartup => "Auto-Sync on Startup",
            Self::AutoSyncInterval => "Background Auto-Sync Interval",
            Self::SaveHistory => "Save Local Play & Search History",
            Self::ReportPlaybackHistory => "Report Playback Telemetry to YTM Algorithm",
            Self::RemoteSuggestions => "Remote Search Autocomplete Suggestions",
            Self::ClearHistory => "Clear Local Play & Search History",
            Self::ThemeAccent => "Global Theme Accent",
            Self::Region => "YouTube Music Region",
            Self::IconMode => "Icon Glyph Set",
            Self::ImageProtocol => "Cover Image Protocol",
            Self::ReducedMotion => "Reduced Motion (Static ⣿ Spinner & Cava)",
            Self::CavaEnabled => "Enable Cava Spectrum Visualizer",
            Self::CavaSmoothing => "Cava Smoothing Curve",
            Self::CavaAudioSource => "Cava Audio Source",
            Self::AudioQuality => "Audio Stream Quality",
            Self::NormalizeAudio => "Dynamic Loudness Normalization (dynaudnorm)",
            Self::Autoplay => "Endless Radio Autoplay Continuation",
            Self::Gapless => "Gapless Audio Playback",
            Self::Prefetch => "Prefetch Next Track Stream URL",
            Self::CacheSecs => "mpv Demuxer Readahead Buffer",
            Self::UseMpvConfig => "Load User ~/.config/mpv/mpv.conf",
            Self::MprisEnabled => "Linux MPRIS D-Bus Media Controls",
            Self::LyricsEnabled => "Enable Lyrics Fetching",
            Self::LyricsSyncedOnly => "Synced Lyrics Only",
            Self::LyricsSource => "Lyrics Source Priority",
            Self::LyricsTranslateId => "Auto-Translate Lyrics to Indonesian",
            Self::CacheEnabled => "Enable L1 + L2 Cache (Covers · Lyrics · Catalog)",
            Self::CatalogTtl => "Catalog Cache TTL",
            Self::ImageCacheMaxMib => "Image Disk Cache Budget",
            Self::PruneCache => "Run Cache Janitor (Prune Expired & Over-Budget)",
            Self::ClearCache => "Clear All Disk Cache & Rotate visitorData",
        }
    }

    pub(super) fn description(self) -> &'static str {
        match self {
            Self::AuthOAuth => {
                "Sign in via Google TV OAuth2 device code stored in your OS keyring."
            }
            Self::AuthBrowser => {
                "Import browser Cookie / DevTools headers using SAPISIDHASH (never passed to yt-dlp)."
            }
            Self::Logout => {
                "Remove account credentials and return to cookie-free anonymous mode (local playlists stay)."
            }
            Self::SyncNow => "Push queued playlist/library edits and pull latest remote state.",
            Self::SyncOnStartup => {
                "Run a background 2-way sync automatically when launching musick."
            }
            Self::AutoSyncInterval => {
                "Interval between background 2-way sync cycles while signed in."
            }
            Self::SaveHistory => {
                "Persist play history, search history, and last playback session in local SQLite."
            }
            Self::ReportPlaybackHistory => {
                "Opt-in playback watchtime pings to personalize your YouTube Music algorithm (requires login)."
            }
            Self::RemoteSuggestions => {
                "Fetch live search autocomplete suggestions from YouTube Music."
            }
            Self::ClearHistory => "Erase local play history and search history immediately.",
            Self::ThemeAccent => {
                "Global accent color applied across the interface and Cava visualizer (Teal default or Magenta)."
            }
            Self::Region => {
                "Country code for Discover mixes, charts, and localized YouTube Music catalog."
            }
            Self::IconMode => "Switch between clean Unicode glyphs and Nerd Font icons.",
            Self::ImageProtocol => {
                "Terminal artwork mode: Auto (compact fallback if unsupported), Kitty, Sixel, Halfblock, or Off."
            }
            Self::ReducedMotion => {
                "Disable animated spinners and live Cava bars for low-motion terminals."
            }
            Self::CavaEnabled => {
                "Show real-time audio spectrum bars in the Player view using cava."
            }
            Self::CavaSmoothing => "Responsiveness vs. smoothness curve for Cava spectrum bars.",
            Self::CavaAudioSource => {
                "Select Auto detection, direct PipeWire capture, or PulseAudio default sink monitor."
            }
            Self::AudioQuality => {
                "Preferred stream bitrate tier (Best opus/m4a, Balanced ~128k, Saver low bandwidth)."
            }
            Self::NormalizeAudio => "Apply mpv dynaudnorm filter to level loudness across tracks.",
            Self::Autoplay => {
                "Automatically extend playback with YouTube Music Radio recommendations."
            }
            Self::Gapless => {
                "Append the next queued track to mpv for seamless gapless transitions."
            }
            Self::Prefetch => {
                "Resolve the next queued track stream URL in the background ahead of time."
            }
            Self::CacheSecs => {
                "Seconds of audio mpv buffers ahead in memory for smoother streaming."
            }
            Self::UseMpvConfig => {
                "Allow mpv to read ~/.config/mpv/mpv.conf (applies cleanly on next fresh mpv start)."
            }
            Self::MprisEnabled => {
                "Expose playback state and media keys over Linux MPRIS D-Bus (playerctl / desktop HUD)."
            }
            Self::LyricsEnabled => "Fetch synced or plain lyrics for the active track.",
            Self::LyricsSyncedOnly => {
                "Hide plain unsynced lyrics and only display time-synced lyrics."
            }
            Self::LyricsSource => {
                "Source preference order: Auto (LRCLIB -> YTM), YouTube first, or LRCLIB only."
            }
            Self::LyricsTranslateId => {
                "Automatically translate non-Indonesian lyrics to Bahasa Indonesia."
            }
            Self::CacheEnabled => {
                "Toggle in-memory L1 and SQLite/disk L2 caching for catalog, lyrics, and covers."
            }
            Self::CatalogTtl => {
                "How long search, Discover, artist, album, and playlist responses stay cached."
            }
            Self::ImageCacheMaxMib => {
                "Maximum disk space for cached cover artwork before LRU pruning kicks in."
            }
            Self::PruneCache => {
                "Immediately evict expired entries and enforce configured cache budgets."
            }
            Self::ClearCache => {
                "Delete all cached covers, lyrics, and catalog data and rotate visitorData."
            }
        }
    }
}

pub(super) fn format_setting_value(row: SettingRow, ctx: &RenderCtx<'_>) -> (String, bool) {
    let s = ctx.settings;
    let chk = |b: bool| {
        if b {
            ctx.icons.check_on
        } else {
            ctx.icons.check_off
        }
    };
    match row {
        SettingRow::AuthOAuth => (
            if ctx.auth_mode == AuthMode::OAuth {
                "Connected ●".to_string()
            } else {
                "[Open Wizard]".to_string()
            },
            false,
        ),
        SettingRow::AuthBrowser => (
            if ctx.auth_mode == AuthMode::Browser {
                "Connected ●".to_string()
            } else {
                "[Import Headers]".to_string()
            },
            false,
        ),
        SettingRow::Logout => (
            if ctx.auth_mode.is_authenticated() {
                "[Logout]".to_string()
            } else {
                "Anonymous".to_string()
            },
            !ctx.auth_mode.is_authenticated(),
        ),
        SettingRow::SyncNow => (
            if ctx.syncing {
                ctx.spinner.inline_text("Syncing")
            } else if !ctx.auth_mode.is_authenticated() {
                "Anonymous (Off)".to_string()
            } else {
                format!("{} pending", ctx.pending_sync)
            },
            !ctx.auth_mode.is_authenticated(),
        ),
        SettingRow::SyncOnStartup => (
            chk(s.sync.sync_on_startup).to_string(),
            !ctx.auth_mode.is_authenticated(),
        ),
        SettingRow::AutoSyncInterval => (
            format_interval_secs(s.sync.auto_sync_interval_secs),
            !ctx.auth_mode.is_authenticated(),
        ),
        SettingRow::SaveHistory => (chk(s.privacy.save_history).to_string(), false),
        SettingRow::ReportPlaybackHistory => (
            if !ctx.auth_mode.is_authenticated() {
                "Locked OFF (Anonymous)".to_string()
            } else {
                chk(s.privacy.report_playback_history).to_string()
            },
            !ctx.auth_mode.is_authenticated(),
        ),
        SettingRow::RemoteSuggestions => (chk(s.privacy.remote_suggestions).to_string(), false),
        SettingRow::ClearHistory => ("[Clear]".to_string(), false),
        SettingRow::ThemeAccent => (s.display.accent.label().to_string(), false),
        SettingRow::Region => (s.display.region.clone(), false),
        SettingRow::IconMode => (s.display.icons.label().to_string(), false),
        SettingRow::ImageProtocol => (s.display.image_protocol.label().to_string(), false),
        SettingRow::ReducedMotion => (chk(s.display.reduced_motion).to_string(), false),
        SettingRow::CavaEnabled => (chk(s.display.cava_enabled).to_string(), false),
        SettingRow::CavaSmoothing => (
            s.display.cava_smoothing.label().to_string(),
            !s.display.cava_enabled,
        ),
        SettingRow::CavaAudioSource => (
            s.display.cava_audio_source.label().to_string(),
            !s.display.cava_enabled,
        ),
        SettingRow::AudioQuality => (s.player.audio_quality.label().to_string(), false),
        SettingRow::NormalizeAudio => (chk(s.player.normalize_audio).to_string(), false),
        SettingRow::Autoplay => (chk(s.player.autoplay).to_string(), false),
        SettingRow::Gapless => (chk(s.player.gapless).to_string(), false),
        SettingRow::Prefetch => (chk(s.player.prefetch).to_string(), false),
        SettingRow::CacheSecs => (format!("{}s", s.player.cache_secs), false),
        SettingRow::UseMpvConfig => (chk(s.player.use_mpv_config).to_string(), false),
        SettingRow::MprisEnabled => (chk(s.player.mpris_enabled).to_string(), false),
        SettingRow::LyricsEnabled => (chk(s.lyrics.enabled).to_string(), false),
        SettingRow::LyricsSyncedOnly => (chk(s.lyrics.synced_only).to_string(), !s.lyrics.enabled),
        SettingRow::LyricsSource => (s.lyrics.source.label().to_string(), !s.lyrics.enabled),
        SettingRow::LyricsTranslateId => {
            (chk(s.lyrics.translate_to_id).to_string(), !s.lyrics.enabled)
        }
        SettingRow::CacheEnabled => (
            format!(
                "{} ({}, {} imgs)",
                chk(s.cache.enabled),
                format_bytes(ctx.cache_stats.total_bytes),
                ctx.cache_stats.image_files
            ),
            false,
        ),
        SettingRow::CatalogTtl => (
            format_interval_secs(s.cache.catalog_ttl_secs),
            !s.cache.enabled,
        ),
        SettingRow::ImageCacheMaxMib => {
            (format!("{} MiB", s.cache.images_max_mib), !s.cache.enabled)
        }
        SettingRow::PruneCache => ("[Prune Now]".to_string(), false),
        SettingRow::ClearCache => ("[Clear Cache]".to_string(), false),
    }
}

fn format_interval_secs(secs: u64) -> String {
    if secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

pub(super) fn cycle_u64(steps: &[u64], current: u64, forward: bool) -> u64 {
    let idx = steps.iter().position(|&v| v == current).unwrap_or_else(|| {
        steps
            .iter()
            .enumerate()
            .min_by_key(|&(_, &v)| v.abs_diff(current))
            .map(|(i, _)| i)
            .unwrap_or(0)
    });
    let next = if forward {
        (idx + 1) % steps.len()
    } else {
        (idx + steps.len() - 1) % steps.len()
    };
    steps[next]
}

pub(super) fn cycle_u32(steps: &[u32], current: u32, forward: bool) -> u32 {
    let idx = steps.iter().position(|&v| v == current).unwrap_or_else(|| {
        steps
            .iter()
            .enumerate()
            .min_by_key(|&(_, &v)| v.abs_diff(current))
            .map(|(i, _)| i)
            .unwrap_or(0)
    });
    let next = if forward {
        (idx + 1) % steps.len()
    } else {
        (idx + steps.len() - 1) % steps.len()
    };
    steps[next]
}
