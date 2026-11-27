mod render;

use musick_core::{AppCommand, Track, UserPlaylist};

use crate::keymap::Action;
use crate::views::artist::shuffle_tracks_in_place;
use crate::views::{TrackSortMode, ViewEffect};
use crate::widgets::{ActiveModal, ConfirmAction, ScrollState, TextInput};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaylistsPane {
    #[default]
    List,
    Tracks,
}

pub struct PlaylistsView {
    pub pane: PlaylistsPane,
    pub playlists: Vec<UserPlaylist>,
    pub selected_tracks: Vec<Track>,
    pub list_cursor: usize,
    pub track_cursor: usize,
    pub list_scroll: ScrollState,
    pub track_scroll: ScrollState,
    pub filter_active: bool,
    pub filter: TextInput,
    pub track_filter_active: bool,
    pub track_filter: TextInput,
    pub track_sort: TrackSortMode,
}

impl Default for PlaylistsView {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaylistsView {
    pub fn new() -> Self {
        Self {
            pane: PlaylistsPane::List,
            playlists: Vec::new(),
            selected_tracks: Vec::new(),
            list_cursor: 0,
            track_cursor: 0,
            list_scroll: ScrollState::new(),
            track_scroll: ScrollState::new(),
            filter_active: false,
            filter: TextInput::new(),
            track_filter_active: false,
            track_filter: TextInput::new(),
            track_sort: TrackSortMode::Original,
        }
    }

    pub fn is_insert_mode(&self) -> bool {
        self.filter_active || self.track_filter_active
    }

    pub fn hovered_track(&self) -> Option<Track> {
        if self.pane != PlaylistsPane::Tracks {
            return None;
        }
        self.visible_tracks()
            .get(self.track_cursor)
            .map(|(_, t)| (*t).clone())
    }

    pub fn filtered_playlists(&self) -> Vec<&UserPlaylist> {
        let q = self.filter.trim().to_lowercase();
        if q.is_empty() {
            self.playlists.iter().collect()
        } else {
            self.playlists
                .iter()
                .filter(|p| p.title.to_lowercase().contains(&q))
                .collect()
        }
    }

    pub fn selected_playlist(&self) -> Option<&UserPlaylist> {
        let filtered = self.filtered_playlists();
        filtered
            .get(self.list_cursor.min(filtered.len().saturating_sub(1)))
            .copied()
    }

    /// returns `(original_index, &Track)` pairs matching the right-pane filter and sort order
    pub fn visible_tracks(&self) -> Vec<(usize, &Track)> {
        self.track_sort
            .apply(&self.selected_tracks, self.track_filter.as_str())
    }

    pub fn is_track_filtered(&self) -> bool {
        !self.track_filter.trim().is_empty()
    }

    pub fn on_playlist_selection_changed(&mut self) {
        self.track_cursor = 0;
        self.track_scroll.reset();
        self.track_filter.clear();
        self.track_filter_active = false;
    }

    pub fn select_by_id(&mut self, playlist_id: &str) {
        self.filter.clear();
        self.filter_active = false;
        self.on_playlist_selection_changed();
        if let Some(idx) = self.playlists.iter().position(|p| p.id == playlist_id) {
            self.list_cursor = idx;
            self.pane = PlaylistsPane::Tracks;
        }
    }

    pub fn set_playlists(&mut self, playlists: Vec<UserPlaylist>) {
        self.playlists = playlists;
        let len = self.filtered_playlists().len();
        self.list_cursor = self.list_cursor.min(len.saturating_sub(1));
    }

    pub fn set_selected_tracks(&mut self, tracks: Vec<Track>) {
        self.selected_tracks = tracks;
        let vis_len = self.visible_tracks().len();
        self.track_cursor = self.track_cursor.min(vis_len.saturating_sub(1));
    }

    pub fn footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        match self.pane {
            PlaylistsPane::List => &[
                ("c", "create"),
                ("R", "rename"),
                ("x", "shuffle"),
                ("N/q", "next/queue"),
                ("dd", "delete"),
                ("I/E", "import/export"),
                ("U/S", "upload/sync"),
                ("l/Tab", "tracks"),
            ],
            PlaylistsPane::Tracks => &[
                ("Enter", "play"),
                ("x", "shuffle"),
                ("J/K", "reorder"),
                ("dd", "remove"),
                ("N/q", "next/queue"),
                ("A/a", "artist/album"),
                ("e", "menu"),
                ("h/Tab", "lists"),
            ],
        }
    }

