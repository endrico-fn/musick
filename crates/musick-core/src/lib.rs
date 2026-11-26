pub mod auth;
pub mod config;
pub mod domain;
pub mod error;
pub mod images;
pub mod innertube;
pub mod lyrics;
pub mod paths;
pub mod protocol;
pub mod search;
pub mod storage;
pub mod sync;

pub use auth::{
    AuthManager, BrowserCredentials, KeyringSecretStore, MemorySecretStore, OAuthCredentials,
    OAuthDevicePrompt, OAuthPollStatus, SecretStore, compute_sapisidhash, parse_browser_headers,
};
pub use images::{ImageService, crop_square};
pub use innertube::{
    CatalogApi, ClientKind, InnerTubeClient, PlaybackTelemetryEvent, PlaybackTracking,
    PlaylistEditAction, PlaylistEditResult, RadioBatch, SongRating,
};
pub use lyrics::{
    LyricsService, active_line_index, looks_indonesian, parse_json3_captions, parse_lrc,
    parse_vtt_captions, should_skip_translation,
};
pub use search::{
    SearchService, Suggestion, YoutubeTarget, extract_youtube_target, is_semantic_result_set,
    parse_query_prefix, rank_search_items, score_search_item, token_set_ratio, token_sort_ratio,
    wratio,
};
pub use storage::{CacheManager, CacheStats, OutboxOp, PruneReport, UserStore};
pub use sync::{SyncEngine, SyncReport};

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
