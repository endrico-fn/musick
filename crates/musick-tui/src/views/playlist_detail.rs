use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use musick_core::{AppCommand, Playlist, SavedCollectionKind};

use crate::keymap::Action;
use crate::text::{Align, trunc_cells};
use crate::theme::{ACCENT, DIM, TEXT, panel_block};
use crate::views::{RenderCtx, ViewEffect};
use crate::widgets::{ColumnDef, CoverRenderer, TableRow, TrackTable};

pub struct PlaylistDetailView {
    pub browse_id: String,
    pub title_hint: String,
    pub playlist: Option<Playlist>,
    pub loading: bool,
    pub saved: bool,
    pub cursor: usize,
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
        }
    }

    pub fn set_playlist(&mut self, playlist: Playlist) {
        self.title_hint = playlist.summary.title.clone();
        self.playlist = Some(playlist);
        self.loading = false;
        self.cursor = 0;
    }

    pub fn footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("Enter/p", "play"),
            ("N/a", "next/queue"),
            ("P/f", "add/like"),
            ("w", "save to Library"),
            ("r", "radio"),
            ("yy", "copy URL"),
            ("Bksp", "back"),
        ]
    }

    pub fn handle_action(&mut self, action: Action) -> ViewEffect {
        let len = self.playlist.as_ref().map(|p| p.tracks.len()).unwrap_or(0);
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
                if let Some(ref pl) = self.playlist
                    && !pl.tracks.is_empty()
                {
                    let start = if matches!(action, Action::PlayDirect) {
                        0
                    } else {
                        self.cursor.min(pl.tracks.len() - 1)
                    };
                    return ViewEffect::Command(AppCommand::PlayTracks {
                        tracks: pl.tracks.clone(),
                        start_index: start,
                        autoplay: true,
                    });
                }
                ViewEffect::None
            }
            Action::PlayNext => self
                .playlist
                .as_ref()
                .and_then(|p| p.tracks.get(self.cursor))
                .map(|t| {
                    ViewEffect::Command(AppCommand::PlayNext {
                        tracks: vec![t.clone()],
                    })
                })
                .unwrap_or(ViewEffect::None),
            Action::AppendQueue => self
                .playlist
                .as_ref()
                .and_then(|p| p.tracks.get(self.cursor))
                .map(|t| {
                    ViewEffect::Command(AppCommand::AppendTracks {
                        tracks: vec![t.clone()],
                    })
                })
                .unwrap_or(ViewEffect::None),
            Action::AddToPlaylist => self
                .playlist
                .as_ref()
                .and_then(|p| p.tracks.get(self.cursor))
                .map(|t| ViewEffect::AddToPlaylistPicker(t.clone()))
                .unwrap_or(ViewEffect::None),
            Action::ToggleLike => self
                .playlist
                .as_ref()
                .and_then(|p| p.tracks.get(self.cursor))
                .map(|t| ViewEffect::ToggleLike(t.clone()))
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
                self.saved = !self.saved;
                ViewEffect::ToggleSave {
                    kind: SavedCollectionKind::Playlist,
                    browse_id: self.browse_id.clone(),
                    title,
                    subtitle,
                    thumb_url: thumb,
                }
            }
            Action::StartRadio => self
                .playlist
                .as_ref()
                .and_then(|p| p.tracks.get(self.cursor).or_else(|| p.tracks.first()))
                .map(|t| {
                    ViewEffect::Command(AppCommand::StartRadio {
                        video_id: Some(t.video_id.clone()),
                        playlist_id: None,
                        seed_track: Some(t.clone()),
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

        let cov_block = panel_block("Playlist", None, false);
        let cov_inner = cov_block.inner(hero_cols[0]);
        frame.render_widget(cov_block, hero_cols[0]);
        let cover_url = self.playlist.as_ref().and_then(|p| p.summary.cover_url());
        covers.render_cover(
            frame,
            cov_inner,
            cover_url.as_deref(),
            self.loading,
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

        let tr_block = panel_block("Tracks", Some("Bksp back"), true);
        let tr_inner = tr_block.inner(parts[1]);
        frame.render_widget(tr_block, parts[1]);

        let playing_vid = ctx.snapshot.track.as_ref().map(|t| t.video_id.as_str());
        let rows: Vec<TableRow> = self
            .playlist
            .as_ref()
            .map(|pl| {
                pl.tracks
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let mut r = TableRow::new(vec![
                            format!("{:>2}. {}", i + 1, t.title),
                            t.artist_names(),
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
                empty_text: "Playlist is empty",
                icons: ctx.icons,
                spinner: ctx.spinner,
            },
            tr_inner,
        );
    }
}
