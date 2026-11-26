pub mod home;
pub mod library;
pub mod playlists;
pub mod queue;

use musick_audio::CavaFrame;
use musick_core::{
    AppCommand, AuthMode, CacheStats, Lyrics, PlaybackSnapshot, PositionTick, QueueSnapshot,
    SavedCollectionKind, SearchCategory, Settings, Track,
};

use crate::theme::IconSet;
use crate::widgets::{ActiveModal, BrailleSpinner};

pub use home::{HomeDiscoverTab, HomeFocusPane, HomePanelMode, HomeView};
pub use library::{LibraryTab, LibraryView};
pub use playlists::{PlaylistsPane, PlaylistsView};
pub use queue::QueueView;

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
    RefreshHome,
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