    fn reorder_guard_toast(&self) -> Option<String> {
        let filtered = self.is_track_filtered();
        let sorted = self.track_sort != TrackSortMode::Original;
        match (filtered, sorted) {
            (true, true) => {
                Some("Clear track filter (Esc) and set sort to Original (o) to reorder".into())
            }
            (true, false) => Some("Clear track filter (Esc) before reordering tracks".into()),
            (false, true) => Some("Set sort to Original (o) before reordering tracks".into()),
            (false, false) => None,
        }
    }

    pub fn handle_action(&mut self, action: Action) -> ViewEffect {
        if self.filter_active {
            let (handled, changed) = self.filter.handle_action(&action);
            if handled {
                if changed {
                    self.list_cursor = 0;
                    self.list_scroll.reset();
                    self.track_cursor = 0;
                    self.track_scroll.reset();
                }
                return ViewEffect::None;
            }
            if matches!(action, Action::InputSubmit | Action::InputCancel) {
                self.filter_active = false;
            }
            return ViewEffect::None;
        }

        if self.track_filter_active {
            let (handled, changed) = self.track_filter.handle_action(&action);
            if handled {
                if changed {
                    self.track_cursor = 0;
                    self.track_scroll.reset();
                }
                return ViewEffect::None;
            }
            if matches!(action, Action::InputSubmit | Action::InputCancel) {
                self.track_filter_active = false;
            }
            return ViewEffect::None;
        }

        match action {
            Action::StartSearch => {
                match self.pane {
                    PlaylistsPane::List => {
                        self.filter_active = true;
                        self.track_filter_active = false;
                    }
                    PlaylistsPane::Tracks => {
                        self.track_filter_active = true;
                        self.filter_active = false;
                    }
                }
                return ViewEffect::None;
            }
            Action::Escape => {
                match self.pane {
                    PlaylistsPane::Tracks => {
                        if !self.track_filter.is_empty() {
                            self.track_filter.clear();
                            self.track_cursor = 0;
                            self.track_scroll.reset();
                        } else {
                            self.pane = PlaylistsPane::List;
                        }
                    }
                    PlaylistsPane::List => {
                        if !self.filter.is_empty() {
                            self.filter.clear();
                            self.list_cursor = 0;
                            self.list_scroll.reset();
                            self.track_cursor = 0;
                            self.track_scroll.reset();
                        }
                    }
                }
                return ViewEffect::None;
            }
            Action::FocusPaneLeft | Action::MoveLeft => {
                self.pane = PlaylistsPane::List;
                return ViewEffect::None;
            }
            Action::FocusPaneRight | Action::MoveRight => {
                self.pane = PlaylistsPane::Tracks;
                return ViewEffect::None;
            }
            Action::NextSubTab | Action::PrevSubTab => {
                self.pane = match self.pane {
                    PlaylistsPane::List => PlaylistsPane::Tracks,
                    PlaylistsPane::Tracks => PlaylistsPane::List,
                };
                return ViewEffect::None;
            }
            Action::CreatePlaylist => {
                return ViewEffect::OpenModal(ActiveModal::CreatePlaylist {
                    input: TextInput::new(),
                });
            }
            Action::ImportPlaylist => {
                return ViewEffect::OpenModal(ActiveModal::ImportPlaylistUrl {
                    input: TextInput::new(),
                });
            }
            _ => {}
        }

        match self.pane {
            PlaylistsPane::List => self.handle_list_pane(action),
            PlaylistsPane::Tracks => self.handle_tracks_pane(action),
        }
    }

