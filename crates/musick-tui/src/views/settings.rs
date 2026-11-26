use ratatui::Frame;
use ratatui::layout::Rect;

use musick_core::{
    AudioQuality, AuthMode, CavaAudioSource, CavaPalette, CavaSmoothing, IconMode, ImageProtocol,
    LyricsSource, Settings, VALID_REGIONS,
};

use crate::keymap::Action;
use crate::text::{Align, format_bytes};
use crate::theme::panel_block;
use crate::views::{RenderCtx, ViewEffect};
use crate::widgets::{ActiveModal, ColumnDef, ConfirmAction, TableRow, TrackTable};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingRow {
    // Account & Sync
    AuthOAuth,
    AuthBrowser,
    SyncNow,
    Logout,
    // Privacy
    SaveHistory,
    ReportPlaybackHistory,
    RemoteSuggestions,
    ClearHistory,
    // Cache
    CacheEnabled,
    PruneCache,
    ClearCache,
    // Display & Cava
    Region,
    CoverSize,
    IconMode,
    ImageProtocol,
    ReducedMotion,
    CavaEnabled,
    CavaPalette,
    CavaSmoothing,
    CavaAudioSource,
    // Player
    AudioQuality,
    NormalizeAudio,
    Autoplay,
    Gapless,
    Prefetch,
    UseMpvConfig,
    MprisEnabled,
    // Lyrics
    LyricsEnabled,
    LyricsSyncedOnly,
    LyricsSource,
    LyricsTranslateId,
}

const ALL_ROWS: &[SettingRow] = &[
    SettingRow::AuthOAuth,
    SettingRow::AuthBrowser,
    SettingRow::SyncNow,
    SettingRow::Logout,
    SettingRow::SaveHistory,
    SettingRow::ReportPlaybackHistory,
    SettingRow::RemoteSuggestions,
    SettingRow::ClearHistory,
    SettingRow::CacheEnabled,
    SettingRow::PruneCache,
    SettingRow::ClearCache,
    SettingRow::Region,
    SettingRow::CoverSize,
    SettingRow::IconMode,
    SettingRow::ImageProtocol,
    SettingRow::ReducedMotion,
    SettingRow::CavaEnabled,
    SettingRow::CavaPalette,
    SettingRow::CavaSmoothing,
    SettingRow::CavaAudioSource,
    SettingRow::AudioQuality,
    SettingRow::NormalizeAudio,
    SettingRow::Autoplay,
    SettingRow::Gapless,
    SettingRow::Prefetch,
    SettingRow::UseMpvConfig,
    SettingRow::MprisEnabled,
    SettingRow::LyricsEnabled,
    SettingRow::LyricsSyncedOnly,
    SettingRow::LyricsSource,
    SettingRow::LyricsTranslateId,
];

pub struct SettingsView {
    pub cursor: usize,
}

impl Default for SettingsView {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsView {
    pub fn new() -> Self {
        Self { cursor: 0 }
    }

