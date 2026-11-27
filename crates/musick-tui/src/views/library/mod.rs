mod render;

use musick_core::{AppCommand, SavedCollection, Track, UserPlaylist};

use crate::keymap::Action;
use crate::views::ViewEffect;
use crate::widgets::{ActiveModal, ConfirmAction, ScrollState, TextInput};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibraryTab {
    #[default]
    UserPlaylists,
    LikedSongs,
    SavedAlbums,
    SavedArtists,
    SavedPlaylists,
    Recent,
}

impl LibraryTab {
    pub const ALL: &'static [Self] = &[
        Self::UserPlaylists,
        Self::LikedSongs,
        Self::SavedAlbums,
        Self::SavedArtists,
        Self::SavedPlaylists,
        Self::Recent,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

pub struct LibraryView {
    pub active_tab: LibraryTab,
    pub user_playlists: Vec<UserPlaylist>,
    pub liked_songs: Vec<Track>,
    pub saved_albums: Vec<SavedCollection>,
    pub saved_artists: Vec<SavedCollection>,
    pub saved_playlists: Vec<SavedCollection>,
    pub recent_tracks: Vec<Track>,

    pub cursor: usize,
    pub scroll: ScrollState,
    pub filter_active: bool,
    pub filter: TextInput,
}

impl Default for LibraryView {
    fn default() -> Self {
        Self::new()
    }
}

impl LibraryView {
    pub fn new() -> Self {
        Self {
            active_tab: LibraryTab::UserPlaylists,
            user_playlists: Vec::new(),
            liked_songs: Vec::new(),
            saved_albums: Vec::new(),
            saved_artists: Vec::new(),
            saved_playlists: Vec::new(),
            recent_tracks: Vec::new(),
            cursor: 0,
            scroll: ScrollState::new(),
            filter_active: false,
            filter: TextInput::new(),
        }
    }

    pub fn is_insert_mode(&self) -> bool {
        self.filter_active
    }

    pub fn hovered_track(&self) -> Option<Track> {
        self.selected_track()
    }