    fn handle_list_pane(&mut self, action: Action) -> ViewEffect {
        let len = self.filtered_playlists().len();
        match action {
            Action::MoveDown => {
                if len > 0 {
                    let next = (self.list_cursor + 1).min(len - 1);
                    if next != self.list_cursor {
                        self.list_cursor = next;
                        self.on_playlist_selection_changed();
                    }
                }
                ViewEffect::None
            }
            Action::MoveUp => {
                let next = self.list_cursor.saturating_sub(1);
                if next != self.list_cursor {
                    self.list_cursor = next;
                    self.on_playlist_selection_changed();
                }
                ViewEffect::None
            }
            Action::HalfPageDown => {
                if len > 0 {
                    let next = (self.list_cursor + 8).min(len - 1);
                    if next != self.list_cursor {
                        self.list_cursor = next;
                        self.on_playlist_selection_changed();
                    }
                }
                ViewEffect::None
            }
            Action::HalfPageUp => {
                let next = self.list_cursor.saturating_sub(8);
                if next != self.list_cursor {
                    self.list_cursor = next;
                    self.on_playlist_selection_changed();
                }
                ViewEffect::None
            }
            Action::JumpTop => {
                if self.list_cursor != 0 {
                    self.list_cursor = 0;
                    self.on_playlist_selection_changed();
                }
                ViewEffect::None
            }
            Action::JumpBottom => {
                let next = len.saturating_sub(1);
                if next != self.list_cursor {
                    self.list_cursor = next;
                    self.on_playlist_selection_changed();
                }
                ViewEffect::None
            }
            Action::Activate => {
                if self.selected_playlist().is_some() {
                    self.pane = PlaylistsPane::Tracks;
                }
                ViewEffect::None
            }
            Action::PlayDirect => {
                if !self.selected_tracks.is_empty() {
                    ViewEffect::Command(AppCommand::PlayTracks {
                        tracks: self.selected_tracks.clone(),
                        start_index: 0,
                        autoplay: true,
                    })
                } else {
                    ViewEffect::None
                }
            }
            Action::ArtistShuffle => {
                if let Some(pl) = self.selected_playlist() {
                    ViewEffect::ShuffleUserPlaylistDirect {
                        playlist_id: pl.id.clone(),
                    }
                } else {
                    ViewEffect::None
                }
            }
            Action::PlayNext => {
                if let Some(pl) = self.selected_playlist() {
                    ViewEffect::QueueUserPlaylist {
                        playlist_id: pl.id.clone(),
                        play_next: true,
                    }
                } else {
                    ViewEffect::None
                }
            }
            Action::AppendQueue => {
                if let Some(pl) = self.selected_playlist() {
                    ViewEffect::QueueUserPlaylist {
                        playlist_id: pl.id.clone(),
                        play_next: false,
                    }
                } else {
                    ViewEffect::None
                }
            }
            Action::RenamePlaylist => {
                if let Some(pl) = self.selected_playlist() {
                    ViewEffect::OpenModal(ActiveModal::RenamePlaylist {
                        playlist_id: pl.id.clone(),
                        input: TextInput::from_text(pl.title.clone()),
                    })
                } else {
                    ViewEffect::None
                }
            }
            Action::DeleteItem => {
                if let Some(pl) = self.selected_playlist() {
                    ViewEffect::OpenModal(ActiveModal::Confirm {
                        title: "Delete Playlist".to_string(),
                        message: format!("Delete playlist \"{}\"?", pl.title),
                        action: ConfirmAction::DeletePlaylist {
                            playlist_id: pl.id.clone(),
                        },
                    })
                } else {
                    ViewEffect::None
                }
            }
            Action::ExportPlaylist => {
                if let Some(pl) = self.selected_playlist() {
                    ViewEffect::ExportPlaylist {
                        playlist_id: pl.id.clone(),
                    }
                } else {
                    ViewEffect::None
                }
            }
            Action::UploadPlaylist => {
                if let Some(pl) = self.selected_playlist() {
                    ViewEffect::UploadPlaylist {
                        playlist_id: pl.id.clone(),
                    }
                } else {
                    ViewEffect::None
                }
            }
            _ => ViewEffect::None,
        }
    }

