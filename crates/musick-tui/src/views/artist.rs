use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use musick_core::{AlbumSummary, AppCommand, Artist, CoverSize, SavedCollectionKind, Track};

use crate::keymap::Action;
use crate::text::{Align, trunc_cells};
use crate::theme::{ACCENT, DIM, TEXT, panel_block};
use crate::views::{RenderCtx, ViewEffect};
use crate::widgets::{
    ColumnDef, CoverRenderer, GridCardItem, SectionTabs, TableRow, TrackTable, grid_columns_for,
    render_cover_grid,
};

const SPLIT_BREAKPOINT: u16 = 106;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArtistTab {
    #[default]
    Overview,
    Songs,
    Albums,
    Singles,
    Videos,
    Playlists,
    Similar,
}

impl ArtistTab {
    pub const ALL: &'static [Self] = &[
        Self::Overview,
        Self::Songs,
        Self::Albums,
        Self::Singles,
        Self::Videos,
        Self::Playlists,
        Self::Similar,
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

const ARTIST_TABS: &[(&str, &str)] = &[
    ("Overview", "Overv."),
    ("Songs", "Songs"),
    ("Albums", "Albums"),
    ("Singles", "Singles"),
    ("Videos", "Videos"),
    ("Playlists", "Lists"),
    ("Similar", "Sim."),
];

pub struct ArtistView {
    pub browse_id: String,
    pub title_hint: String,
    pub artist: Option<Artist>,
    pub loading: bool,
    pub saved: bool,
    pub active_tab: ArtistTab,
    pub cursor: usize,
    pub grid_mode: bool,
    pub cover_size: CoverSize,
    pub sort_newest: bool,
    pub last_width: u16,
}

impl ArtistView {
    pub fn new(browse_id: String, title_hint: String, cover_size: CoverSize, saved: bool) -> Self {
        Self {
            browse_id,
            title_hint,
            artist: None,
            loading: true,
            saved,
            active_tab: ArtistTab::Overview,
            cursor: 0,
            grid_mode: true,
            cover_size,
            sort_newest: true,
            last_width: 100,
        }
    }

    pub fn set_artist(&mut self, artist: Artist) {
        self.title_hint = artist.summary.name.clone();
        self.artist = Some(artist);
        self.loading = false;
        self.cursor = 0;
    }

