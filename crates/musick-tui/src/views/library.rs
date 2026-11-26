use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use musick_core::{
    AppCommand, CoverSize, PlaylistSource, SavedCollection, SavedCollectionKind, Track,
    UserPlaylist,
};

use crate::keymap::Action;
use crate::text::Align;
use crate::theme::{ACCENT, CURSOR_BG, TEXT, panel_block};
use crate::views::{RenderCtx, ViewEffect};
use crate::widgets::{
    ColumnDef, CoverRenderer, GridCardItem, SectionTabs, TableRow, TrackTable, grid_columns_for,
    render_cover_grid,
};

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

const LIBRARY_TAB_LABELS: &[(&str, &str)] = &[
    ("User Playlists", "Playlists"),
    ("Liked Songs", "Liked"),
    ("Saved Albums", "Albums"),
    ("Saved Artists", "Artists"),
    ("Saved Playlists", "Saved Pl"),
    ("Recent", "Recent"),
];

pub struct LibraryView {
    pub active_tab: LibraryTab,
    pub user_playlists: Vec<UserPlaylist>,
    pub liked_songs: Vec<Track>,
    pub saved_albums: Vec<SavedCollection>,
    pub saved_artists: Vec<SavedCollection>,
    pub saved_playlists: Vec<SavedCollection>,
    pub recent_tracks: Vec<Track>,

    pub cursor: usize,
    pub grid_mode: bool,
    pub cover_size: CoverSize,
    pub last_width: u16,
    pub filter_active: bool,
    pub filter_query: String,
}

impl Default for LibraryView {
    fn default() -> Self {
        Self::new(CoverSize::M)
    }
}

impl LibraryView {
    pub fn new(cover_size: CoverSize) -> Self {
        Self {
            active_tab: LibraryTab::UserPlaylists,
            user_playlists: Vec::new(),
            liked_songs: Vec::new(),
            saved_albums: Vec::new(),
            saved_artists: Vec::new(),
            saved_playlists: Vec::new(),
            recent_tracks: Vec::new(),
            cursor: 0,
            grid_mode: false,
            cover_size,
            last_width: 80,
            filter_active: false,
            filter_query: String::new(),
        }
    }

    pub fn is_insert_mode(&self) -> bool {
        self.filter_active
    }

