use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use musick_core::{AppCommand, Playlist, SavedCollectionKind, Track};

use crate::keymap::Action;
use crate::text::{Align, trunc_cells, wrap_cells};
use crate::theme::{DIM, TEXT, active_theme, panel_block};
use crate::views::artist::shuffle_tracks_in_place;
use crate::views::detail_hero::{DetailCover, render_detail_cover, split_detail_hero};
use crate::views::{RenderCtx, TrackSortMode, ViewEffect};
use crate::widgets::{
    ColumnDef, CoverRenderer, ScrollState, TableRow, TextInput, TrackTable, render_filter_bar,
};

pub struct PlaylistDetailView {
    pub browse_id: String,
    pub title_hint: String,
    pub playlist: Option<Playlist>,
    pub loading: bool,
    pub saved: bool,
    pub cursor: usize,
    pub scroll: ScrollState,
    pub filter_active: bool,
    pub filter: TextInput,
    pub sort_mode: TrackSortMode,
}

impl PlaylistDetailView {
    pub fn new(browse_id: String, title_hint: String, saved: bool) -> Self {
        Self {
            browse_id,
            title_hint,
            playlist: None,
            loading: true,
            saved,
            cursor: 0,
            scroll: ScrollState::new(),
            filter_active: false,
            filter: TextInput::new(),
            sort_mode: TrackSortMode::Original,
        }
    }

    pub fn is_insert_mode(&self) -> bool {
        self.filter_active
    }

    pub fn has_filter(&self) -> bool {
        !self.filter.is_empty()
    }

    pub fn set_playlist(&mut self, playlist: Playlist) {
        self.title_hint = playlist.summary.title.clone();
        self.playlist = Some(playlist);
        self.loading = false;
        self.cursor = 0;
        self.scroll.reset();
    }

    pub fn visible_tracks(&self) -> Vec<(usize, &Track)> {
        let Some(ref pl) = self.playlist else {
            return Vec::new();
        };
        self.sort_mode.apply(&pl.tracks, self.filter.as_str())
    }

    pub fn hovered_track(&self) -> Option<Track> {
        self.visible_tracks()
            .get(self.cursor)
            .map(|(_, t)| (*t).clone())
    }

