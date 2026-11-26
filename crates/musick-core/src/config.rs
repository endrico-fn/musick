use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::CoreResult;
use crate::paths::atomic_write_text;

pub const VALID_REGIONS: &[&str] = &["ID", "US", "GB", "JP", "KR", "ZZ"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    #[default]
    Anonymous,
    OAuth,
    Browser,
}

impl AuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anonymous => "anonymous",
            Self::OAuth => "oauth",
            Self::Browser => "browser",
        }
    }

    pub fn is_authenticated(self) -> bool {
        !matches!(self, Self::Anonymous)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CoverSize {
    S,
    #[default]
    M,
    L,
}

impl CoverSize {
    pub fn cycle(self) -> Self {
        match self {
            Self::S => Self::M,
            Self::M => Self::L,
            Self::L => Self::S,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::S => "S",
            Self::M => "M",
            Self::L => "L",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IconMode {
    #[default]
    Unicode,
    Nerd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageProtocol {
    #[default]
    Auto,
    #[serde(alias = "tgp")]
    Kitty,
    Sixel,
    #[serde(alias = "halfcell")]
    Halfblock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CavaPalette {
    #[default]
    Teal,
    Emerald,
    Aurora,
    Sunset,
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CavaSmoothing {
    #[default]
    Smooth,
    Balanced,
    Crisp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CavaAudioSource {
    #[default]
    Auto,
    Monitor,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioQuality {
    #[default]
    Best,
    Balanced,
    Saver,
}

impl AudioQuality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Best => "best",
            Self::Balanced => "balanced",
            Self::Saver => "saver",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LyricsSource {
    #[default]
    Auto,
    Youtube,
    Lrclib,
}

impl LyricsSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Youtube => "youtube",
            Self::Lrclib => "lrclib",
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_region() -> String {
    "ID".to_string()
}

fn default_cache_secs() -> u32 {
    30
}

fn default_volume() -> u8 {
    80
}

fn default_images_max_mib() -> u64 {
    256
}

fn default_images_max_files() -> usize {
    2048
}

fn default_lyrics_max_mib() -> u64 {
    32
}

fn default_catalog_max_mib() -> u64 {
    64
}

fn default_catalog_ttl_secs() -> u64 {
    600
}

fn default_visitor_data_ttl_secs() -> u64 {
    86_400
}

fn default_image_ttl_days() -> u64 {
    30
}

fn default_sync_interval_secs() -> u64 {
    300
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacySettings {
    /// No cookies / anonymous by default; user must explicitly opt into OAuth or Browser auth.
    pub auth_mode: AuthMode,
    pub save_history: bool,
    /// Strictly opt-in playback telemetry to YouTube Music algorithm (never active in Anonymous).
    pub report_playback_history: bool,
    #[serde(default = "default_true")]
    pub remote_suggestions: bool,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            auth_mode: AuthMode::Anonymous,
            save_history: false,
            report_playback_history: false,
            remote_suggestions: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_images_max_mib")]
    pub images_max_mib: u64,
    #[serde(default = "default_images_max_files")]
    pub images_max_files: usize,
    #[serde(default = "default_lyrics_max_mib")]
    pub lyrics_max_mib: u64,
    #[serde(default = "default_catalog_max_mib")]
    pub catalog_max_mib: u64,
    #[serde(default = "default_catalog_ttl_secs")]
    pub catalog_ttl_secs: u64,
    #[serde(default = "default_visitor_data_ttl_secs")]
    pub visitor_data_ttl_secs: u64,
    #[serde(default = "default_image_ttl_days")]
    pub image_ttl_days: u64,
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            images_max_mib: default_images_max_mib(),
            images_max_files: default_images_max_files(),
            lyrics_max_mib: default_lyrics_max_mib(),
            catalog_max_mib: default_catalog_max_mib(),
            catalog_ttl_secs: default_catalog_ttl_secs(),
            visitor_data_ttl_secs: default_visitor_data_ttl_secs(),
            image_ttl_days: default_image_ttl_days(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplaySettings {
    pub cover_size: CoverSize,
    pub icons: IconMode,
    pub image_protocol: ImageProtocol,
    #[serde(default = "default_region")]
    pub region: String,
    pub reduced_motion: bool,
    #[serde(default = "default_true")]
    pub cava_enabled: bool,
    pub cava_palette: CavaPalette,
    pub cava_smoothing: CavaSmoothing,
    pub cava_audio_source: CavaAudioSource,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            cover_size: CoverSize::M,
            icons: IconMode::Unicode,
            image_protocol: ImageProtocol::Auto,
            region: default_region(),
            reduced_motion: false,
            cava_enabled: true,
            cava_palette: CavaPalette::Teal,
            cava_smoothing: CavaSmoothing::Smooth,
            cava_audio_source: CavaAudioSource::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerSettings {
    #[serde(default = "default_true")]
    pub prefetch: bool,
    #[serde(default = "default_true")]
    pub gapless: bool,
    #[serde(default = "default_true")]
    pub autoplay: bool,
    pub normalize_audio: bool,
    pub audio_quality: AudioQuality,
    #[serde(default = "default_cache_secs")]
    pub cache_secs: u32,
    #[serde(default = "default_volume")]
    pub volume: u8,
    pub use_mpv_config: bool,
    #[serde(default = "default_true")]
    pub mpris_enabled: bool,
}

impl Default for PlayerSettings {
    fn default() -> Self {
        Self {
            prefetch: true,
            gapless: true,
            autoplay: true,
            normalize_audio: false,
            audio_quality: AudioQuality::Best,
            cache_secs: default_cache_secs(),
            volume: default_volume(),
            use_mpv_config: false,
            mpris_enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LyricsSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub synced_only: bool,
    pub source: LyricsSource,
    pub translate_to_id: bool,
}

impl Default for LyricsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            synced_only: false,
            source: LyricsSource::Auto,
            translate_to_id: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncSettings {
    #[serde(default = "default_sync_interval_secs")]
    pub auto_sync_interval_secs: u64,
    #[serde(default = "default_true")]
    pub sync_on_startup: bool,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            auto_sync_interval_secs: default_sync_interval_secs(),
            sync_on_startup: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub privacy: PrivacySettings,
    pub cache: CacheSettings,
    pub display: DisplaySettings,
    pub player: PlayerSettings,
    pub lyrics: LyricsSettings,
    pub sync: SyncSettings,
}

impl Settings {
    pub fn normalize(&mut self) -> &mut Self {
        if !VALID_REGIONS.contains(&self.display.region.as_str()) {
            self.display.region = default_region();
        }
        self.player.cache_secs = self.player.cache_secs.clamp(5, 300);
        self.player.volume = self.player.volume.clamp(0, 130);
        self.cache.images_max_mib = self.cache.images_max_mib.clamp(16, 4096);
        self.cache.images_max_files = self.cache.images_max_files.clamp(64, 20_000);
        self.cache.lyrics_max_mib = self.cache.lyrics_max_mib.clamp(4, 512);
        self.cache.catalog_max_mib = self.cache.catalog_max_mib.clamp(8, 1024);
        self.cache.catalog_ttl_secs = self.cache.catalog_ttl_secs.clamp(60, 86_400);
        self.cache.visitor_data_ttl_secs = self.cache.visitor_data_ttl_secs.clamp(300, 604_800);
        self.cache.image_ttl_days = self.cache.image_ttl_days.clamp(1, 365);
        self.sync.auto_sync_interval_secs = self.sync.auto_sync_interval_secs.clamp(60, 3600);
        if !self.privacy.auth_mode.is_authenticated() {
            self.privacy.report_playback_history = false;
        }
        self
    }

    pub fn from_toml_str(raw: &str) -> Self {
        if let Ok(mut parsed) = toml::from_str::<Self>(raw) {
            parsed.normalize();
            return parsed;
        }

        // Fall back to section-by-section recovery if one field in legacy settings has an invalid value.
        let mut settings = Self::default();
        if let Ok(table) = toml::from_str::<toml::Table>(raw) {
            if let Some(sec) = table.get("privacy")
                && let Ok(v) = sec.clone().try_into::<PrivacySettings>()
            {
                settings.privacy = v;
            }
            if let Some(sec) = table.get("cache")
                && let Ok(v) = sec.clone().try_into::<CacheSettings>()
            {
                settings.cache = v;
            }
            if let Some(sec) = table.get("display")
                && let Ok(v) = sec.clone().try_into::<DisplaySettings>()
            {
                settings.display = v;
            }
            if let Some(sec) = table.get("player")
                && let Ok(v) = sec.clone().try_into::<PlayerSettings>()
            {
                settings.player = v;
            }
            if let Some(sec) = table.get("lyrics")
                && let Ok(v) = sec.clone().try_into::<LyricsSettings>()
            {
                settings.lyrics = v;
            }
            if let Some(sec) = table.get("sync")
                && let Ok(v) = sec.clone().try_into::<SyncSettings>()
            {
                settings.sync = v;
            }
        }
        settings.normalize();
        settings
    }

    pub fn to_toml_string(&self) -> CoreResult<String> {
        let mut copy = self.clone();
        copy.normalize();
        toml::to_string_pretty(&copy).map_err(|e| crate::error::CoreError::Config(e.to_string()))
    }

    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(raw) => Self::from_toml_str(&raw),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> CoreResult<()> {
        let text = self.to_toml_string()?;
        atomic_write_text(path, &text)
    }
}