    pub fn footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("Tab", "sub-tab"),
            ("/", "filter"),
            ("Enter", "open/play"),
            ("p", "play"),
            ("N/a", "next/queue"),
            ("dd/w", "remove"),
            ("S", "sync"),
        ]
    }

    pub(super) fn matches_track(t: &Track, q: &str) -> bool {
        q.is_empty()
            || t.title.to_lowercase().contains(q)
            || t.artist_names().to_lowercase().contains(q)
    }

    pub(super) fn matches_collection(c: &SavedCollection, q: &str) -> bool {
        q.is_empty()
            || c.title.to_lowercase().contains(q)
            || c.subtitle
                .as_deref()
                .is_some_and(|s| s.to_lowercase().contains(q))
    }

    pub(super) fn current_len(&self) -> usize {
        let q = self.filter.trim().to_lowercase();
        match self.active_tab {
            LibraryTab::UserPlaylists => self
                .user_playlists
                .iter()
                .filter(|p| q.is_empty() || p.title.to_lowercase().contains(&q))
                .count(),
            LibraryTab::LikedSongs => self
                .liked_songs
                .iter()
                .filter(|t| Self::matches_track(t, &q))
                .count(),
            LibraryTab::SavedAlbums => self
                .saved_albums
                .iter()
                .filter(|c| Self::matches_collection(c, &q))
                .count(),
            LibraryTab::SavedArtists => self
                .saved_artists
                .iter()
                .filter(|c| Self::matches_collection(c, &q))
                .count(),
            LibraryTab::SavedPlaylists => self
                .saved_playlists
                .iter()
                .filter(|c| Self::matches_collection(c, &q))
                .count(),
            LibraryTab::Recent => self
                .recent_tracks
                .iter()
                .filter(|t| Self::matches_track(t, &q))
                .count(),
        }
    }

    pub fn handle_action(&mut self, action: Action) -> ViewEffect {
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

        let len = self.current_len();

        match action {
            Action::NextSubTab => {
                self.active_tab = self.active_tab.next();
                self.cursor = 0;
                self.scroll.reset();
                ViewEffect::None
            }
            Action::PrevSubTab => {
                self.active_tab = self.active_tab.prev();
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
            Action::Activate => self.activate_selected(),
            Action::PlayDirect => self.play_selected_direct(),
            Action::PlayNext => self
                .selected_track()
                .map(|t| ViewEffect::Command(AppCommand::PlayNext { tracks: vec![t] }))
                .unwrap_or(ViewEffect::None),
            Action::AppendQueue => self
                .selected_track()
                .map(|t| ViewEffect::Command(AppCommand::AppendTracks { tracks: vec![t] }))
                .unwrap_or(ViewEffect::None),
            Action::AddToPlaylist => self
                .selected_track()
                .map(ViewEffect::AddToPlaylistPicker)
                .unwrap_or(ViewEffect::None),
            Action::ToggleLike => self
                .selected_track()
                .map(ViewEffect::ToggleLike)
                .unwrap_or(ViewEffect::None),
            Action::ToggleSave => self.remove_or_unsave_selected(false),
            Action::DeleteItem => self.remove_or_unsave_selected(true),
            Action::StartRadio => self
                .selected_track()
                .map(|t| {
                    ViewEffect::Command(AppCommand::StartRadio {
                        video_id: Some(t.video_id.clone()),
                        playlist_id: None,
                        seed_track: Some(t),
                    })
                })
                .unwrap_or(ViewEffect::None),
            Action::CopyUrl => self
                .selected_url()
                .map(ViewEffect::CopyUrl)
                .unwrap_or(ViewEffect::None),
            _ => ViewEffect::None,
        }
    }

    fn selected_user_playlist(&self) -> Option<&UserPlaylist> {
        let q = self.filter.trim().to_lowercase();
        self.user_playlists
            .iter()
            .filter(|p| q.is_empty() || p.title.to_lowercase().contains(&q))
            .nth(self.cursor)
    }

    fn selected_track(&self) -> Option<Track> {
        let q = self.filter.trim().to_lowercase();
        let list = match self.active_tab {
            LibraryTab::LikedSongs => &self.liked_songs,
            LibraryTab::Recent => &self.recent_tracks,
            _ => return None,
        };
        list.iter()
            .filter(|t| Self::matches_track(t, &q))
            .nth(self.cursor)
            .cloned()
    }

    fn selected_collection(&self) -> Option<SavedCollection> {
        let q = self.filter.trim().to_lowercase();
        let list = match self.active_tab {
            LibraryTab::SavedAlbums => &self.saved_albums,
            LibraryTab::SavedArtists => &self.saved_artists,
            LibraryTab::SavedPlaylists => &self.saved_playlists,
            _ => return None,
        };
        list.iter()
            .filter(|c| Self::matches_collection(c, &q))
            .nth(self.cursor)
            .cloned()
    }

    fn activate_selected(&self) -> ViewEffect {
        let q = self.filter.trim().to_lowercase();
        match self.active_tab {
            LibraryTab::UserPlaylists => {
                if let Some(pl) = self.selected_user_playlist() {
                    ViewEffect::OpenUserPlaylist {
                        playlist_id: pl.id.clone(),
                    }
                } else {
                    ViewEffect::None
                }
            }
            LibraryTab::LikedSongs => {
                let tracks: Vec<Track> = self
                    .liked_songs
                    .iter()
                    .filter(|t| Self::matches_track(t, &q))
                    .cloned()
                    .collect();
                if tracks.is_empty() {
                    return ViewEffect::None;
                }
                let start = self.cursor.min(tracks.len() - 1);
                ViewEffect::Command(AppCommand::PlayTracks {
                    tracks,
                    start_index: start,
                    autoplay: true,
                })
            }
            LibraryTab::Recent => {
                let tracks: Vec<Track> = self
                    .recent_tracks
                    .iter()
                    .filter(|t| Self::matches_track(t, &q))
                    .cloned()
                    .collect();
                if tracks.is_empty() {
                    return ViewEffect::None;
                }
                let start = self.cursor.min(tracks.len() - 1);
                ViewEffect::Command(AppCommand::PlayTracks {
                    tracks,
                    start_index: start,
                    autoplay: true,
                })
            }
            LibraryTab::SavedAlbums => self
                .selected_collection()
                .map(|c| ViewEffect::OpenAlbum {
                    browse_id: c.browse_id,
                    title_hint: c.title,
                })
                .unwrap_or(ViewEffect::None),
            LibraryTab::SavedArtists => self
                .selected_collection()
                .map(|c| ViewEffect::OpenArtist {
                    browse_id: c.browse_id,
                    name_hint: c.title,
                })
                .unwrap_or(ViewEffect::None),
            LibraryTab::SavedPlaylists => self
                .selected_collection()
                .map(|c| ViewEffect::OpenPlaylist {
                    browse_id: c.browse_id,
                    title_hint: c.title,
                })
                .unwrap_or(ViewEffect::None),
        }
    }

    fn play_selected_direct(&self) -> ViewEffect {
        match self.active_tab {
            LibraryTab::LikedSongs | LibraryTab::Recent => self.activate_selected(),
            LibraryTab::SavedAlbums => self
                .selected_collection()
                .map(|c| ViewEffect::PlayAlbumDirect {
                    browse_id: c.browse_id,
                })
                .unwrap_or(ViewEffect::None),
            LibraryTab::SavedArtists => self
                .selected_collection()
                .map(|c| ViewEffect::PlayArtistDirect {
                    browse_id: c.browse_id,
                })
                .unwrap_or(ViewEffect::None),
            LibraryTab::SavedPlaylists => self
                .selected_collection()
                .map(|c| ViewEffect::PlayPlaylistDirect {
                    browse_id: c.browse_id,
                })
                .unwrap_or(ViewEffect::None),
            LibraryTab::UserPlaylists => self
                .selected_user_playlist()
                .map(|pl| ViewEffect::PlayUserPlaylistDirect {
                    playlist_id: pl.id.clone(),
                })
                .unwrap_or(ViewEffect::None),
        }
    }

    fn selected_url(&self) -> Option<String> {
        match self.active_tab {
            LibraryTab::UserPlaylists => self
                .selected_user_playlist()
                .and_then(|pl| pl.remote_id.as_deref())
                .map(|rid| format!("https://music.youtube.com/playlist?list={rid}")),
            LibraryTab::LikedSongs | LibraryTab::Recent => self.selected_track().map(|t| t.url()),
            LibraryTab::SavedAlbums => self
                .selected_collection()
                .map(|c| format!("https://music.youtube.com/browse/{}", c.browse_id)),
            LibraryTab::SavedArtists => self
                .selected_collection()
                .map(|c| format!("https://music.youtube.com/channel/{}", c.browse_id)),
            LibraryTab::SavedPlaylists => self
                .selected_collection()
                .map(|c| format!("https://music.youtube.com/playlist?list={}", c.browse_id)),
        }
    }

    fn remove_or_unsave_selected(&self, is_delete: bool) -> ViewEffect {
        match self.active_tab {
            LibraryTab::UserPlaylists if is_delete => self
                .selected_user_playlist()
                .map(|pl| {
                    ViewEffect::OpenModal(ActiveModal::Confirm {
                        title: "Delete Playlist".to_string(),
                        message: format!("Delete playlist \"{}\"?", pl.title),
                        action: ConfirmAction::DeletePlaylist {
                            playlist_id: pl.id.clone(),
                        },
                    })
                })
                .unwrap_or(ViewEffect::None),
            LibraryTab::LikedSongs => self
                .selected_track()
                .map(ViewEffect::ToggleLike)
                .unwrap_or(ViewEffect::None),
            LibraryTab::SavedAlbums | LibraryTab::SavedArtists | LibraryTab::SavedPlaylists => self
                .selected_collection()
                .map(|c| ViewEffect::ToggleSave {
                    kind: c.kind,
                    browse_id: c.browse_id,
                    title: c.title,
                    subtitle: c.subtitle,
                    thumb_url: c.thumb_url,
                })
                .unwrap_or(ViewEffect::None),
            _ => ViewEffect::None,
        }
    }
}