    pub fn footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        if self.filter_active {
            &[("Enter", "apply filter"), ("Esc", "close filter")]
        } else {
            &[
                ("Enter/p", "play"),
                ("x", "shuffle"),
                ("N/q", "next/queue"),
                ("A/a", "artist/album"),
                ("P/f/w", "add/like/save"),
                ("e", "menu"),
                ("Bksp", "back"),
            ]
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

        let visible = self.visible_tracks();
        let len = visible.len();
        let selected = visible.get(self.cursor).map(|(_, t)| (*t).clone());
        let all_visible: Vec<Track> = visible.into_iter().map(|(_, t)| t.clone()).collect();

        match action {
            Action::StartSearch => {
                self.filter_active = true;
                ViewEffect::None
            }
            Action::CycleSort => {
                self.sort_mode = self.sort_mode.cycle();
                self.cursor = 0;
                self.scroll.reset();
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
            Action::Activate | Action::PlayDirect => {
                if !all_visible.is_empty() {
                    let start = if matches!(action, Action::PlayDirect) {
                        0
                    } else {
                        self.cursor.min(all_visible.len() - 1)
                    };
                    return ViewEffect::Command(AppCommand::PlayTracks {
                        tracks: all_visible,
                        start_index: start,
                        autoplay: true,
                    });
                }
                ViewEffect::None
            }
            Action::ArtistShuffle => {
                if !all_visible.is_empty() {
                    let mut shuffled = all_visible;
                    shuffle_tracks_in_place(&mut shuffled);
                    return ViewEffect::Command(AppCommand::PlayTracks {
                        tracks: shuffled,
                        start_index: 0,
                        autoplay: true,
                    });
                }
                ViewEffect::None
            }
            Action::PlayNext => selected
                .map(|t| ViewEffect::Command(AppCommand::PlayNext { tracks: vec![t] }))
                .unwrap_or(ViewEffect::None),
            Action::AppendQueue => selected
                .map(|t| ViewEffect::Command(AppCommand::AppendTracks { tracks: vec![t] }))
                .unwrap_or(ViewEffect::None),
            Action::AddToPlaylist => selected
                .map(ViewEffect::AddToPlaylistPicker)
                .unwrap_or(ViewEffect::None),
            Action::ToggleLike => selected
                .map(ViewEffect::ToggleLike)
                .unwrap_or(ViewEffect::None),
            Action::ToggleSave => {
                let (title, subtitle, thumb) = match &self.playlist {
                    Some(p) => (
                        p.summary.title.clone(),
                        p.summary.author.clone(),
                        p.summary.cover_url(),
                    ),
                    None => (self.title_hint.clone(), None, None),
                };
                ViewEffect::ToggleSave {
                    kind: SavedCollectionKind::Playlist,
                    browse_id: self.browse_id.clone(),
                    title,
                    subtitle,
                    thumb_url: thumb,
                }
            }
            Action::StartRadio => selected
                .or_else(|| all_visible.first().cloned())
                .map(|t| {
                    ViewEffect::Command(AppCommand::StartRadio {
                        video_id: Some(t.video_id.clone()),
                        playlist_id: None,
                        seed_track: Some(t),
                    })
                })
                .unwrap_or(ViewEffect::None),
            Action::CopyUrl => {
                if let Some(t) = selected {
                    ViewEffect::CopyUrl(t.url())
                } else {
                    ViewEffect::CopyUrl(format!(
                        "https://music.youtube.com/playlist?list={}",
                        self.browse_id
                    ))
                }
            }
            Action::CopyCollectionUrl => ViewEffect::CopyUrl(format!(
                "https://music.youtube.com/playlist?list={}",
                self.browse_id
            )),
            _ => ViewEffect::None,
        }
    }

    pub fn render(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        ctx: &RenderCtx<'_>,
        covers: &mut CoverRenderer,
    ) {
        if area.width < 20 || area.height < 8 {
            return;
        }
        let hero = split_detail_hero(area, covers);

        let cover_url = self.playlist.as_ref().and_then(|p| p.summary.cover_url());
        render_detail_cover(
            frame,
            DetailCover {
                area: hero.cover_area,
                title: "Playlist",
                url: cover_url.as_deref(),
                entity_loading: self.loading,
                placeholder_glyph: ctx.icons.playlist,
            },
            ctx.spinner,
            covers,
        );

        let sub = if self.saved {
            "Saved in Library ✓"
        } else {
            "w save · p play"
        };
        let info_block = panel_block("Details", Some(sub), false);
        let info_inner = info_block.inner(hero.info_area);
        frame.render_widget(info_block, hero.info_area);

        if self.loading && self.playlist.is_none() {
            frame.render_widget(
                Paragraph::new(ctx.spinner.loading_line("Loading community playlist…")),
                info_inner,
            );
        } else if let Some(ref pl) = self.playlist {
            let w = usize::from(info_inner.width);
            let max_lines = usize::from(info_inner.height);
            let mut lines = vec![Line::from(Span::styled(
                trunc_cells(&pl.summary.title, w),
                Style::default()
                    .fg(TEXT)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            ))];
            if let Some(ref auth) = pl.summary.author {
                lines.push(Line::from(Span::styled(
                    trunc_cells(auth, w),
                    Style::default().fg(active_theme().accent).bg(Color::Reset),
                )));
            }
            let mut meta = Vec::new();
            if let Some(ref views) = pl.views
                && !views.is_empty()
            {
                meta.push(views.clone());
            }
            meta.push(format!("{} tracks", pl.tracks.len()));
            if let Some(ref dur) = pl.duration_text
                && !dur.is_empty()
            {
                meta.push(dur.clone());
            }
            if let Some(ref yr) = pl.year
                && !yr.is_empty()
            {
                meta.push(yr.clone());
            }
            lines.push(Line::from(Span::styled(
                trunc_cells(&meta.join(" · "), w),
                Style::default().fg(DIM).bg(Color::Reset),
            )));
            if let Some(ref desc) = pl.description {
                let prose: String = desc.split_whitespace().collect::<Vec<_>>().join(" ");
                let rem = max_lines.saturating_sub(lines.len());
                if !prose.is_empty() && rem > 0 {
                    for ln in wrap_cells(&prose, w).into_iter().take(rem) {
                        lines.push(Line::from(Span::styled(
                            ln,
                            Style::default().fg(DIM).bg(Color::Reset),
                        )));
                    }
                }
            }
            frame.render_widget(Paragraph::new(lines), info_inner);
        }

        let tr_sub = format!("{} · / filter · o sort · Bksp back", self.sort_mode.label());
        let tr_block = panel_block("Tracks", Some(&tr_sub), false);
        let tr_inner = tr_block.inner(hero.body_area);
        frame.render_widget(tr_block, hero.body_area);

        let (filter_rect, table_rect) = if self.filter_active || !self.filter.is_empty() {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(tr_inner);
            (Some(split[0]), split[1])
        } else {
            (None, tr_inner)
        };

        if let Some(fr) = filter_rect {
            render_filter_bar(frame, fr, &self.filter, self.filter_active, "/ ", "");
        }

        let playing_vid = ctx.snapshot.track.as_ref().map(|t| t.video_id.as_str());
        let visible = self.visible_tracks();
        let rows: Vec<TableRow> = visible
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

        let cols = [
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
                columns: &cols,
                rows: &rows,
                cursor: self.cursor,
                scroll: Some(&self.scroll),
                focused: true,
                show_header: true,
                loading: self.loading,
                loading_label: "Loading playlist tracks…",
                empty_text: if self.filter.is_empty() {
                    "Playlist is empty"
                } else {
                    "No tracks match filter"
                },
                icons: ctx.icons,
                spinner: ctx.spinner,
            },
            table_rect,
        );
    }
}
