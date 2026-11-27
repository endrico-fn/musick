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
pub mod singleflight;
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

/// build a shared http/2 rustls client with zero cookie persistence
pub fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .pool_max_idle_per_host(8)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

pub use config::{
    AudioQuality, AuthMode, CacheSettings, CavaAudioSource, CavaPalette, CavaSmoothing,
    DisplaySettings, IconMode, ImageProtocol, LyricsSettings, LyricsSource, PlayerSettings,
    PrivacySettings, Settings, SyncSettings, VALID_REGIONS,
};
pub use domain::{
    Album, AlbumSummary, Artist, ArtistRef, ArtistSummary, DiscoverFeed, LyricLine, Lyrics,
    Playlist, PlaylistSource, PlaylistSummary, QueueRole, RepeatMode, SavedCollection,
    SavedCollectionKind, SearchCategory, SearchItem, SessionState, Thumb, Track, UserPlaylist,
    best_thumb_url, clean_media_title, format_duration, format_duration_allow_zero,
    is_valid_video_id, search_item_key,
};
pub use error::{CoreError, CoreResult};
pub use paths::{AppPaths, atomic_write_bytes, atomic_write_text, ensure_private_dir};
pub use protocol::{AppCommand, PlaybackSnapshot, PlayerEvent, PositionTick, QueueSnapshot};
pub use singleflight::SingleFlight;