    fn handle_tracks_pane(&mut self, action: Action) -> ViewEffect {
        let pl_id = self.selected_playlist().map(|p| p.id.clone());
        let visible = self.visible_tracks();
        let len = visible.len();
        let selected_entry = visible
            .get(self.track_cursor)
            .map(|&(orig_idx, t)| (orig_idx, t.clone()));
        let visible_owned: Option<Vec<Track>> =
            if matches!(
                action,
                Action::Activate | Action::PlayDirect | Action::ArtistShuffle
            ) && len > 0
            {
                Some(visible.iter().map(|(_, t)| (*t).clone()).collect())
            } else {
                None
            };
        drop(visible);

        match action {
            Action::CycleSort => {
                self.track_sort = self.track_sort.cycle();
                self.track_cursor = 0;
                ViewEffect::None
            }
            Action::MoveDown => {
                if len > 0 {
                    self.track_cursor = (self.track_cursor + 1).min(len - 1);
                }
                ViewEffect::None
            }
            Action::MoveUp => {
                self.track_cursor = self.track_cursor.saturating_sub(1);
                ViewEffect::None
            }
            Action::HalfPageDown => {
                if len > 0 {
                    self.track_cursor = (self.track_cursor + 8).min(len - 1);
                }
                ViewEffect::None
            }
            Action::HalfPageUp => {
                self.track_cursor = self.track_cursor.saturating_sub(8);
                ViewEffect::None
            }
            Action::JumpTop => {
                self.track_cursor = 0;
                ViewEffect::None
            }
            Action::JumpBottom => {
                self.track_cursor = len.saturating_sub(1);
                ViewEffect::None
            }
            Action::Activate | Action::PlayDirect => {
                if let Some(tracks) = visible_owned {
                    ViewEffect::Command(AppCommand::PlayTracks {
                        start_index: self.track_cursor.min(tracks.len() - 1),
                        tracks,
                        autoplay: true,
                    })
                } else {
                    ViewEffect::None
                }
            }
            Action::ArtistShuffle => {
                if let Some(mut tracks) = visible_owned {
                    shuffle_tracks_in_place(&mut tracks);
                    ViewEffect::Command(AppCommand::PlayTracks {
                        tracks,
                        start_index: 0,
                        autoplay: true,
                    })
                } else {
                    ViewEffect::None
                }
            }
            Action::PlayNext => selected_entry
                .map(|(_, t)| ViewEffect::Command(AppCommand::PlayNext { tracks: vec![t] }))
                .unwrap_or(ViewEffect::None),
            Action::AppendQueue => selected_entry
                .map(|(_, t)| ViewEffect::Command(AppCommand::AppendTracks { tracks: vec![t] }))
                .unwrap_or(ViewEffect::None),
            Action::ToggleLike => selected_entry
                .map(|(_, t)| ViewEffect::ToggleLike(t))
                .unwrap_or(ViewEffect::None),
            Action::AddToPlaylist => selected_entry
                .map(|(_, t)| ViewEffect::AddToPlaylistPicker(t))
                .unwrap_or(ViewEffect::None),
            Action::StartRadio => selected_entry
                .map(|(_, t)| {
                    ViewEffect::Command(AppCommand::StartRadio {
                        video_id: Some(t.video_id.clone()),
                        playlist_id: None,
                        seed_track: Some(t),
                    })
                })
                .unwrap_or(ViewEffect::None),
            Action::DeleteItem => {
                if let (Some(pid), Some((orig_pos, _))) = (pl_id, selected_entry) {
                    return ViewEffect::RemovePlaylistTrack {
                        playlist_id: pid,
                        position: orig_pos,
                    };
                }
                ViewEffect::None
            }
            Action::MoveItemDown => {
                if let Some(pid) = pl_id {
                    if let Some(msg) = self.reorder_guard_toast() {
                        return ViewEffect::StatusToast(msg);
                    }
                    if self.track_cursor + 1 < len {
                        let from = self.track_cursor;
                        let to = from + 1;
                        self.track_cursor = to;
                        return ViewEffect::MovePlaylistTrack {
                            playlist_id: pid,
                            from,
                            to,
                        };
                    }
                }
                ViewEffect::None
            }
            Action::MoveItemUp => {
                if let Some(pid) = pl_id {
                    if let Some(msg) = self.reorder_guard_toast() {
                        return ViewEffect::StatusToast(msg);
                    }
                    if self.track_cursor > 0 && self.track_cursor < len {
                        let from = self.track_cursor;
                        let to = from - 1;
                        self.track_cursor = to;
                        return ViewEffect::MovePlaylistTrack {
                            playlist_id: pid,
                            from,
                            to,
                        };
                    }
                }
                ViewEffect::None
            }
            Action::CopyUrl => selected_entry
                .map(|(_, t)| ViewEffect::CopyUrl(t.url()))
                .unwrap_or(ViewEffect::None),
            _ => ViewEffect::None,
        }
    }
}
