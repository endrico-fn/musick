use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use musick_core::{AlbumSummary, AppCommand, Artist, CoverSize, SavedCollectionKind, Track};

use crate::keymap::Action;
use crate::text::{Align, trunc_cells, wrap_cells};
use crate::theme::{ACCENT, BORDER_DIM, DIM, TEXT, panel_block};
use crate::views::{RenderCtx, ViewEffect};
use crate::widgets::{
    ColumnDef, CoverRenderer, GridCardItem, MediaListItem, SectionTabs, TableRow, TrackTable,
    grid_columns_for, render_cover_grid, render_media_list,
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
            Action::CopyUrl => ViewEffect::CopyUrl(self.selected_url()),
            _ => ViewEffect::None,
        }
    }

    fn selected_url(&self) -> String {
        if let Some(ref art) = self.artist {
            match self.active_tab {
                ArtistTab::Overview | ArtistTab::Songs => {
                    if let Some(t) = art.songs.get(self.cursor) {
                        return t.url();
                    }
                }
                ArtistTab::Videos => {
                    if let Some(t) = art.videos.get(self.cursor) {
                        return t.url();
                    }
                }
                ArtistTab::Albums => {
                    if let Some(al) = self.sorted_releases(false).get(self.cursor) {
                        return format!("https://music.youtube.com/browse/{}", al.browse_id);
                    }
                }
                ArtistTab::Singles => {
                    if let Some(al) = self.sorted_releases(true).get(self.cursor) {
                        return format!("https://music.youtube.com/browse/{}", al.browse_id);
                    }
                }
                ArtistTab::Playlists => {
                    if let Some(pl) = art.playlists.get(self.cursor) {
                        return format!("https://music.youtube.com/playlist?list={}", pl.browse_id);
                    }
                }
                ArtistTab::Similar => {
                    if let Some(rel) = art.related.get(self.cursor) {
                        return format!("https://music.youtube.com/channel/{}", rel.browse_id);
                    }
                }
            }
        }
        format!("https://music.youtube.com/channel/{}", self.browse_id)
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
        let hero_h = if area.height >= 24 {
            9
        } else if area.height >= 16 {
            7
        } else {
            5
        };
        let v_parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(hero_h), Constraint::Min(4)])
            .split(area);

        // Hero Row: True 1:1 Square Cover + Artist Info
        let max_cov_w = area.width.saturating_sub(20).min(area.width / 3).max(10);
        let cov_w = covers.square_panel_width(hero_h, max_cov_w);
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
        let cover_loading =
            self.loading || cover_url.as_deref().is_some_and(|u| !covers.has_image(u));
        covers.render_cover(
            frame,
            cov_inner,
            cover_url.as_deref(),
            cover_loading,
            ctx.icons.artist,
            ctx.spinner,
        );

        let info_sub = if self.saved {
            "Saved ✓"
        } else {
            "w follow · r radio"
        };
        let info_block = panel_block("Profile", Some(info_sub), false);
        let info_inner = info_block.inner(hero_cols[2]);
        frame.render_widget(info_block, hero_cols[2]);

        if self.loading && self.artist.is_none() {
            frame.render_widget(
                Paragraph::new(ctx.spinner.loading_line("Loading artist profile…")),
                info_inner,
            );
        } else if let Some(ref art) = self.artist {
            let w = usize::from(info_inner.width);
            let max_lines = usize::from(info_inner.height);
            let mut lines = vec![Line::from(Span::styled(
                trunc_cells(&art.summary.name, w),
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
                if s.to_ascii_lowercase().contains("subscriber") {
                    stats.push(s.clone());
                } else {
                    stats.push(format!("{s} subscribers"));
                }
            }
            if !stats.is_empty() && lines.len() < max_lines {
                lines.push(Line::from(Span::styled(
                    trunc_cells(&stats.join(" · "), w),
                    Style::default().fg(ACCENT).bg(Color::Reset),
                )));
            }
            if let Some(ref desc) = art.description {
                let prose: String = desc.split_whitespace().collect::<Vec<_>>().join(" ");
                if !prose.is_empty() && lines.len() < max_lines {
                    if max_lines.saturating_sub(lines.len()) >= 3 {
                        lines.push(Line::from(""));
                    }
                    let rem_rows = max_lines.saturating_sub(lines.len());
                    if rem_rows > 0 {
                        let wrapped = wrap_cells(&prose, w);
                        let clipped = wrapped.len() > rem_rows;
                        for (idx, mut ln) in wrapped.into_iter().take(rem_rows).enumerate() {
                            if clipped && idx + 1 == rem_rows {
                                let trimmed = ln.trim_end_matches(['.', ',', ' ']);
                                ln = trunc_cells(&format!("{trimmed}…"), w);
                            }
                            lines.push(Line::from(Span::styled(
                                ln,
                                Style::default().fg(DIM).bg(Color::Reset),
                            )));
                        }
                    }
                }
            }
            frame.render_widget(Paragraph::new(lines), info_inner);
        }

        // Body Panel: 7 Sub-tabs + Content
        let body_block = panel_block(
            "Discography & Tracks",
            Some("Tab switch · Bksp back"),
            false,
        );
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
                let top_songs = &art.songs[..art.songs.len().min(10)];
                if area.width >= SPLIT_BREAKPOINT && content_area.width >= 60 {
                    let cols = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Percentage(60),
                            Constraint::Length(3),
                            Constraint::Min(22),
                        ])
                        .split(content_area);

                    let left_split = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(1), Constraint::Min(1)])
                        .split(cols[0]);
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(
                            "Top Songs",
                            Style::default()
                                .fg(ACCENT)
                                .bg(Color::Reset)
                                .add_modifier(Modifier::BOLD),
                        ))),
                        left_split[0],
                    );
                    self.render_track_media_list(
                        frame,
                        left_split[1],
                        top_songs,
                        playing_vid,
                        ctx,
                        covers,
                        true,
                    );

                    // clean vertical divider with 1-cell breathing gap on both sides
                    let div_x = cols[1].x + 1;
                    let buf = frame.buffer_mut();
                    for dy in cols[1].y..cols[1].y + cols[1].height {
                        if let Some(cell) = buf.cell_mut((div_x, dy)) {
                            cell.set_symbol("│");
                            cell.set_fg(BORDER_DIM);
                            cell.set_bg(Color::Reset);
                        }
                    }

                    self.render_overview_side(frame, cols[2], art, ctx, covers);
                } else {
                    self.render_track_media_list(
                        frame,
                        content_area,
                        top_songs,
                        playing_vid,
                        ctx,
                        covers,
                        true,
                    );
                }
            }
            ArtistTab::Songs => {
                self.render_track_media_list(
                    frame,
                    content_area,
                    &art.songs,
                    playing_vid,
                    ctx,
                    covers,
                    true,
                );
            }
            ArtistTab::Videos => {
                self.render_track_media_list(
                    frame,
                    content_area,
                    &art.videos,
                    playing_vid,
                    ctx,
                    covers,
                    false,
                );
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
                let items: Vec<MediaListItem> = art
                    .related
                    .iter()
                    .map(|r| MediaListItem {
                        title: r.name.clone(),
                        subtitle: r.subtitle.clone().unwrap_or_else(|| "Artist".to_string()),
                        meta_top: "Artist".to_string(),
                        meta_bottom: String::new(),
                        cover_url: r.cover_url(),
                        glyph: ctx.icons.artist,
                        is_playing: false,
                        is_dim: false,
                    })
                    .collect();
                render_media_list(
                    frame,
                    content_area,
                    &items,
                    self.cursor,
                    true,
                    false,
                    "",
                    "No similar artists found",
                    ctx.icons,
                    ctx.spinner,
                    covers,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_track_media_list(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        tracks: &[Track],
        playing_vid: Option<&str>,
        ctx: &RenderCtx<'_>,
        covers: &mut CoverRenderer,
        numbered: bool,
    ) {
        let items: Vec<MediaListItem> = tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let title = if numbered {
                    format!("{:>2}. {}", i + 1, t.title)
                } else {
                    t.title.clone()
                };
                let subtitle = match t.album.as_deref().filter(|a| !a.is_empty()) {
                    Some(alb) if !alb.eq_ignore_ascii_case(&t.title) => {
                        format!("{} · {alb}", t.artist_names())
                    }
                    _ => t.artist_names(),
                };
                MediaListItem {
                    title,
                    subtitle,
                    meta_top: t.plays.clone().unwrap_or_default(),
                    meta_bottom: t.duration_text(),
                    cover_url: t.cover_url(),
                    glyph: if t.is_video() {
                        ctx.icons.video
                    } else {
                        ctx.icons.song
                    },
                    is_playing: playing_vid == Some(t.video_id.as_str()),
                    is_dim: !t.is_available,
                }
            })
            .collect();
        render_media_list(
            frame,
            area,
            &items,
            self.cursor,
            true,
            false,
            "",
            "No tracks available",
            ctx.icons,
            ctx.spinner,
            covers,
        );
    }

    fn render_overview_side(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        art: &Artist,
        ctx: &RenderCtx<'_>,
        covers: &mut CoverRenderer,
    ) {
        if area.width < 10 || area.height == 0 {
            return;
        }
        let w = usize::from(area.width);
        let mut y = area.y;
        let max_y = area.y + area.height;

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                trunc_cells("Latest & Top Releases", w),
                Style::default()
                    .fg(ACCENT)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            ))),
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            },
        );
        y += 1;

        let mut releases: Vec<&AlbumSummary> =
            art.albums.iter().chain(art.singles.iter()).collect();
        releases.sort_by(|a, b| {
            let ya = a.year.as_deref().unwrap_or("");
            let yb = b.year.as_deref().unwrap_or("");
            yb.cmp(ya)
        });

        let mut list_start = 0usize;
        if let Some(latest) = releases.first()
            && y + 2 <= max_y
        {
            list_start = 1;
            let show_mini = covers.is_enabled() && area.width >= 18;
            let mini_w = if show_mini {
                covers
                    .mini_cover_width(2)
                    .min(area.width.saturating_sub(10))
            } else {
                0
            };
            if show_mini {
                covers.render_mini_cover(
                    frame,
                    Rect {
                        x: area.x,
                        y,
                        width: mini_w,
                        height: 2,
                    },
                    latest.cover_url().as_deref(),
                    ctx.icons.album,
                    true,
                );
            }
            let tx = area.x + if show_mini { mini_w + 1 } else { 0 };
            let tw = usize::from(
                area.width
                    .saturating_sub(if show_mini { mini_w + 1 } else { 0 }),
            );
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(
                        trunc_cells(&latest.title, tw),
                        Style::default()
                            .fg(TEXT)
                            .bg(Color::Reset)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(Span::styled(
                        trunc_cells(&latest.subtitle(), tw),
                        Style::default().fg(DIM).bg(Color::Reset),
                    )),
                ]),
                Rect {
                    x: tx,
                    y,
                    width: tw as u16,
                    height: 2,
                },
            );
            y += 2;
        }

        let mut lines = Vec::new();
        for al in releases.iter().skip(list_start).take(4) {
            let yr = al.year.as_deref().unwrap_or(&al.kind);
            let suffix = format!(" ({yr})");
            let title_budget = w.saturating_sub(suffix.len() + 3);
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", ctx.icons.album),
                    Style::default().fg(ACCENT).bg(Color::Reset),
                ),
                Span::styled(
                    trunc_cells(&al.title, title_budget),
                    Style::default().fg(TEXT).bg(Color::Reset),
                ),
                Span::styled(suffix, Style::default().fg(DIM).bg(Color::Reset)),
            ]));
        }

        if !art.related.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                trunc_cells("Fans Also Like", w),
                Style::default()
                    .fg(ACCENT)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            )));
            for rel in art.related.iter().take(4) {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{} ", ctx.icons.artist),
                        Style::default().fg(DIM).bg(Color::Reset),
                    ),
                    Span::styled(
                        trunc_cells(&rel.name, w.saturating_sub(2)),
                        Style::default().fg(TEXT).bg(Color::Reset),
                    ),
                ]));
            }
        }

        if y < max_y && !lines.is_empty() {
            frame.render_widget(
                Paragraph::new(lines),
                Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: max_y - y,
                },
            );
        }
    }
}
