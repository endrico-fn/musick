mod render;
mod rows;

use musick_core::{
    AudioQuality, AuthMode, CavaAudioSource, CavaSmoothing, IconMode, ImageProtocol, LyricsSource,
    Settings, VALID_REGIONS,
};

use crate::keymap::Action;
use crate::views::ViewEffect;
use crate::widgets::{ActiveModal, ConfirmAction, ScrollState, TextInput};

pub use rows::{SettingRow, SettingsSection};

use rows::{
    ALL_ROWS, AUTO_SYNC_STEPS, CACHE_SECS_STEPS, CATALOG_TTL_STEPS, IMAGE_CAP_MIB_STEPS, cycle_u32,
    cycle_u64,
};

pub struct SettingsView {
    pub section: SettingsSection,
    pub secret_backend: &'static str,
    pub cursor: usize,
    pub scroll: ScrollState,
    pub filter_active: bool,
    pub filter: TextInput,
}

impl Default for SettingsView {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsView {
    pub fn new() -> Self {
        Self {
            section: SettingsSection::All,
            secret_backend: "OS Keyring",
            cursor: 0,
            scroll: ScrollState::new(),
            filter_active: false,
            filter: TextInput::new(),
        }
    }

    pub fn is_insert_mode(&self) -> bool {
        self.filter_active
    }

    pub fn has_filter(&self) -> bool {
        !self.filter.trim().is_empty()
    }

    pub fn select_row(&mut self, target: SettingRow) {
        if !self.section.matches(target) {
            self.section = SettingsSection::All;
        }
        let vis = self.visible_rows();
        if let Some(pos) = vis.iter().position(|r| *r == target) {
            self.cursor = pos;
        }
    }

    pub(super) fn visible_rows(&self) -> Vec<SettingRow> {
        let q = self.filter.trim().to_lowercase();
        ALL_ROWS
            .iter()
            .copied()
            .filter(|&r| self.section.matches(r))
            .filter(|r| {
                q.is_empty()
                    || r.group().to_lowercase().contains(&q)
                    || r.label().to_lowercase().contains(&q)
                    || r.description().to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        if self.filter_active {
            &[("Enter", "apply filter"), ("Esc", "close filter")]
        } else {
            &[
                ("Tab", "section"),
                ("/", "filter"),
                ("j k", "select"),
                ("Enter/l/h", "toggle / cycle"),
                ("S", "sync now"),
                ("?", "help"),
                ("Q", "quit"),
            ]
        }
    }

    pub fn handle_action(
        &mut self,
        action: Action,
        settings: &Settings,
        auth_mode: AuthMode,
    ) -> ViewEffect {
        if self.filter_active {
            let (handled, changed) = self.filter.handle_action(&action);
            if handled {
                if changed {
                    self.cursor = 0;
                    self.scroll.reset();
                }
                return ViewEffect::None;
            }
            if matches!(action, Action::InputSubmit | Action::InputCancel) {
                self.filter_active = false;
            }
            return ViewEffect::None;
        }

        let vis = self.visible_rows();
        let len = vis.len();
        if len > 0 {
            self.cursor = self.cursor.min(len - 1);
        } else {
            self.cursor = 0;
        }

        match action {
            Action::NextSubTab => {
                self.section = self.section.next();
                self.cursor = 0;
                self.scroll.reset();
                ViewEffect::None
            }
            Action::PrevSubTab => {
                self.section = self.section.prev();
                self.cursor = 0;
                self.scroll.reset();
                ViewEffect::None
            }
            Action::StartSearch => {
                self.filter_active = true;
                ViewEffect::None
            }
            Action::Escape => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.cursor = 0;
                    self.scroll.reset();
                }
                ViewEffect::None
            }
            Action::MoveDown => {
                if len > 0 {
                    self.cursor = (self.cursor + 1).min(len - 1);
                }
                ViewEffect::None
            }
            Action::MoveUp => {
                self.cursor = self.cursor.saturating_sub(1);
                ViewEffect::None
            }
            Action::HalfPageDown => {
                if len > 0 {
                    self.cursor = (self.cursor + 8).min(len - 1);
                }
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
                self.cursor = len.saturating_sub(1);
                ViewEffect::None
            }
            Action::Activate => vis
                .get(self.cursor)
                .map(|&row| self.activate_row(row, settings, auth_mode, true, true))
                .unwrap_or(ViewEffect::None),
            Action::MoveRight => vis
                .get(self.cursor)
                .map(|&row| self.activate_row(row, settings, auth_mode, true, false))
                .unwrap_or(ViewEffect::None),
            Action::MoveLeft => vis
                .get(self.cursor)
                .map(|&row| self.activate_row(row, settings, auth_mode, false, false))
                .unwrap_or(ViewEffect::None),
            _ => ViewEffect::None,
        }
    }

