pub mod album;
pub mod artist;
pub mod detail_hero;
pub mod library;
pub mod player;
pub mod playlist_detail;
pub mod playlists;
pub mod queue;
pub mod settings;

use musick_audio::CavaFrame;
use musick_core::{
    AppCommand, AuthMode, CacheStats, Lyrics, PlaybackSnapshot, PositionTick, QueueSnapshot,
    SavedCollectionKind, SearchCategory, Settings, Track,
};

use crate::theme::IconSet;
use crate::widgets::{ActiveModal, BrailleSpinner};

pub use album::AlbumView;
pub use artist::{ArtistOverviewPane, ArtistTab, ArtistView};
pub use library::{LibraryTab, LibraryView};
pub use player::{DiscoverTab, PlayerFocusPane, PlayerPanelMode, PlayerView};
pub use playlist_detail::PlaylistDetailView;
pub use playlists::{PlaylistsPane, PlaylistsView};
pub use queue::QueueView;
pub use settings::{SettingRow, SettingsView};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrackSortMode {
    #[default]
    Original,
    Title,
    Duration,
}

impl TrackSortMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::Original => Self::Title,
            Self::Title => Self::Duration,
            Self::Duration => Self::Original,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Title => "Title",
            Self::Duration => "Duration",
        }
    }

    /// filter and sort tracks while preserving each track's original index
    pub fn apply<'a>(self, tracks: &'a [Track], filter_query: &str) -> Vec<(usize, &'a Track)> {
        let q = filter_query.trim().to_lowercase();
        let mut items: Vec<(usize, &'a Track)> = tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                q.is_empty()
                    || t.title.to_lowercase().contains(&q)
                    || t.artist_names().to_lowercase().contains(&q)
            })
            .collect();
        match self {
            Self::Original => {}
            Self::Title => {
                items.sort_by(|a, b| {
                    a.1.title
                        .to_lowercase()
                        .cmp(&b.1.title.to_lowercase())
                        .then_with(|| a.0.cmp(&b.0))
                });
            }
            Self::Duration => {
                items.sort_by(|a, b| {
                    a.1.duration
                        .unwrap_or(u32::MAX)
                        .cmp(&b.1.duration.unwrap_or(u32::MAX))
                        .then_with(|| a.0.cmp(&b.0))
                });
            }
        }
        items
    }
}

pub struct RenderCtx<'a> {
    pub snapshot: &'a PlaybackSnapshot,
    pub position: PositionTick,
    pub queue: &'a QueueSnapshot,
    pub lyrics: &'a Lyrics,
    pub cava: &'a CavaFrame,
    pub settings: &'a Settings,
    pub icons: IconSet,
    pub spinner: BrailleSpinner,
    pub auth_mode: AuthMode,
    pub syncing: bool,
    pub pending_sync: usize,
    pub cache_stats: &'a CacheStats,
}

#[derive(Debug, Clone)]
pub enum ViewEffect {
    None,
    Command(AppCommand),
    OpenArtist {
        browse_id: String,
        name_hint: String,
    },
    OpenAlbum {
        browse_id: String,
        title_hint: String,
    },
    OpenPlaylist {
        browse_id: String,
        title_hint: String,
    },
    OpenUserPlaylist {
        playlist_id: String,
    },
    PlayUserPlaylistDirect {
        playlist_id: String,
    },
    PlayAlbumDirect {
        browse_id: String,
    },
    PlayPlaylistDirect {
        browse_id: String,
    },
    PlayArtistDirect {
        browse_id: String,
    },
    TriggerSearch {
        category: SearchCategory,
        query: String,
    },
    TriggerAutocomplete {
        query: String,
    },
    RefreshDiscover,
    OpenModal(ActiveModal),
    SyncNow,
    ToggleLike(Track),
    ToggleSave {
        kind: SavedCollectionKind,
        browse_id: String,
        title: String,
        subtitle: Option<String>,
        thumb_url: Option<String>,
    },
    AddToPlaylistPicker(Track),
    RemovePlaylistTrack {
        playlist_id: String,
        position: usize,
    },
    MovePlaylistTrack {
        playlist_id: String,
        from: usize,
        to: usize,
    },
    ExportPlaylist {
        playlist_id: String,
    },
    UploadPlaylist {
        playlist_id: String,
    },
    CopyUrl(String),
    UpdateSettings(Box<Settings>),
    ClearCache,
    PruneCache,
    ClearHistory,
    Logout,
    StatusToast(String),
}