    pub fn footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("j k", "select"),
            ("Enter/l/h", "toggle / cycle"),
            ("S", "sync now"),
            ("?", "help"),
            ("q", "quit"),
        ]
    }

    pub fn handle_action(
        &mut self,
        action: Action,
        settings: &Settings,
        auth_mode: AuthMode,
    ) -> ViewEffect {
        let len = ALL_ROWS.len();
        match action {
            Action::MoveDown => {
                self.cursor = (self.cursor + 1).min(len - 1);
                ViewEffect::None
            }
            Action::MoveUp => {
                self.cursor = self.cursor.saturating_sub(1);
                ViewEffect::None
            }
            Action::HalfPageDown => {
                self.cursor = (self.cursor + 8).min(len - 1);
                ViewEffect::None
            }
            Action::HalfPageUp => {
                self.cursor = self.cursor.saturating_sub(8);
                ViewEffect::None
            }
            Action::JumpTop => {
                self.cursor = 0;
                ViewEffect::None
            }
            Action::JumpBottom => {
                self.cursor = len - 1;
                ViewEffect::None
            }
            Action::Activate | Action::MoveRight | Action::MoveLeft => {
                let forward = !matches!(action, Action::MoveLeft);
                self.activate_row(ALL_ROWS[self.cursor], settings, auth_mode, forward)
            }
            _ => ViewEffect::None,
        }
    }

    fn activate_row(
        &self,
        row: SettingRow,
        settings: &Settings,
        auth_mode: AuthMode,
        forward: bool,
    ) -> ViewEffect {
        let mut next = settings.clone();
        match row {
            SettingRow::AuthOAuth => {
                return ViewEffect::OpenModal(ActiveModal::OAuthWizard {
                    client_id: std::env::var("MUSICK_OAUTH_CLIENT_ID").unwrap_or_default(),
                    client_secret: std::env::var("MUSICK_OAUTH_CLIENT_SECRET").unwrap_or_default(),
                    page_id: String::new(),
                    active_field: 0,
                    device_user_code: None,
                    verification_url: None,
                    polling: false,
                    error: None,
                });
            }
            SettingRow::AuthBrowser => {
                return ViewEffect::OpenModal(ActiveModal::BrowserAuthImport {
                    raw_headers: String::new(),
                    page_id: String::new(),
                    active_field: 0,
                    error: None,
                });
            }
            SettingRow::SyncNow => return ViewEffect::SyncNow,
            SettingRow::Logout => {
                if !auth_mode.is_authenticated() {
                    return ViewEffect::StatusToast(
                        "Already in Anonymous (no cookies) mode".into(),
                    );
                }
                return ViewEffect::OpenModal(ActiveModal::Confirm {
                    title: "Logout".to_string(),
                    message:
                        "Log out and wipe synced remote cache? Local playlists stay untouched."
                            .to_string(),
                    action: ConfirmAction::Logout,
                });
            }
            SettingRow::SaveHistory => {
                next.privacy.save_history = !next.privacy.save_history;
            }
            SettingRow::ReportPlaybackHistory => {
                // Locked strictly off in Anonymous mode!
                if !auth_mode.is_authenticated() {
                    return ViewEffect::StatusToast(
                        "Locked OFF in Anonymous mode — sign in via OAuth or Browser Auth first"
                            .into(),
                    );
                }
                next.privacy.report_playback_history = !next.privacy.report_playback_history;
            }
            SettingRow::RemoteSuggestions => {
                next.privacy.remote_suggestions = !next.privacy.remote_suggestions;
            }
            SettingRow::ClearHistory => return ViewEffect::ClearHistory,
            SettingRow::CacheEnabled => {
                next.cache.enabled = !next.cache.enabled;
            }
            SettingRow::PruneCache => return ViewEffect::PruneCache,
            SettingRow::ClearCache => {
                return ViewEffect::OpenModal(ActiveModal::Confirm {
                    title: "Clear Cache".to_string(),
                    message:
                        "Clear all cached covers, catalog, lyrics, and rotate visitorData? Playlists & Library stay intact."
                            .to_string(),
                    action: ConfirmAction::ClearCache,
                });
            }
            SettingRow::Region => {
                let idx = VALID_REGIONS
                    .iter()
                    .position(|r| *r == next.display.region)
                    .unwrap_or(0);
                let next_idx = if forward {
                    (idx + 1) % VALID_REGIONS.len()
                } else {
                    (idx + VALID_REGIONS.len() - 1) % VALID_REGIONS.len()
                };
                next.display.region = VALID_REGIONS[next_idx].to_string();
            }
            SettingRow::CoverSize => {
                next.display.cover_size = next.display.cover_size.cycle();
            }
            SettingRow::IconMode => {
                next.display.icons = match next.display.icons {
                    IconMode::Unicode => IconMode::Nerd,
                    IconMode::Nerd => IconMode::Unicode,
                };
            }
            SettingRow::ImageProtocol => {
                next.display.image_protocol = match next.display.image_protocol {
                    ImageProtocol::Auto => ImageProtocol::Kitty,
                    ImageProtocol::Kitty => ImageProtocol::Sixel,
                    ImageProtocol::Sixel => ImageProtocol::Halfblock,
                    ImageProtocol::Halfblock => ImageProtocol::Auto,
                };
            }
            SettingRow::ReducedMotion => {
                next.display.reduced_motion = !next.display.reduced_motion;
            }
            SettingRow::CavaEnabled => {
                next.display.cava_enabled = !next.display.cava_enabled;
            }
            SettingRow::CavaPalette => {
                next.display.cava_palette = match next.display.cava_palette {
                    CavaPalette::Teal => CavaPalette::Emerald,
                    CavaPalette::Emerald => CavaPalette::Aurora,
                    CavaPalette::Aurora => CavaPalette::Sunset,
                    CavaPalette::Sunset => CavaPalette::Mono,
                    CavaPalette::Mono => CavaPalette::Teal,
                };
            }
            SettingRow::CavaSmoothing => {
                next.display.cava_smoothing = match next.display.cava_smoothing {
                    CavaSmoothing::Smooth => CavaSmoothing::Balanced,
                    CavaSmoothing::Balanced => CavaSmoothing::Crisp,
                    CavaSmoothing::Crisp => CavaSmoothing::Smooth,
                };
            }
            SettingRow::CavaAudioSource => {
                next.display.cava_audio_source = match next.display.cava_audio_source {
                    CavaAudioSource::Auto => CavaAudioSource::Monitor,
                    CavaAudioSource::Monitor => CavaAudioSource::Off,
                    CavaAudioSource::Off => CavaAudioSource::Auto,
                };
            }
            SettingRow::AudioQuality => {
                next.player.audio_quality = match next.player.audio_quality {
                    AudioQuality::Best => AudioQuality::Balanced,
                    AudioQuality::Balanced => AudioQuality::Saver,
                    AudioQuality::Saver => AudioQuality::Best,
                };
            }
            SettingRow::NormalizeAudio => {
                next.player.normalize_audio = !next.player.normalize_audio;
            }
            SettingRow::Autoplay => {
                next.player.autoplay = !next.player.autoplay;
            }
            SettingRow::Gapless => {
                next.player.gapless = !next.player.gapless;
            }
            SettingRow::Prefetch => {
                next.player.prefetch = !next.player.prefetch;
            }
            SettingRow::UseMpvConfig => {
                next.player.use_mpv_config = !next.player.use_mpv_config;
            }
            SettingRow::MprisEnabled => {
                next.player.mpris_enabled = !next.player.mpris_enabled;
            }
            SettingRow::LyricsEnabled => {
                next.lyrics.enabled = !next.lyrics.enabled;
            }
            SettingRow::LyricsSyncedOnly => {
                next.lyrics.synced_only = !next.lyrics.synced_only;
            }
            SettingRow::LyricsSource => {
                next.lyrics.source = match next.lyrics.source {
                    LyricsSource::Auto => LyricsSource::Youtube,
                    LyricsSource::Youtube => LyricsSource::Lrclib,
                    LyricsSource::Lrclib => LyricsSource::Auto,
                };
            }
            SettingRow::LyricsTranslateId => {
                next.lyrics.translate_to_id = !next.lyrics.translate_to_id;
            }
        }
        next.normalize();
        ViewEffect::UpdateSettings(Box::new(next))
    }

    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx<'_>) {
        if area.width < 20 || area.height < 5 {
            return;
        }
        let cache_summary = format!(
            "Auth: {} · Cache: {}",
            ctx.auth_mode.as_str(),
            format_bytes(ctx.cache_stats.total_bytes)
        );
        let block = panel_block("Settings", Some(&cache_summary), true);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let s = ctx.settings;
        let chk = |b: bool| {
            if b {
                ctx.icons.check_on
            } else {
                ctx.icons.check_off
            }
        };

        let rows: Vec<TableRow> = ALL_ROWS
            .iter()
            .map(|&r| {
                let (cat, label, val, is_dim) = match r {
                    SettingRow::AuthOAuth => (
                        "Account",
                        "OAuth2 Device Login (TV Flow)",
                        if ctx.auth_mode == AuthMode::OAuth {
                            "Connected ●".to_string()
                        } else {
                            "[Open Wizard]".to_string()
                        },
                        false,
                    ),
                    SettingRow::AuthBrowser => (
                        "Account",
                        "Browser Auth Import (SAPISIDHASH)",
                        if ctx.auth_mode == AuthMode::Browser {
                            "Connected ●".to_string()
                        } else {
                            "[Import Headers]".to_string()
                        },
                        false,
                    ),
                    SettingRow::SyncNow => (
                        "Sync",
                        "Sync Playlists & Library Now",
                        if ctx.syncing {
                            ctx.spinner.inline_text("Syncing")
                        } else if !ctx.auth_mode.is_authenticated() {
                            "Anonymous (off)".to_string()
                        } else {
                            format!("{} pending", ctx.pending_sync)
                        },
                        !ctx.auth_mode.is_authenticated(),
                    ),
                    SettingRow::Logout => (
                        "Account",
                        "Logout & Reset to Anonymous (no cookies)",
                        if ctx.auth_mode.is_authenticated() {
                            "[Logout]".to_string()
                        } else {
                            "Anonymous".to_string()
                        },
                        !ctx.auth_mode.is_authenticated(),
                    ),
                    SettingRow::SaveHistory => (
                        "Privacy",
                        "Save Local Play & Search History",
                        chk(s.privacy.save_history).to_string(),
                        false,
                    ),
                    SettingRow::ReportPlaybackHistory => (
                        "Privacy",
                        "Report Playback Telemetry to YTM Algorithm",
                        if !ctx.auth_mode.is_authenticated() {
                            "Locked OFF (Anonymous)".to_string()
                        } else {
                            chk(s.privacy.report_playback_history).to_string()
                        },
                        !ctx.auth_mode.is_authenticated(),
                    ),
                    SettingRow::RemoteSuggestions => (
                        "Privacy",
                        "Remote Search Autocomplete Suggestions",
                        chk(s.privacy.remote_suggestions).to_string(),
                        false,
                    ),
                    SettingRow::ClearHistory => (
                        "Privacy",
                        "Clear Local Play & Search History",
                        "[Clear]".to_string(),
                        false,
                    ),
                    SettingRow::CacheEnabled => (
                        "Cache",
                        "Enable L2 Disk Cache (Covers · Lyrics · Catalog)",
                        format!(
                            "{} ({}, {} imgs)",
                            chk(s.cache.enabled),
                            format_bytes(ctx.cache_stats.total_bytes),
                            ctx.cache_stats.image_files
                        ),
                        false,
                    ),
                    SettingRow::PruneCache => (
                        "Cache",
                        "Run Cache Janitor (Prune Expired & Over-Budget)",
                        "[Prune Now]".to_string(),
                        false,
                    ),
                    SettingRow::ClearCache => (
                        "Cache",
                        "Clear All Disk Cache & Rotate visitorData",
                        "[Clear Cache]".to_string(),
                        false,
                    ),
                    SettingRow::Region => (
                        "Display",
                        "YouTube Music Region",
                        s.display.region.clone(),
                        false,
                    ),
                    SettingRow::CoverSize => (
                        "Display",
                        "Cover Grid Card Size",
                        s.display.cover_size.as_str().to_string(),
                        false,
                    ),
                    SettingRow::IconMode => (
                        "Display",
                        "Icon Glyph Set",
                        format!("{:?}", s.display.icons),
                        false,
                    ),
                    SettingRow::ImageProtocol => (
                        "Display",
                        "Cover Image Protocol",
                        format!("{:?}", s.display.image_protocol),
                        false,
                    ),
                    SettingRow::ReducedMotion => (
                        "Display",
                        "Reduced Motion (Static ⣿ Spinner & Cava)",
                        chk(s.display.reduced_motion).to_string(),
                        false,
                    ),
                    SettingRow::CavaEnabled => (
                        "Cava",
                        "Enable Cava Spectrum Visualizer",
                        chk(s.display.cava_enabled).to_string(),
                        false,
                    ),
                    SettingRow::CavaPalette => (
                        "Cava",
                        "Cava Color Palette",
                        format!("{:?}", s.display.cava_palette),
                        false,
                    ),
                    SettingRow::CavaSmoothing => (
                        "Cava",
                        "Cava Smoothing Curve",
                        format!("{:?}", s.display.cava_smoothing),
                        false,
                    ),
                    SettingRow::CavaAudioSource => (
                        "Cava",
                        "Cava Audio Source",
                        format!("{:?}", s.display.cava_audio_source),
                        false,
                    ),
                    SettingRow::AudioQuality => (
                        "Player",
                        "Audio Stream Quality",
                        s.player.audio_quality.as_str().to_string(),
                        false,
                    ),
                    SettingRow::NormalizeAudio => (
                        "Player",
                        "Dynamic Loudness Normalization (dynaudnorm)",
                        chk(s.player.normalize_audio).to_string(),
                        false,
                    ),
                    SettingRow::Autoplay => (
                        "Player",
                        "Endless Radio Autoplay Continuation",
                        chk(s.player.autoplay).to_string(),
                        false,
                    ),
                    SettingRow::Gapless => (
                        "Player",
                        "Gapless Audio Playback",
                        chk(s.player.gapless).to_string(),
                        false,
                    ),
                    SettingRow::Prefetch => (
                        "Player",
                        "Prefetch Next Track Stream URL",
                        chk(s.player.prefetch).to_string(),
                        false,
                    ),
                    SettingRow::UseMpvConfig => (
                        "Player",
                        "Load User ~/.config/mpv/mpv.conf",
                        chk(s.player.use_mpv_config).to_string(),
                        false,
                    ),
                    SettingRow::MprisEnabled => (
                        "Player",
                        "Linux MPRIS D-Bus Media Controls",
                        chk(s.player.mpris_enabled).to_string(),
                        false,
                    ),
                    SettingRow::LyricsEnabled => (
                        "Lyrics",
                        "Enable Lyrics Fetching",
                        chk(s.lyrics.enabled).to_string(),
                        false,
                    ),
                    SettingRow::LyricsSyncedOnly => (
                        "Lyrics",
                        "Synced Lyrics Only",
                        chk(s.lyrics.synced_only).to_string(),
                        false,
                    ),
                    SettingRow::LyricsSource => (
                        "Lyrics",
                        "Lyrics Source Priority",
                        s.lyrics.source.as_str().to_string(),
                        false,
                    ),
                    SettingRow::LyricsTranslateId => (
                        "Lyrics",
                        "Auto-Translate Lyrics to Indonesian",
                        chk(s.lyrics.translate_to_id).to_string(),
                        false,
                    ),
                };
                let mut tr = TableRow::new(vec![cat.to_string(), label.to_string(), val]);
                tr.is_dim = is_dim;
                tr
            })
            .collect();

        let cols = [
            ColumnDef {
                title: "Group",
                fixed_width: Some(9),
                align: Align::Left,
            },
            ColumnDef {
                title: "Setting",
                fixed_width: None,
                align: Align::Left,
            },
            ColumnDef {
                title: "Value",
                fixed_width: Some(24),
                align: Align::Right,
            },
        ];
        frame.render_widget(
            TrackTable {
                columns: &cols,
                rows: &rows,
                cursor: self.cursor,
                focused: true,
                show_header: true,
                loading: false,
                loading_label: "",
                empty_text: "",
                icons: ctx.icons,
                spinner: ctx.spinner,
            },
            inner,
        );
    }
}