    fn activate_row(
        &self,
        row: SettingRow,
        settings: &Settings,
        auth_mode: AuthMode,
        forward: bool,
        is_enter: bool,
    ) -> ViewEffect {
        let mut next = settings.clone();
        match row {
            SettingRow::AuthOAuth => {
                if !is_enter {
                    return ViewEffect::None;
                }
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
                if !is_enter {
                    return ViewEffect::None;
                }
                return ViewEffect::OpenModal(ActiveModal::BrowserAuthImport {
                    raw_headers: String::new(),
                    page_id: String::new(),
                    active_field: 0,
                    error: None,
                });
            }
            SettingRow::SyncNow => {
                return if is_enter {
                    ViewEffect::SyncNow
                } else {
                    ViewEffect::None
                };
            }
            SettingRow::SyncOnStartup => {
                next.sync.sync_on_startup = !next.sync.sync_on_startup;
            }
            SettingRow::AutoSyncInterval => {
                next.sync.auto_sync_interval_secs =
                    cycle_u64(AUTO_SYNC_STEPS, next.sync.auto_sync_interval_secs, forward);
            }
            SettingRow::Logout => {
                if !is_enter {
                    return ViewEffect::None;
                }
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
            SettingRow::ClearHistory => {
                return if is_enter {
                    ViewEffect::ClearHistory
                } else {
                    ViewEffect::None
                };
            }
            SettingRow::CacheEnabled => {
                next.cache.enabled = !next.cache.enabled;
            }
            SettingRow::CatalogTtl => {
                next.cache.catalog_ttl_secs =
                    cycle_u64(CATALOG_TTL_STEPS, next.cache.catalog_ttl_secs, forward);
            }
            SettingRow::ImageCacheMaxMib => {
                next.cache.images_max_mib =
                    cycle_u64(IMAGE_CAP_MIB_STEPS, next.cache.images_max_mib, forward);
            }
            SettingRow::PruneCache => {
                return if is_enter {
                    ViewEffect::PruneCache
                } else {
                    ViewEffect::None
                };
            }
            SettingRow::ClearCache => {
                if !is_enter {
                    return ViewEffect::None;
                }
                return ViewEffect::OpenModal(ActiveModal::Confirm {
                    title: "Clear Cache".to_string(),
                    message:
                        "Clear all cached covers, catalog, lyrics, and rotate visitorData? Playlists & Library stay intact."
                            .to_string(),
                    action: ConfirmAction::ClearCache,
                });
            }
            SettingRow::ThemeAccent => {
                next.display.accent = next.display.accent.toggle();
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
            SettingRow::IconMode => {
                next.display.icons = match next.display.icons {
                    IconMode::Unicode => IconMode::Nerd,
                    IconMode::Nerd => IconMode::Unicode,
                };
            }
            SettingRow::ImageProtocol => {
                next.display.image_protocol = match (next.display.image_protocol, forward) {
                    (ImageProtocol::Auto, true) | (ImageProtocol::Sixel, false) => {
                        ImageProtocol::Kitty
                    }
                    (ImageProtocol::Kitty, true) | (ImageProtocol::Halfblock, false) => {
                        ImageProtocol::Sixel
                    }
                    (ImageProtocol::Sixel, true) | (ImageProtocol::Off, false) => {
                        ImageProtocol::Halfblock
                    }
                    (ImageProtocol::Halfblock, true) | (ImageProtocol::Auto, false) => {
                        ImageProtocol::Off
                    }
                    (ImageProtocol::Off, true) | (ImageProtocol::Kitty, false) => {
                        ImageProtocol::Auto
                    }
                };
            }
            SettingRow::ReducedMotion => {
                next.display.reduced_motion = !next.display.reduced_motion;
            }
            SettingRow::CavaEnabled => {
                next.display.cava_enabled = !next.display.cava_enabled;
            }
            SettingRow::CavaSmoothing => {
                next.display.cava_smoothing = match (next.display.cava_smoothing, forward) {
                    (CavaSmoothing::Smooth, true) | (CavaSmoothing::Crisp, false) => {
                        CavaSmoothing::Balanced
                    }
                    (CavaSmoothing::Balanced, true) | (CavaSmoothing::Smooth, false) => {
                        CavaSmoothing::Crisp
                    }
                    (CavaSmoothing::Crisp, true) | (CavaSmoothing::Balanced, false) => {
                        CavaSmoothing::Smooth
                    }
                };
            }
            SettingRow::CavaAudioSource => {
                next.display.cava_audio_source = match (next.display.cava_audio_source, forward) {
                    (CavaAudioSource::Auto, true) | (CavaAudioSource::PulseMonitor, false) => {
                        CavaAudioSource::PipeWire
                    }
                    (CavaAudioSource::PipeWire, true) | (CavaAudioSource::Auto, false) => {
                        CavaAudioSource::PulseMonitor
                    }
                    (CavaAudioSource::PulseMonitor, true) | (CavaAudioSource::PipeWire, false) => {
                        CavaAudioSource::Auto
                    }
                };
            }
            SettingRow::AudioQuality => {
                next.player.audio_quality = match (next.player.audio_quality, forward) {
                    (AudioQuality::Best, true) | (AudioQuality::Saver, false) => {
                        AudioQuality::Balanced
                    }
                    (AudioQuality::Balanced, true) | (AudioQuality::Best, false) => {
                        AudioQuality::Saver
                    }
                    (AudioQuality::Saver, true) | (AudioQuality::Balanced, false) => {
                        AudioQuality::Best
                    }
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
            SettingRow::CacheSecs => {
                next.player.cache_secs =
                    cycle_u32(CACHE_SECS_STEPS, next.player.cache_secs, forward);
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
                next.lyrics.source = match (next.lyrics.source, forward) {
                    (LyricsSource::Auto, true) | (LyricsSource::Lrclib, false) => {
                        LyricsSource::Youtube
                    }
                    (LyricsSource::Youtube, true) | (LyricsSource::Auto, false) => {
                        LyricsSource::Lrclib
                    }
                    (LyricsSource::Lrclib, true) | (LyricsSource::Youtube, false) => {
                        LyricsSource::Auto
                    }
                };
            }
            SettingRow::LyricsTranslateId => {
                next.lyrics.translate_to_id = !next.lyrics.translate_to_id;
            }
        }
        next.normalize();
        ViewEffect::UpdateSettings(Box::new(next))
    }
}
