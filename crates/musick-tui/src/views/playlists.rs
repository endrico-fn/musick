use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use musick_core::{AppCommand, PlaylistSource, Track, UserPlaylist};

use crate::keymap::Action;
use crate::text::Align;
use crate::theme::{ACCENT, CURSOR_BG, TEXT, panel_block};
use crate::views::{RenderCtx, TrackSortMode, ViewEffect};
use crate::widgets::{ActiveModal, ColumnDef, ConfirmAction, TableRow, TrackTable};

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
    pub filter_active: bool,
    pub filter_query: String,
    pub track_filter_active: bool,
    pub track_filter_query: String,
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
            filter_active: false,
            filter_query: String::new(),
            track_filter_active: false,
            track_filter_query: String::new(),
            track_sort: TrackSortMode::Original,
        }
    }

    pub fn is_insert_mode(&self) -> bool {
        self.filter_active || self.track_filter_active
    }

    pub fn filtered_playlists(&self) -> Vec<&UserPlaylist> {
        let q = self.filter_query.trim().to_lowercase();
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

    /// Returns `(original_index, &Track)` pairs matching the right-pane track filter
    /// and ordered by `self.track_sort`.
    pub fn visible_tracks(&self) -> Vec<(usize, &Track)> {
        self.track_sort
            .apply(&self.selected_tracks, &self.track_filter_query)
    }

    pub fn is_track_filtered(&self) -> bool {
        !self.track_filter_query.trim().is_empty()
    }

    pub fn on_playlist_selection_changed(&mut self) {
        self.track_cursor = 0;
        self.track_filter_query.clear();
        self.track_filter_active = false;
    }

    pub fn select_by_id(&mut self, playlist_id: &str) {
        self.filter_query.clear();
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
                ("dd", "delete"),
                ("I/E", "import/export"),
                ("U", "upload YTM"),
                ("S", "sync"),
                ("l/Tab", "tracks"),
                ("/", "filter"),
            ],
            PlaylistsPane::Tracks => &[
                ("Enter", "play"),
                ("/", "filter"),
                ("o", "sort"),
                ("J/K", "reorder"),
                ("dd", "remove"),
                ("N/a", "next/queue"),
                ("f", "like"),
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
            return match action {
                Action::InputChar(ch) => {
                    self.filter_query.push(ch);
                    self.list_cursor = 0;
                    self.track_cursor = 0;
                    ViewEffect::None
                }
                Action::InputBackspace => {
                    self.filter_query.pop();
                    self.list_cursor = 0;
                    self.track_cursor = 0;
                    ViewEffect::None
                }
                Action::InputSubmit | Action::InputCancel => {
                    self.filter_active = false;
                    ViewEffect::None
                }
                _ => ViewEffect::None,
            };
        }

        if self.track_filter_active {
            return match action {
                Action::InputChar(ch) => {
                    self.track_filter_query.push(ch);
                    self.track_cursor = 0;
                    ViewEffect::None
                }
                Action::InputBackspace => {
                    self.track_filter_query.pop();
                    self.track_cursor = 0;
                    ViewEffect::None
                }
                Action::InputSubmit | Action::InputCancel => {
                    self.track_filter_active = false;
                    ViewEffect::None
                }
                _ => ViewEffect::None,
            };
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
                        if !self.track_filter_query.is_empty() {
                            self.track_filter_query.clear();
                            self.track_cursor = 0;
                        } else {
                            self.pane = PlaylistsPane::List;
                        }
                    }
                    PlaylistsPane::List => {
                        if !self.filter_query.is_empty() {
                            self.filter_query.clear();
                            self.list_cursor = 0;
                            self.track_cursor = 0;
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
            Action::CycleCoverSize => {
                // `c` creates a new playlist on the Playlists page
                return ViewEffect::OpenModal(ActiveModal::CreatePlaylist {
                    input: String::new(),
                });
            }
            Action::ImportPlaylist => {
                return ViewEffect::OpenModal(ActiveModal::ImportPlaylistUrl {
                    input: String::new(),
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
                        autoplay: false,
                    })
                } else {
                    ViewEffect::None
                }
            }
            Action::RenamePlaylist => {
                if let Some(pl) = self.selected_playlist() {
                    ViewEffect::OpenModal(ActiveModal::RenamePlaylist {
                        playlist_id: pl.id.clone(),
                        input: pl.title.clone(),
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
            if matches!(action, Action::Activate | Action::PlayDirect) && len > 0 {
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

    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx<'_>) {
        if area.width < 20 || area.height < 5 {
            return;
        }
        let left_w = ((u32::from(area.width) * 34) / 100).clamp(20, 38) as u16;
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(left_w),
                Constraint::Length(1),
                Constraint::Min(20),
            ])
            .split(area);

        // Left Pane: Playlist Directory
        let filtered = self.filtered_playlists();
        let list_sub = if ctx.syncing {
            ctx.spinner.inline_text("Syncing")
        } else {
            format!("{} lists", filtered.len())
        };
        let left_block = panel_block(
            "Playlists",
            Some(&list_sub),
            self.pane == PlaylistsPane::List,
        );
        let left_inner = left_block.inner(panes[0]);
        frame.render_widget(left_block, panes[0]);

        let (filter_rect, list_rect) = if self.filter_active || !self.filter_query.is_empty() {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(left_inner);
            (Some(split[0]), split[1])
        } else {
            (None, left_inner)
        };

        if let Some(fr) = filter_rect {
            let line = Line::from(vec![
                Span::styled("/ ", Style::default().fg(ACCENT).bg(Color::Reset)),
                Span::styled(
                    if self.filter_active {
                        format!("{}█", self.filter_query)
                    } else {
                        self.filter_query.clone()
                    },
                    Style::default().fg(TEXT).bg(if self.filter_active {
                        CURSOR_BG
                    } else {
                        Color::Reset
                    }),
                ),
            ]);
            frame.render_widget(Paragraph::new(line), fr);
        }

        let pl_rows: Vec<TableRow> = filtered
            .iter()
            .map(|pl| {
                let mut row = TableRow::new(vec![pl.title.clone(), format!("{}", pl.track_count)]);
                row.prefix_icon = Some(if pl.source == PlaylistSource::YtmSynced {
                    ctx.icons.cloud
                } else {
                    ctx.icons.local
                });
                row
            })
            .collect();
        let pl_cols = [
            ColumnDef {
                title: "Playlist",
                fixed_width: None,
                align: Align::Left,
            },
            ColumnDef {
                title: "#",
                fixed_width: Some(4),
                align: Align::Right,
            },
        ];
        frame.render_widget(
            TrackTable {
                columns: &pl_cols,
                rows: &pl_rows,
                cursor: self.list_cursor,
                focused: self.pane == PlaylistsPane::List,
                show_header: false,
                loading: false,
                loading_label: "",
                empty_text: if self.filter_query.trim().is_empty() {
                    "Press c to create or I to import"
                } else {
                    "No playlists match filter"
                },
                icons: ctx.icons,
                spinner: ctx.spinner,
            },
            list_rect,
        );

        // Right Pane: Selected Playlist Tracks
        let visible = self.visible_tracks();
        let (right_title, right_sub) = match self.selected_playlist() {
            Some(pl) => {
                let src = match pl.source {
                    PlaylistSource::YtmSynced => "YTM Synced",
                    PlaylistSource::Local => "Local · U upload",
                };
                let count_text = if self.is_track_filtered() {
                    format!("{}/{} tracks", visible.len(), self.selected_tracks.len())
                } else {
                    format!("{} tracks", self.selected_tracks.len())
                };
                (
                    pl.title.as_str(),
                    format!("{count_text} · {} · {src}", self.track_sort.label()),
                )
            }
            None => ("Tracks", "No playlist selected".to_string()),
        };

        let right_block = panel_block(
            right_title,
            Some(&right_sub),
            self.pane == PlaylistsPane::Tracks,
        );
        let right_inner = right_block.inner(panes[2]);
        frame.render_widget(right_block, panes[2]);

        let (track_filter_rect, track_table_rect) =
            if self.track_filter_active || !self.track_filter_query.is_empty() {
                let split = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(1)])
                    .split(right_inner);
                (Some(split[0]), split[1])
            } else {
                (None, right_inner)
            };

        if let Some(fr) = track_filter_rect {
            let line = Line::from(vec![
                Span::styled("/ ", Style::default().fg(ACCENT).bg(Color::Reset)),
                Span::styled(
                    if self.track_filter_active {
                        format!("{}█", self.track_filter_query)
                    } else {
                        self.track_filter_query.clone()
                    },
                    Style::default().fg(TEXT).bg(if self.track_filter_active {
                        CURSOR_BG
                    } else {
                        Color::Reset
                    }),
                ),
            ]);
            frame.render_widget(Paragraph::new(line), fr);
        }

        let playing_vid = ctx.snapshot.track.as_ref().map(|t| t.video_id.as_str());
        let tr_rows: Vec<TableRow> = visible
            .iter()
            .map(|(orig_idx, t)| {
                let mut r = TableRow::new(vec![
                    format!("{:>2}. {}", orig_idx + 1, t.title),
                    t.artist_names(),
                    t.duration_text(),
                ]);
                r.is_playing = playing_vid == Some(t.video_id.as_str());
                r.is_dim = !t.is_available;
                r
            })
            .collect();
        let tr_cols = [
            ColumnDef {
                title: "Title",
                fixed_width: None,
                align: Align::Left,
            },
            ColumnDef {
                title: "Artist",
                fixed_width: Some(18),
                align: Align::Left,
            },
            ColumnDef {
                title: "Time",
                fixed_width: Some(6),
                align: Align::Right,
            },
        ];

        frame.render_widget(
            TrackTable {
                columns: &tr_cols,
                rows: &tr_rows,
                cursor: self.track_cursor,
                focused: self.pane == PlaylistsPane::Tracks,
                show_header: true,
                loading: false,
                loading_label: "",
                empty_text: if self.is_track_filtered() {
                    "No tracks match filter"
                } else {
                    "Playlist is empty — press P on any song to add tracks here"
                },
                icons: ctx.icons,
                spinner: ctx.spinner,
            },
            track_table_rect,
        );
    }
}
