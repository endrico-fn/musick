use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use musick_core::{Album, AppCommand, SavedCollectionKind};

use crate::keymap::Action;
use crate::text::{Align, trunc_cells};
use crate::theme::{ACCENT, DIM, TEXT, panel_block};
use crate::views::{RenderCtx, ViewEffect};
use crate::widgets::{ColumnDef, CoverRenderer, TableRow, TrackTable};

pub struct AlbumView {
    pub browse_id: String,
    pub title_hint: String,
    pub album: Option<Album>,
    pub loading: bool,
    pub saved: bool,
    pub cursor: usize,
}

impl AlbumView {
    pub fn new(browse_id: String, title_hint: String, saved: bool) -> Self {
        Self {
            browse_id,
            title_hint,
            album: None,
            loading: true,
            saved,
            cursor: 0,
        }
    }

    pub fn set_album(&mut self, album: Album) {
        self.title_hint = album.summary.title.clone();
        self.album = Some(album);
        self.loading = false;
        self.cursor = 0;
    }

    pub fn footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("Enter/p", "play"),
            ("N/a", "next/queue"),
            ("P/f", "list/like"),
            ("w", "save album"),
            ("r", "radio"),
            ("yy", "copy URL"),
            ("Bksp", "back"),
        ]
    }

    pub fn handle_action(&mut self, action: Action) -> ViewEffect {
        let len = self.album.as_ref().map(|a| a.tracks.len()).unwrap_or(0);
        match action {
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
                if let Some(ref alb) = self.album
                    && !alb.tracks.is_empty()
                {
                    let start = if matches!(action, Action::PlayDirect) {
                        0
                    } else {
                        self.cursor.min(alb.tracks.len() - 1)
                    };
                    return ViewEffect::Command(AppCommand::PlayTracks {
                        tracks: alb.tracks.clone(),
                        start_index: start,
                        autoplay: true,
                    });
                }
                ViewEffect::None
            }
            Action::PlayNext => self
                .album
                .as_ref()
                .and_then(|a| a.tracks.get(self.cursor))
                .map(|t| {
                    ViewEffect::Command(AppCommand::PlayNext {
                        tracks: vec![t.clone()],
                    })
                })
                .unwrap_or(ViewEffect::None),
            Action::AppendQueue => self
                .album
                .as_ref()
                .and_then(|a| a.tracks.get(self.cursor))
                .map(|t| {
                    ViewEffect::Command(AppCommand::AppendTracks {
                        tracks: vec![t.clone()],
                    })
                })
                .unwrap_or(ViewEffect::None),
            Action::AddToPlaylist => self
                .album
                .as_ref()
                .and_then(|a| a.tracks.get(self.cursor))
                .map(|t| ViewEffect::AddToPlaylistPicker(t.clone()))
                .unwrap_or(ViewEffect::None),
            Action::ToggleLike => self
                .album
                .as_ref()
                .and_then(|a| a.tracks.get(self.cursor))
                .map(|t| ViewEffect::ToggleLike(t.clone()))
                .unwrap_or(ViewEffect::None),
            Action::ToggleSave => {
                let (title, subtitle, thumb) = match &self.album {
                    Some(a) => (
                        a.summary.title.clone(),
                        Some(a.summary.subtitle()),
                        a.summary.cover_url(),
                    ),
                    None => (self.title_hint.clone(), None, None),
                };
                self.saved = !self.saved;
                ViewEffect::ToggleSave {
                    kind: SavedCollectionKind::Album,
                    browse_id: self.browse_id.clone(),
                    title,
                    subtitle,
                    thumb_url: thumb,
                }
            }
            Action::StartRadio => self
                .album
                .as_ref()
                .and_then(|a| a.tracks.get(self.cursor).or_else(|| a.tracks.first()))
                .map(|t| {
                    ViewEffect::Command(AppCommand::StartRadio {
                        video_id: Some(t.video_id.clone()),
                        playlist_id: None,
                        seed_track: Some(t.clone()),
                    })
                })
                .unwrap_or(ViewEffect::None),
            Action::CopyUrl => ViewEffect::CopyUrl(format!(
                "https://music.youtube.com/browse/{}",
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
        let hero_h = if area.height >= 20 { 7 } else { 5 };
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(hero_h), Constraint::Min(4)])
            .split(area);

        let cov_w = hero_h.saturating_mul(2).min(16);
        let hero_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(cov_w),
                Constraint::Length(1),
                Constraint::Min(16),
            ])
            .split(parts[0]);

        let cov_block = panel_block("Album", None, false);
        let cov_inner = cov_block.inner(hero_cols[0]);
        frame.render_widget(cov_block, hero_cols[0]);
        let cover_url = self.album.as_ref().and_then(|a| a.summary.cover_url());
        let cover_loading =
            self.loading || cover_url.as_deref().is_some_and(|u| !covers.has_image(u));
        covers.render_cover(
            frame,
            cov_inner,
            cover_url.as_deref(),
            cover_loading,
            ctx.icons.album,
            ctx.spinner,
        );

        let sub = if self.saved {
            "Saved ✓"
        } else {
            "w save · p play"
        };
        let info_block = panel_block(&self.title_hint, Some(sub), false);
        let info_inner = info_block.inner(hero_cols[2]);
        frame.render_widget(info_block, hero_cols[2]);

        if self.loading && self.album.is_none() {
            frame.render_widget(
                Paragraph::new(ctx.spinner.loading_line("Loading album…")),
                info_inner,
            );
        } else if let Some(ref alb) = self.album {
            let w = usize::from(info_inner.width.saturating_sub(2));
            let artists = alb
                .summary
                .artists
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let mut lines = vec![
                Line::from(Span::styled(
                    format!(" {}", trunc_cells(&alb.summary.title, w)),
                    Style::default()
                        .fg(TEXT)
                        .bg(Color::Reset)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!(" {}", trunc_cells(&artists, w)),
                    Style::default().fg(ACCENT).bg(Color::Reset),
                )),
                Line::from(Span::styled(
                    format!(" {} · {} tracks", alb.summary.subtitle(), alb.tracks.len()),
                    Style::default().fg(DIM).bg(Color::Reset),
                )),
            ];
            if let Some(ref dur) = alb.duration_text {
                lines.push(Line::from(Span::styled(
                    format!(" {}", trunc_cells(dur, w)),
                    Style::default().fg(DIM).bg(Color::Reset),
                )));
            }
            frame.render_widget(Paragraph::new(lines), info_inner);
        }

        let tr_block = panel_block("Tracks", Some("Bksp back"), true);
        let tr_inner = tr_block.inner(parts[1]);
        frame.render_widget(tr_block, parts[1]);

        let playing_vid = ctx.snapshot.track.as_ref().map(|t| t.video_id.as_str());
        let rows: Vec<TableRow> = self
            .album
            .as_ref()
            .map(|alb| {
                alb.tracks
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
                    .collect()
            })
            .unwrap_or_default();

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
                loading: self.loading,
                loading_label: "Loading album tracks…",
                empty_text: "No tracks found in album",
                icons: ctx.icons,
                spinner: ctx.spinner,
            },
            tr_inner,
        );
    }
}
