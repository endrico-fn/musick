use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use musick_core::{AppCommand, Playlist, SavedCollectionKind, Track};

use crate::keymap::Action;
use crate::text::{Align, trunc_cells};
use crate::theme::{ACCENT, CURSOR_BG, DIM, TEXT, panel_block};
use crate::views::{RenderCtx, TrackSortMode, ViewEffect};
use crate::widgets::{ColumnDef, CoverRenderer, TableRow, TrackTable};

pub struct PlaylistDetailView {
    pub browse_id: String,
    pub title_hint: String,
    pub playlist: Option<Playlist>,
    pub loading: bool,
    pub saved: bool,
    pub cursor: usize,
    pub filter_active: bool,
    pub filter_query: String,
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
            filter_active: false,
            filter_query: String::new(),
            sort_mode: TrackSortMode::Original,
        }
    }

    pub fn is_insert_mode(&self) -> bool {
        self.filter_active
    }

    pub fn has_filter(&self) -> bool {
        !self.filter_query.is_empty()
    }

    pub fn set_playlist(&mut self, playlist: Playlist) {
        self.title_hint = playlist.summary.title.clone();
        self.playlist = Some(playlist);
        self.loading = false;
        self.cursor = 0;
    }

    pub fn visible_tracks(&self) -> Vec<(usize, &Track)> {
        let Some(ref pl) = self.playlist else {
            return Vec::new();
        };
        self.sort_mode.apply(&pl.tracks, &self.filter_query)
    }

    pub fn footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("Enter/p", "play"),
            ("/", "filter"),
            ("o", "sort"),
            ("N/a", "next/queue"),
            ("P/f", "add/like"),
            ("w", "save to Library"),
            ("r", "radio"),
            ("Bksp", "back"),
        ]
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
                ViewEffect::None
            }
            Action::Escape => {
                if !self.filter_query.is_empty() {
                    self.filter_query.clear();
                    self.cursor = 0;
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
            Action::CopyUrl => ViewEffect::CopyUrl(format!(
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
        let hero_h = if area.height >= 24 {
            9
        } else if area.height >= 16 {
            7
        } else {
            5
        };
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(hero_h), Constraint::Min(4)])
            .split(area);

        let max_cov_w = area.width.saturating_sub(20).min(area.width / 3).max(10);
        let cov_w = covers.square_panel_width(hero_h, max_cov_w);
        let hero_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(cov_w),
                Constraint::Length(1),
                Constraint::Min(16),
            ])
            .split(parts[0]);

        let cov_block = panel_block("Playlist", None, false);
        let cov_inner = cov_block.inner(hero_cols[0]);
        frame.render_widget(cov_block, hero_cols[0]);
        let cover_url = self.playlist.as_ref().and_then(|p| p.summary.cover_url());
        let cover_loading =
            self.loading || cover_url.as_deref().is_some_and(|u| !covers.has_image(u));
        covers.render_cover(
            frame,
            cov_inner,
            cover_url.as_deref(),
            cover_loading,
            ctx.icons.playlist,
            ctx.spinner,
        );

        let sub = if self.saved {
            "Saved in Library ✓"
        } else {
            "w save · p play"
        };
        let info_block = panel_block(&self.title_hint, Some(sub), false);
        let info_inner = info_block.inner(hero_cols[2]);
        frame.render_widget(info_block, hero_cols[2]);

        if self.loading && self.playlist.is_none() {
            frame.render_widget(
                Paragraph::new(ctx.spinner.loading_line("Loading community playlist…")),
                info_inner,
            );
        } else if let Some(ref pl) = self.playlist {
            let w = usize::from(info_inner.width.saturating_sub(2));
            let mut lines = vec![Line::from(Span::styled(
                format!(" {}", trunc_cells(&pl.summary.title, w)),
                Style::default()
                    .fg(TEXT)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            ))];
            if let Some(ref auth) = pl.summary.author {
                lines.push(Line::from(Span::styled(
                    format!(" {}", trunc_cells(auth, w)),
                    Style::default().fg(ACCENT).bg(Color::Reset),
                )));
            }
            lines.push(Line::from(Span::styled(
                format!(
                    " {} tracks{}",
                    pl.tracks.len(),
                    pl.duration_text
                        .as_deref()
                        .map(|d| format!(" · {d}"))
                        .unwrap_or_default()
                ),
                Style::default().fg(DIM).bg(Color::Reset),
            )));
            frame.render_widget(Paragraph::new(lines), info_inner);
        }

        let tr_sub = format!("{} · / filter · o sort · Bksp back", self.sort_mode.label());
        let tr_block = panel_block("Tracks", Some(&tr_sub), true);
        let tr_inner = tr_block.inner(parts[1]);
        frame.render_widget(tr_block, parts[1]);

        let (filter_rect, table_rect) = if self.filter_active || !self.filter_query.is_empty() {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(tr_inner);
            (Some(split[0]), split[1])
        } else {
            (None, tr_inner)
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
                focused: true,
                show_header: true,
                loading: self.loading,
                loading_label: "Loading playlist tracks…",
                empty_text: if self.filter_query.is_empty() {
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
