pub mod auth;
pub mod config;
pub mod domain;
pub mod error;
pub mod paths;
pub mod protocol;

pub use auth::{
    AuthManager, BrowserCredentials, KeyringSecretStore, MemorySecretStore, OAuthCredentials,
    OAuthDevicePrompt, OAuthPollStatus, SecretStore, compute_sapisidhash, parse_browser_headers,
};

pub use config::{
    AudioQuality, AuthMode, CacheSettings, CavaAudioSource, CavaPalette, CavaSmoothing, CoverSize,
    DisplaySettings, IconMode, ImageProtocol, LyricsSettings, LyricsSource, PlayerSettings,
    PrivacySettings, Settings, SyncSettings, VALID_REGIONS,
};
pub use domain::{
    Album, AlbumSummary, Artist, ArtistRef, ArtistSummary, HomeFeed, LyricLine, Lyrics, Playlist,
    PlaylistSource, PlaylistSummary, QueueRole, RepeatMode, SavedCollection, SavedCollectionKind,
    SearchCategory, SearchItem, Thumb, Track, UserPlaylist, best_thumb_url, clean_media_title,
    format_duration, format_duration_allow_zero, search_item_key,
};
pub use error::{CoreError, CoreResult};
pub use paths::{AppPaths, atomic_write_bytes, atomic_write_text, ensure_private_dir};
pub use protocol::{AppCommand, PlaybackSnapshot, PlayerEvent, PositionTick, QueueSnapshot};