    pub fn footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("Tab", "sub-tab"),
            ("/", "filter"),
            ("Enter", "open/play"),
            ("p", "play"),
            ("v", "grid/list"),
            ("c", "cover size"),
            ("dd/w", "remove"),
            ("S", "sync"),
        ]
    }

    fn current_len(&self) -> usize {
        let q = self.filter_query.trim().to_lowercase();
        let matches = |s: &str| q.is_empty() || s.to_lowercase().contains(&q);
        match self.active_tab {
            LibraryTab::UserPlaylists => {
                self.user_playlists.iter().filter(|p| matches(&p.title)).count()
            }
            LibraryTab::LikedSongs => {
                self.liked_songs.iter().filter(|t| matches(&t.title)).count()
            }
            LibraryTab::SavedAlbums => {
                self.saved_albums.iter().filter(|c| matches(&c.title)).count()
            }
            LibraryTab::SavedArtists => {
                self.saved_artists.iter().filter(|c| matches(&c.title)).count()
            }
            LibraryTab::SavedPlaylists => self
                .saved_playlists
                .iter()
                .filter(|c| matches(&c.title))
                .count(),
            LibraryTab::Recent => {
                self.recent_tracks.iter().filter(|t| matches(&t.title)).count()
            }
        }
    }

    fn supports_grid(&self) -> bool {
        matches!(
            self.active_tab,
            LibraryTab::SavedAlbums | LibraryTab::SavedArtists | LibraryTab::SavedPlaylists
        )
    }

    pub fn handle_action(&mut self, action: Action) -> ViewEffect {
        if self.filter_active {
            return match action {
                Action::InputChar(ch) => {
                    self.filter_query.push(ch);
                    self.cursor = 0;
                    ViewEffect::None
                }
                Action::InputBackspace => {
                    self.filter_query.pop();
                    self.cursor = 0;
                    ViewEffect::None
                }
                Action::InputSubmit | Action::InputCancel => {
                    self.filter_active = false;
                    ViewEffect::None
                }
                _ => ViewEffect::None,
            };
        }

        let len = self.current_len();
        let grid_cols = if self.grid_mode && self.supports_grid() {
            grid_columns_for(self.last_width, self.cover_size)
        } else {
            1
        };

        match action {
            Action::NextSubTab => {
                self.active_tab = self.active_tab.next();
                self.cursor = 0;
                ViewEffect::None
            }
            Action::PrevSubTab => {
                self.active_tab = self.active_tab.prev();
                self.cursor = 0;
                ViewEffect::None
            }
            Action::StartSearch => {
                self.filter_active = true;
                ViewEffect::None
            }
            Action::Escape => {
                if !self.filter_query.is_empty() {
                    self.filter_query.clear();
                    self.cursor = 0;
                }
                ViewEffect::None
            }
            Action::ToggleGridList => {
                if self.supports_grid() {
                    self.grid_mode = !self.grid_mode;
                }
                ViewEffect::None
            }
            Action::CycleCoverSize => {
                self.cover_size = self.cover_size.cycle();
                ViewEffect::None
            }
            Action::MoveDown => {
                if len > 0 {
                    self.cursor = (self.cursor + grid_cols).min(len - 1);
                }
                ViewEffect::None
            }
            Action::MoveUp => {
                self.cursor = self.cursor.saturating_sub(grid_cols);
                ViewEffect::None
            }
            Action::MoveRight => {
                if self.grid_mode && self.supports_grid() && len > 0 {
                    self.cursor = (self.cursor + 1).min(len - 1);
                }
                ViewEffect::None
            }
            Action::MoveLeft => {
                if self.grid_mode && self.supports_grid() {
                    self.cursor = self.cursor.saturating_sub(1);
                }
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
                .map(|t| {
                    ViewEffect::Command(AppCommand::PlayNext {
                        tracks: vec![t],
                    })
                })
                .unwrap_or(ViewEffect::None),
            Action::AppendQueue => self
                .selected_track()
                .map(|t| {
                    ViewEffect::Command(AppCommand::AppendTracks {
                        tracks: vec![t],
                    })
                })
                .unwrap_or(ViewEffect::None),
            Action::AddToPlaylist => self
                .selected_track()
                .map(ViewEffect::AddToPlaylistPicker)
                .unwrap_or(ViewEffect::None),
            Action::ToggleLike => self
                .selected_track()
                .map(ViewEffect::ToggleLike)
                .unwrap_or(ViewEffect::None),
            Action::ToggleSave | Action::DeleteItem => self.remove_or_unsave_selected(),
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
            _ => ViewEffect::None,
        }
    }

    fn selected_track(&self) -> Option<Track> {
        let q = self.filter_query.trim().to_lowercase();
        let list = match self.active_tab {
            LibraryTab::LikedSongs => &self.liked_songs,
            LibraryTab::Recent => &self.recent_tracks,
            _ => return None,
        };
        list.iter()
            .filter(|t| q.is_empty() || t.title.to_lowercase().contains(&q))
            .nth(self.cursor)
            .cloned()
    }

    fn selected_collection(&self) -> Option<SavedCollection> {
        let q = self.filter_query.trim().to_lowercase();
        let list = match self.active_tab {
            LibraryTab::SavedAlbums => &self.saved_albums,
            LibraryTab::SavedArtists => &self.saved_artists,
            LibraryTab::SavedPlaylists => &self.saved_playlists,
            _ => return None,
        };
        list.iter()
            .filter(|c| q.is_empty() || c.title.to_lowercase().contains(&q))
            .nth(self.cursor)
            .cloned()
    }

    fn activate_selected(&self) -> ViewEffect {
        let q = self.filter_query.trim().to_lowercase();
        match self.active_tab {
            LibraryTab::UserPlaylists => {
                if let Some(pl) = self
                    .user_playlists
                    .iter()
                    .filter(|p| q.is_empty() || p.title.to_lowercase().contains(&q))
                    .nth(self.cursor)
                {
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
                    .filter(|t| q.is_empty() || t.title.to_lowercase().contains(&q))
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
                    .filter(|t| q.is_empty() || t.title.to_lowercase().contains(&q))
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
            LibraryTab::UserPlaylists => self.activate_selected(),
        }
    }

    fn remove_or_unsave_selected(&self) -> ViewEffect {
        match self.active_tab {
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

    pub fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        ctx: &RenderCtx<'_>,
        covers: &mut CoverRenderer,
    ) {
        if area.width < 20 || area.height < 5 {
            return;
        }
        let sub = format!("{} items", self.current_len());
        let block = panel_block("Library", Some(&sub), true);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.last_width = inner.width;

        let show_filter = self.filter_active || !self.filter_query.is_empty();
        let constraints = if show_filter {
            vec![
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ]
        } else {
            vec![Constraint::Length(1), Constraint::Min(1)]
        };
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        frame.render_widget(
            SectionTabs {
                tabs: LIBRARY_TAB_LABELS,
                active: self.active_tab.index(),
                right_hint: Some("Tab switch"),
            },
            parts[0],
        );

        let body_rect = if show_filter {
            let line = Line::from(vec![
                Span::styled("/ ", Style::default().fg(ACCENT).bg(Color::Reset)),
                Span::styled(
                    if self.filter_active {
                        format!("{}█", self.filter_query)
                    } else {
                        self.filter_query.clone()
                    },
                    Style::default()
                        .fg(TEXT)
                        .bg(if self.filter_active {
                            CURSOR_BG
                        } else {
                            Color::Reset
                        }),
                ),
            ]);
            frame.render_widget(Paragraph::new(line), parts[1]);
            parts[2]
        } else {
            parts[1]
        };

        let q = self.filter_query.trim().to_lowercase();
        let playing_vid = ctx.snapshot.track.as_ref().map(|t| t.video_id.as_str());

        match self.active_tab {
            LibraryTab::UserPlaylists => {
                let rows: Vec<TableRow> = self
                    .user_playlists
                    .iter()
                    .filter(|p| q.is_empty() || p.title.to_lowercase().contains(&q))
                    .map(|p| {
                        let mut r = TableRow::new(vec![
                            p.title.clone(),
                            if p.source == PlaylistSource::YtmSynced {
                                "YTM Synced".to_string()
                            } else {
                                "Local".to_string()
                            },
                            format!("{} tracks", p.track_count),
                        ]);
                        r.prefix_icon = Some(if p.source == PlaylistSource::YtmSynced {
                            ctx.icons.cloud
                        } else {
                            ctx.icons.local
                        });
                        r
                    })
                    .collect();
                let cols = [
                    ColumnDef {
                        title: "Playlist",
                        fixed_width: None,
                        align: Align::Left,
                    },
                    ColumnDef {
                        title: "Source",
                        fixed_width: Some(12),
                        align: Align::Left,
                    },
                    ColumnDef {
                        title: "Count",
                        fixed_width: Some(10),
                        align: Align::Right,
                    },
                ];
                frame.render_widget(
                    TrackTable {
                        columns: &cols,
                        rows: &rows,
                        cursor: self.cursor,
                        focused: true,
                        show_header: true,
                        loading: false,
                        loading_label: "",
                        empty_text: "No user playlists — open 2 Playlists and press c to create",
                        icons: ctx.icons,
                        spinner: ctx.spinner,
                    },
                    body_rect,
                );
            }
            LibraryTab::LikedSongs | LibraryTab::Recent => {
                let list = if self.active_tab == LibraryTab::LikedSongs {
                    &self.liked_songs
                } else {
                    &self.recent_tracks
                };
                let rows: Vec<TableRow> = list
                    .iter()
                    .filter(|t| q.is_empty() || t.title.to_lowercase().contains(&q))
                    .map(|t| {
                        let mut r = TableRow::new(vec![
                            t.title.clone(),
                            t.artist_names(),
                            t.duration_text(),
                        ]);
                        r.is_playing = playing_vid == Some(t.video_id.as_str());
                        r.prefix_icon = Some(if self.active_tab == LibraryTab::LikedSongs {
                            ctx.icons.liked
                        } else {
                            ctx.icons.song
                        });
                        r
                    })
                    .collect();
                let cols = [
                    ColumnDef {
                        title: "Title",
                        fixed_width: None,
                        align: Align::Left,
                    },
                    ColumnDef {
                        title: "Artist",
                        fixed_width: Some(20),
                        align: Align::Left,
                    },
                    ColumnDef {
                        title: "Time",
                        fixed_width: Some(6),
                        align: Align::Right,
                    },
                ];
                let empty = if self.active_tab == LibraryTab::LikedSongs {
                    "No liked songs yet — press f on any song to like it"
                } else {
                    "No recent plays yet (enable Save History in Settings to persist across restarts)"
                };
                frame.render_widget(
                    TrackTable {
                        columns: &cols,
                        rows: &rows,
                        cursor: self.cursor,
                        focused: true,
                        show_header: true,
                        loading: false,
                        loading_label: "",
                        empty_text: empty,
                        icons: ctx.icons,
                        spinner: ctx.spinner,
                    },
                    body_rect,
                );
            }
            LibraryTab::SavedAlbums | LibraryTab::SavedArtists | LibraryTab::SavedPlaylists => {
                let (list, glyph, empty) = match self.active_tab {
                    LibraryTab::SavedAlbums => (
                        &self.saved_albums,
                        ctx.icons.album,
                        "No saved albums — press w on an album to save it",
                    ),
                    LibraryTab::SavedArtists => (
                        &self.saved_artists,
                        ctx.icons.artist,
                        "No saved artists — press w on an artist to follow/save",
                    ),
                    _ => (
                        &self.saved_playlists,
                        ctx.icons.playlist,
                        "No saved community playlists — press w on a playlist to save it",
                    ),
                };
                let filtered: Vec<&SavedCollection> = list
                    .iter()
                    .filter(|c| q.is_empty() || c.title.to_lowercase().contains(&q))
                    .collect();

                if self.grid_mode {
                    let cards: Vec<GridCardItem> = filtered
                        .iter()
                        .map(|c| GridCardItem {
                            title: c.title.clone(),
                            subtitle: c.subtitle.clone().unwrap_or_default(),
                            cover_url: c.thumb_url.clone(),
                            glyph,
                        })
                        .collect();
                    render_cover_grid(
                        frame,
                        body_rect,
                        &cards,
                        self.cursor,
                        self.cover_size,
                        true,
                        false,
                        empty,
                        covers,
                        ctx.spinner,
                    );
                } else {
                    let rows: Vec<TableRow> = filtered
                        .iter()
                        .map(|c| {
                            let mut r = TableRow::new(vec![
                                c.title.clone(),
                                c.subtitle.clone().unwrap_or_default(),
                                match c.kind {
                                    SavedCollectionKind::Album => "Album".to_string(),
                                    SavedCollectionKind::Artist => "Artist".to_string(),
                                    SavedCollectionKind::Playlist => "Playlist".to_string(),
                                },
                            ]);
                            r.prefix_icon = Some(glyph);
                            r
                        })
                        .collect();
                    let cols = [
                        ColumnDef {
                            title: "Name",
                            fixed_width: None,
                            align: Align::Left,
                        },
                        ColumnDef {
                            title: "Details",
                            fixed_width: Some(22),
                            align: Align::Left,
                        },
                        ColumnDef {
                            title: "Kind",
                            fixed_width: Some(9),
                            align: Align::Right,
                        },
                    ];
                    frame.render_widget(
                        TrackTable {
                            columns: &cols,
                            rows: &rows,
                            cursor: self.cursor,
                            focused: true,
                            show_header: true,
                            loading: false,
                            loading_label: "",
                            empty_text: empty,
                            icons: ctx.icons,
                            spinner: ctx.spinner,
                        },
                        body_rect,
                    );
                }
            }
        }
    }
}