    pub fn footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("Tab", "sub-tab"),
            ("Enter/p", "open/play"),
            ("N/a", "next/queue"),
            ("w", "follow/save"),
            ("v/c/o", "grid/size/sort"),
            ("r", "radio"),
            ("Bksp", "back"),
        ]
    }

    fn sorted_releases(&self, singles: bool) -> Vec<AlbumSummary> {
        let Some(ref art) = self.artist else {
            return Vec::new();
        };
        let mut items = if singles {
            art.singles.clone()
        } else {
            art.albums.clone()
        };
        items.sort_by(|a, b| {
            let ya = a.year.as_deref().unwrap_or("");
            let yb = b.year.as_deref().unwrap_or("");
            if self.sort_newest {
                yb.cmp(ya)
            } else {
                ya.cmp(yb)
            }
        });
        items
    }

    fn current_len(&self) -> usize {
        let Some(ref art) = self.artist else {
            return 0;
        };
        match self.active_tab {
            ArtistTab::Overview => art.songs.len().min(10),
            ArtistTab::Songs => art.songs.len(),
            ArtistTab::Albums => art.albums.len(),
            ArtistTab::Singles => art.singles.len(),
            ArtistTab::Videos => art.videos.len(),
            ArtistTab::Playlists => art.playlists.len(),
            ArtistTab::Similar => art.related.len(),
        }
    }

    fn supports_grid(&self) -> bool {
        matches!(self.active_tab, ArtistTab::Albums | ArtistTab::Singles)
    }

    pub fn handle_action(&mut self, action: Action) -> ViewEffect {
        let len = self.current_len();
        let cols = if self.grid_mode && self.supports_grid() {
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
            Action::CycleSort => {
                self.sort_newest = !self.sort_newest;
                ViewEffect::None
            }
            Action::MoveDown => {
                if len > 0 {
                    self.cursor = (self.cursor + cols).min(len - 1);
                }
                ViewEffect::None
            }
            Action::MoveUp => {
                self.cursor = self.cursor.saturating_sub(cols);
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
            Action::ToggleSave => {
                let (title, subtitle, thumb) = match &self.artist {
                    Some(a) => (
                        a.summary.name.clone(),
                        a.summary.subtitle.clone(),
                        a.summary.cover_url(),
                    ),
                    None => (self.title_hint.clone(), None, None),
                };
                self.saved = !self.saved;
                ViewEffect::ToggleSave {
                    kind: SavedCollectionKind::Artist,
                    browse_id: self.browse_id.clone(),
                    title,
                    subtitle,
                    thumb_url: thumb,
                }
            }
            Action::StartRadio => {
                if let Some(ref art) = self.artist {
                    if let Some(ref rid) = art.radio_id {
                        return ViewEffect::Command(AppCommand::StartRadio {
                            video_id: None,
                            playlist_id: Some(rid.clone()),
                            seed_track: None,
                        });
                    }
                    if let Some(tr) = art.songs.first() {
                        return ViewEffect::Command(AppCommand::StartRadio {
                            video_id: Some(tr.video_id.clone()),
                            playlist_id: None,
                            seed_track: Some(tr.clone()),
                        });
                    }
                }
                ViewEffect::None
            }
            Action::Activate => self.activate_item(),
            Action::PlayDirect => self.play_direct(),
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
            Action::CopyUrl => ViewEffect::CopyUrl(format!(
                "https://music.youtube.com/channel/{}",
                self.browse_id
            )),
            _ => ViewEffect::None,
        }
    }

    fn selected_track(&self) -> Option<Track> {
        let art = self.artist.as_ref()?;
        match self.active_tab {
            ArtistTab::Overview | ArtistTab::Songs => art.songs.get(self.cursor).cloned(),
            ArtistTab::Videos => art.videos.get(self.cursor).cloned(),
            _ => None,
        }
    }

    fn activate_item(&self) -> ViewEffect {
        let Some(ref art) = self.artist else {
            return ViewEffect::None;
        };
        match self.active_tab {
            ArtistTab::Overview | ArtistTab::Songs => {
                if art.songs.is_empty() {
                    return ViewEffect::None;
                }
                ViewEffect::Command(AppCommand::PlayTracks {
                    tracks: art.songs.clone(),
                    start_index: self.cursor.min(art.songs.len() - 1),
                    autoplay: true,
                })
            }
            ArtistTab::Videos => {
                if art.videos.is_empty() {
                    return ViewEffect::None;
                }
                ViewEffect::Command(AppCommand::PlayTracks {
                    tracks: art.videos.clone(),
                    start_index: self.cursor.min(art.videos.len() - 1),
                    autoplay: true,
                })
            }
            ArtistTab::Albums => self
                .sorted_releases(false)
                .get(self.cursor)
                .map(|al| ViewEffect::OpenAlbum {
                    browse_id: al.browse_id.clone(),
                    title_hint: al.title.clone(),
                })
                .unwrap_or(ViewEffect::None),
            ArtistTab::Singles => self
                .sorted_releases(true)
                .get(self.cursor)
                .map(|al| ViewEffect::OpenAlbum {
                    browse_id: al.browse_id.clone(),
                    title_hint: al.title.clone(),
                })
                .unwrap_or(ViewEffect::None),
            ArtistTab::Playlists => art
                .playlists
                .get(self.cursor)
                .map(|pl| ViewEffect::OpenPlaylist {
                    browse_id: pl.browse_id.clone(),
                    title_hint: pl.title.clone(),
                })
                .unwrap_or(ViewEffect::None),
            ArtistTab::Similar => art
                .related
                .get(self.cursor)
                .map(|rel| ViewEffect::OpenArtist {
                    browse_id: rel.browse_id.clone(),
                    name_hint: rel.name.clone(),
                })
                .unwrap_or(ViewEffect::None),
        }
    }

    fn play_direct(&self) -> ViewEffect {
        match self.active_tab {
            ArtistTab::Albums => self
                .sorted_releases(false)
                .get(self.cursor)
                .map(|al| ViewEffect::PlayAlbumDirect {
                    browse_id: al.browse_id.clone(),
                })
                .unwrap_or(ViewEffect::None),
            ArtistTab::Singles => self
                .sorted_releases(true)
                .get(self.cursor)
                .map(|al| ViewEffect::PlayAlbumDirect {
                    browse_id: al.browse_id.clone(),
                })
                .unwrap_or(ViewEffect::None),
            ArtistTab::Playlists => self
                .artist
                .as_ref()
                .and_then(|a| a.playlists.get(self.cursor))
                .map(|pl| ViewEffect::PlayPlaylistDirect {
                    browse_id: pl.browse_id.clone(),
                })
                .unwrap_or(ViewEffect::None),
            _ => self.activate_item(),
        }
    }

    pub fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        ctx: &RenderCtx<'_>,
        covers: &mut CoverRenderer,
    ) {
        if area.width < 20 || area.height < 8 {
            return;
        }
        let hero_h = if area.height >= 20 { 7 } else { 5 };
        let v_parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(hero_h), Constraint::Min(4)])
            .split(area);

        // Hero Row: Cover + Artist Info
        let cov_w = hero_h.saturating_mul(2).min(16);
        let hero_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(cov_w),
                Constraint::Length(1),
                Constraint::Min(16),
            ])
            .split(v_parts[0]);

        let cov_block = panel_block("Artist", None, false);
        let cov_inner = cov_block.inner(hero_cols[0]);
        frame.render_widget(cov_block, hero_cols[0]);
        let cover_url = self.artist.as_ref().and_then(|a| a.summary.cover_url());
        covers.render_cover(
            frame,
            cov_inner,
            cover_url.as_deref(),
            self.loading,
            ctx.icons.artist,
            ctx.spinner,
        );

        let info_sub = if self.saved {
            "Saved ✓"
        } else {
            "w save · r radio"
        };
        let info_block = panel_block(&self.title_hint, Some(info_sub), false);
        let info_inner = info_block.inner(hero_cols[2]);
        frame.render_widget(info_block, hero_cols[2]);

        if self.loading && self.artist.is_none() {
            frame.render_widget(
                Paragraph::new(ctx.spinner.loading_line("Loading artist profile…")),
                info_inner,
            );
        } else if let Some(ref art) = self.artist {
            let w = usize::from(info_inner.width.saturating_sub(2));
            let mut lines = vec![Line::from(Span::styled(
                format!(" {}", trunc_cells(&art.summary.name, w)),
                Style::default()
                    .fg(TEXT)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            ))];
            let mut stats = Vec::new();
            if let Some(ref l) = art.listeners {
                stats.push(l.clone());
            }
            if let Some(ref s) = art.subscribers {
                stats.push(format!("{s} subscribers"));
            }
            if !stats.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!(" {}", trunc_cells(&stats.join(" · "), w)),
                    Style::default().fg(ACCENT).bg(Color::Reset),
                )));
            }
            if let Some(ref desc) = art.description {
                let first_line = desc.lines().next().unwrap_or("");
                lines.push(Line::from(Span::styled(
                    format!(" {}", trunc_cells(first_line, w)),
                    Style::default().fg(DIM).bg(Color::Reset),
                )));
            }
            frame.render_widget(Paragraph::new(lines), info_inner);
        }

        // Body Panel: 7 Sub-tabs + Content
        let body_block = panel_block("Discography & Tracks", Some("Tab switch · Bksp back"), true);
        let body_inner = body_block.inner(v_parts[1]);
        frame.render_widget(body_block, v_parts[1]);
        self.last_width = body_inner.width;

        if body_inner.height < 2 || body_inner.width < 10 {
            return;
        }
        let body_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(body_inner);

        frame.render_widget(
            SectionTabs {
                tabs: ARTIST_TABS,
                active: self.active_tab.index(),
                right_hint: if self.supports_grid() {
                    Some(if self.grid_mode { "Grid" } else { "List" })
                } else {
                    None
                },
            },
            body_split[0],
        );

        let content_area = body_split[1];
        if self.loading && self.artist.is_none() {
            frame.render_widget(
                Paragraph::new(ctx.spinner.loading_line("Fetching catalog…")),
                content_area,
            );
            return;
        }

        let Some(ref art) = self.artist else {
            return;
        };
        let playing_vid = ctx.snapshot.track.as_ref().map(|t| t.video_id.as_str());

        match self.active_tab {
            ArtistTab::Overview => {
                // Responsive split overview at >= 106 columns!
                let top_songs = &art.songs[..art.songs.len().min(10)];
                if area.width >= SPLIT_BREAKPOINT && content_area.width >= 60 {
                    let cols = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Percentage(58),
                            Constraint::Length(1),
                            Constraint::Min(20),
                        ])
                        .split(content_area);
                    self.render_track_list(frame, cols[0], top_songs, playing_vid, ctx);
                    self.render_overview_side(frame, cols[2], art);
                } else {
                    self.render_track_list(frame, content_area, top_songs, playing_vid, ctx);
                }
            }
            ArtistTab::Songs => {
                self.render_track_list(frame, content_area, &art.songs, playing_vid, ctx);
            }
            ArtistTab::Videos => {
                self.render_track_list(frame, content_area, &art.videos, playing_vid, ctx);
            }
            ArtistTab::Albums | ArtistTab::Singles => {
                let items = self.sorted_releases(self.active_tab == ArtistTab::Singles);
                if self.grid_mode {
                    let cards: Vec<GridCardItem> = items
                        .iter()
                        .map(|al| GridCardItem {
                            title: al.title.clone(),
                            subtitle: al.subtitle(),
                            cover_url: al.cover_url(),
                            glyph: ctx.icons.album,
                        })
                        .collect();
                    render_cover_grid(
                        frame,
                        content_area,
                        &cards,
                        self.cursor,
                        self.cover_size,
                        true,
                        false,
                        "No releases found",
                        covers,
                        ctx.spinner,
                    );
                } else {
                    let rows: Vec<TableRow> = items
                        .iter()
                        .map(|al| {
                            let mut r = TableRow::new(vec![
                                al.title.clone(),
                                al.kind.clone(),
                                al.year.clone().unwrap_or_default(),
                            ]);
                            r.prefix_icon = Some(ctx.icons.album);
                            r
                        })
                        .collect();
                    let cols = [
                        ColumnDef {
                            title: "Release",
                            fixed_width: None,
                            align: Align::Left,
                        },
                        ColumnDef {
                            title: "Kind",
                            fixed_width: Some(8),
                            align: Align::Left,
                        },
                        ColumnDef {
                            title: "Year",
                            fixed_width: Some(6),
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
                            empty_text: "No releases found",
                            icons: ctx.icons,
                            spinner: ctx.spinner,
                        },
                        content_area,
                    );
                }
            }
            ArtistTab::Playlists => {
                let rows: Vec<TableRow> = art
                    .playlists
                    .iter()
                    .map(|p| {
                        let mut r = TableRow::new(vec![
                            p.title.clone(),
                            p.author.clone().unwrap_or_default(),
                            p.count.clone().unwrap_or_default(),
                        ]);
                        r.prefix_icon = Some(ctx.icons.playlist);
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
                        title: "Author",
                        fixed_width: Some(18),
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
                        empty_text: "No playlists featured",
                        icons: ctx.icons,
                        spinner: ctx.spinner,
                    },
                    content_area,
                );
            }
            ArtistTab::Similar => {
                let rows: Vec<TableRow> = art
                    .related
                    .iter()
                    .map(|r| {
                        let mut row = TableRow::new(vec![
                            r.name.clone(),
                            r.subtitle.clone().unwrap_or_default(),
                        ]);
                        row.prefix_icon = Some(ctx.icons.artist);
                        row
                    })
                    .collect();
                let cols = [
                    ColumnDef {
                        title: "Artist",
                        fixed_width: None,
                        align: Align::Left,
                    },
                    ColumnDef {
                        title: "Audience",
                        fixed_width: Some(20),
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
                        empty_text: "No similar artists found",
                        icons: ctx.icons,
                        spinner: ctx.spinner,
                    },
                    content_area,
                );
            }
        }
    }

    fn render_track_list(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        tracks: &[Track],
        playing_vid: Option<&str>,
        ctx: &RenderCtx<'_>,
    ) {
        let rows: Vec<TableRow> = tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mut r = TableRow::new(vec![
                    format!("{:>2}. {}", i + 1, t.title),
                    t.plays.clone().unwrap_or_default(),
                    t.duration_text(),
                ]);
                r.is_playing = playing_vid == Some(t.video_id.as_str());
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
                title: "Plays",
                fixed_width: Some(8),
                align: Align::Right,
            },
            ColumnDef {
                title: "Time",
                fixed_width: Some(6),
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
                empty_text: "No tracks available",
                icons: ctx.icons,
                spinner: ctx.spinner,
            },
            area,
        );
    }

    fn render_overview_side(&self, frame: &mut Frame<'_>, area: Rect, art: &Artist) {
        let w = usize::from(area.width.saturating_sub(2));
        let mut lines = vec![Line::from(Span::styled(
            "Latest & Top Releases",
            Style::default()
                .fg(ACCENT)
                .bg(Color::Reset)
                .add_modifier(Modifier::BOLD),
        ))];
        for al in art.albums.iter().chain(art.singles.iter()).take(5) {
            lines.push(Line::from(vec![
                Span::styled("  ◎ ", Style::default().fg(ACCENT).bg(Color::Reset)),
                Span::styled(
                    trunc_cells(&al.title, w.saturating_sub(10)),
                    Style::default().fg(TEXT).bg(Color::Reset),
                ),
                Span::styled(
                    format!(" ({})", al.year.as_deref().unwrap_or(&al.kind)),
                    Style::default().fg(DIM).bg(Color::Reset),
                ),
            ]));
        }
        if !art.related.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Fans Also Like",
                Style::default()
                    .fg(ACCENT)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            )));
            for rel in art.related.iter().take(4) {
                lines.push(Line::from(Span::styled(
                    format!("  ◉ {}", trunc_cells(&rel.name, w.saturating_sub(4))),
                    Style::default().fg(DIM).bg(Color::Reset),
                )));
            }
        }
        frame.render_widget(Paragraph::new(lines), area);
    }
}
