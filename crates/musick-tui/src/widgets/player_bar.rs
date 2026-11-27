use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use musick_core::{
    PlaybackSnapshot, PositionTick, RepeatMode, format_duration, format_duration_allow_zero,
};

use crate::text::{cell_width, trunc_cells};
use crate::theme::{BORDER, DIM, ERR, IconSet, TEXT, WARN_GOLD, active_theme, panel_block};
use crate::widgets::BrailleSpinner;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayerBarControl {
    Prev,
    PlayPause,
    Next,
    Shuffle,
    Repeat,
    Volume,
    #[default]
    Seekbar,
}

impl PlayerBarControl {
    const ALL: &'static [Self] = &[
        Self::Prev,
        Self::PlayPause,
        Self::Next,
        Self::Shuffle,
        Self::Repeat,
        Self::Volume,
        Self::Seekbar,
    ];

    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|&c| c == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|&c| c == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerBarHitboxes {
    pub bar_area: Rect,
    pub prev: Rect,
    pub play_pause: Rect,
    pub next: Rect,
    pub shuffle: Rect,
    pub repeat: Rect,
    pub volume_mute: Rect,
    pub volume_bar: Rect,
    pub seekbar: Rect,
}

pub fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    rect.width > 0
        && rect.height > 0
        && col >= rect.x
        && col < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

pub struct NowPlayingBar<'a> {
    pub snapshot: &'a PlaybackSnapshot,
    pub position: PositionTick,
    pub icons: IconSet,
    pub spinner: BrailleSpinner,
    pub focused: bool,
}

impl NowPlayingBar<'_> {
    fn subtitle_text(&self) -> String {
        if self.snapshot.resolving {
            return self.spinner.inline_text("Resolving");
        }
        if self.snapshot.error.is_some() {
            return String::new();
        }
        if self.position.buffering {
            return self.spinner.inline_text("Buffering");
        }
        if self.snapshot.restored && self.snapshot.track.is_some() {
            return "Last played".to_string();
        }
        let mut badges = Vec::new();
        if !self.snapshot.audio_info.is_empty() {
            badges.push(self.snapshot.audio_info.clone());
        }
        if self.snapshot.normalize_audio {
            badges.push("Norm".to_string());
        }
        if self.snapshot.radio_active {
            badges.push("Radio".to_string());
        }
        if self.snapshot.upcoming_count > 0 {
            badges.push(format!("+{} up next", self.snapshot.upcoming_count));
        }
        badges.join(" · ")
    }

    pub fn render_with_hitboxes(
        self,
        area: Rect,
        buf: &mut Buffer,
        selected_control: Option<PlayerBarControl>,
    ) -> PlayerBarHitboxes {
        let mut hit = PlayerBarHitboxes {
            bar_area: area,
            ..PlayerBarHitboxes::default()
        };
        if area.height < 3 || area.width < 12 {
            return hit;
        }
        let theme = active_theme();
        let subtitle = self.subtitle_text();
        let block = panel_block(
            "Now Playing",
            if subtitle.is_empty() {
                None
            } else {
                Some(subtitle.as_str())
            },
            self.focused,
        );
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 6 || inner.height == 0 {
            return hit;
        }
        let content_x = inner.x;
        let content_w = inner.width as usize;

        // Row 1: Track Title · Artist · Album (or error / idle prompt)
        let row1_line = if let Some(ref err) = self.snapshot.error {
            Line::from(Span::styled(
                trunc_cells(&format!("Error: {err}"), content_w),
                Style::default().fg(ERR).bg(Color::Reset),
            ))
        } else if let Some(ref tr) = self.snapshot.track {
            let mut full = Vec::new();
            let title_s = trunc_cells(&tr.title, content_w);
            let mut rem = content_w.saturating_sub(cell_width(&title_s));
            full.push(Span::styled(
                title_s,
                Style::default()
                    .fg(TEXT)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            ));
            let artist_part = format!("  ·  {}", tr.artist_names());
            if rem > 6 {
                let a = trunc_cells(&artist_part, rem);
                rem = rem.saturating_sub(cell_width(&a));
                full.push(Span::styled(a, Style::default().fg(DIM).bg(Color::Reset)));
            }
            if let Some(ref album) = tr.album
                && rem > 8
            {
                let al_part = trunc_cells(&format!("  ·  {album}"), rem);
                full.push(Span::styled(
                    al_part,
                    Style::default().fg(DIM).bg(Color::Reset),
                ));
            }
            Line::from(full)
        } else {
            Line::from(Span::styled(
                trunc_cells("Nothing playing  ·  Press / to search", content_w),
                Style::default().fg(DIM).bg(Color::Reset),
            ))
        };
        row1_line.render(
            Rect {
                x: content_x,
                y: inner.y,
                width: content_w as u16,
                height: 1,
            },
            buf,
        );

        let focused_ctrl = if self.focused { selected_control } else { None };

        // Row 2: Transport controls + Volume bar (gold overdrive > 100%)
        if inner.height >= 2 {
            let row2_y = inner.y + 1;
            let play_glyph = if self.snapshot.playing && !self.snapshot.paused {
                self.icons.pause
            } else {
                self.icons.playing
            };
            let rpt_glyph = match self.snapshot.repeat {
                RepeatMode::Off => self.icons.repeat_off,
                RepeatMode::All => self.icons.repeat_all,
                RepeatMode::One => self.icons.repeat_one,
            };

            let idle_btn = Style::default().fg(TEXT).bg(theme.btn_bg);
            let active_btn = Style::default()
                .fg(theme.btn_primary_fg)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD);
            let btn_style = |ctrl: PlayerBarControl, active: bool| -> Style {
                if focused_ctrl == Some(ctrl) {
                    Style::default()
                        .fg(theme.btn_primary_fg)
                        .bg(TEXT)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else if active {
                    active_btn
                } else {
                    idle_btn
                }
            };

            let prev_lbl = format!(" {} ", self.icons.prev);
            let play_lbl = format!(" {play_glyph} ");
            let next_lbl = format!(" {} ", self.icons.next);
            let shf_lbl = format!(" {} ", self.icons.shuffle);
            let rpt_lbl = format!(" {rpt_glyph} ");

            let prev_w = cell_width(&prev_lbl);
            let play_w = cell_width(&play_lbl);
            let next_w = cell_width(&next_lbl);
            let shf_w = cell_width(&shf_lbl);
            let rpt_w = cell_width(&rpt_lbl);

            let mut cx = content_x;
            hit.prev = Rect::new(cx, row2_y, prev_w as u16, 1);
            cx = cx.saturating_add(prev_w as u16 + 1);
            hit.play_pause = Rect::new(cx, row2_y, play_w as u16, 1);
            cx = cx.saturating_add(play_w as u16 + 1);
            hit.next = Rect::new(cx, row2_y, next_w as u16, 1);
            cx = cx.saturating_add(next_w as u16 + 1);
            hit.shuffle = Rect::new(cx, row2_y, shf_w as u16, 1);
            cx = cx.saturating_add(shf_w as u16 + 1);
            hit.repeat = Rect::new(cx, row2_y, rpt_w as u16, 1);

            let mut left_w = prev_w + 1 + play_w + 1 + next_w + 1 + shf_w + 1 + rpt_w;

            let mut spans = vec![
                Span::styled(prev_lbl, btn_style(PlayerBarControl::Prev, false)),
                Span::raw(" "),
                Span::styled(play_lbl, btn_style(PlayerBarControl::PlayPause, true)),
                Span::raw(" "),
                Span::styled(next_lbl, btn_style(PlayerBarControl::Next, false)),
                Span::raw(" "),
                Span::styled(
                    shf_lbl,
                    btn_style(PlayerBarControl::Shuffle, self.snapshot.shuffle),
                ),
                Span::raw(" "),
                Span::styled(
                    rpt_lbl,
                    btn_style(
                        PlayerBarControl::Repeat,
                        self.snapshot.repeat != RepeatMode::Off,
                    ),
                ),
            ];

            let vol = self.snapshot.volume.clamp(0, 130);
            let muted = self.snapshot.muted;
            let vol_focused = focused_ctrl == Some(PlayerBarControl::Volume);
            let filled = if muted {
                0
            } else {
                ((f64::from(vol) / 10.0).round() as usize).min(10)
            };
            let vol_bar = format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled));
            let vol_color = if muted {
                BORDER
            } else if vol > 100 {
                WARN_GOLD
            } else {
                theme.accent
            };
            let vol_pct_color = if vol_focused {
                theme.accent
            } else if muted {
                DIM
            } else if vol > 100 {
                WARN_GOLD
            } else {
                DIM
            };
            let vol_prefix = format!("{} ", self.icons.volume);
            let vol_pct = if muted {
                " MUTE".to_string()
            } else {
                format!("{vol:>4}%")
            };
            let vol_prefix_w = cell_width(&vol_prefix);
            let vol_total_w = vol_prefix_w + 1 + 10 + 1 + cell_width(&vol_pct);

            if content_w > left_w + vol_total_w {
                let pad = content_w - left_w - vol_total_w;
                let vol_start_x = content_x.saturating_add((left_w + pad) as u16);
                hit.volume_mute = Rect::new(vol_start_x, row2_y, vol_prefix_w as u16, 1);
                hit.volume_bar = Rect::new(
                    vol_start_x.saturating_add(vol_prefix_w as u16 + 1),
                    row2_y,
                    10,
                    1,
                );

                let prefix_style = if vol_focused {
                    Style::default()
                        .fg(theme.accent)
                        .bg(Color::Reset)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    Style::default().fg(DIM).bg(Color::Reset)
                };
                let wall_fg = if vol_focused { theme.accent } else { BORDER };

                spans.push(Span::raw(" ".repeat(pad)));
                spans.push(Span::styled(vol_prefix, prefix_style));
                spans.push(Span::styled(
                    "▕",
                    Style::default().fg(wall_fg).bg(Color::Reset),
                ));
                spans.push(Span::styled(
                    vol_bar,
                    Style::default().fg(vol_color).bg(Color::Reset),
                ));
                spans.push(Span::styled(
                    "▏",
                    Style::default().fg(wall_fg).bg(Color::Reset),
                ));
                spans.push(Span::styled(
                    vol_pct,
                    Style::default().fg(vol_pct_color).bg(Color::Reset),
                ));
            } else {
                let compact_vol = if muted {
                    " MUTE".to_string()
                } else {
                    format!(" {vol}%")
                };
                let cv_w = cell_width(&compact_vol);
                left_w += cv_w;
                if content_w >= left_w {
                    hit.volume_mute = Rect::new(
                        content_x.saturating_add((left_w - cv_w) as u16),
                        row2_y,
                        cv_w as u16,
                        1,
                    );
                    spans.push(Span::styled(
                        compact_vol,
                        Style::default().fg(vol_pct_color).bg(Color::Reset),
                    ));
                }
            }

            Line::from(spans).render(
                Rect {
                    x: content_x,
                    y: row2_y,
                    width: content_w as u16,
                    height: 1,
                },
                buf,
            );
        }

        // Row 3: Seekbar ` 1:47 ━━━━━━━━●──────────  3:20`
        if inner.height >= 3 {
            let row3_y = inner.y + 2;
            let pos = self.position.position_secs.max(0.0);
            let dur = self.position.duration_secs.max(0.0);
            let left_str = if dur > 0.0 || pos > 0.0 {
                format_duration_allow_zero(pos as u32)
            } else {
                "0:00".to_string()
            };
            let right_str = if dur > 0.0 {
                format_duration(Some(dur as u32))
            } else {
                "--:--".to_string()
            };
            let left_pad = format!("{left_str:>5} ");
            let right_pad = format!(" {right_str:>5}");
            let left_pad_w = cell_width(&left_pad);
            let labels_w = left_pad_w + cell_width(&right_pad);
            let bar_w = content_w.saturating_sub(labels_w).max(4);
            let ratio = if dur > 0.0 {
                (pos / dur).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let knob = ((ratio * (bar_w.saturating_sub(1)) as f64).round() as usize)
                .min(bar_w.saturating_sub(1));
            let tail = bar_w.saturating_sub(knob + 1);
            let seek_focused = focused_ctrl == Some(PlayerBarControl::Seekbar);

            hit.seekbar = Rect::new(
                content_x.saturating_add(left_pad_w as u16),
                row3_y,
                bar_w as u16,
                1,
            );

            let time_style = if seek_focused {
                Style::default()
                    .fg(theme.accent)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(DIM).bg(Color::Reset)
            };
            let knob_span = if seek_focused {
                Span::styled(
                    "◆",
                    Style::default()
                        .fg(theme.btn_primary_fg)
                        .bg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("●", Style::default().fg(theme.accent).bg(Color::Reset))
            };

            let seek_line = Line::from(vec![
                Span::styled(left_pad, time_style),
                Span::styled(
                    "━".repeat(knob),
                    Style::default().fg(theme.accent).bg(Color::Reset),
                ),
                knob_span,
                Span::styled(
                    "─".repeat(tail),
                    Style::default().fg(BORDER).bg(Color::Reset),
                ),
                Span::styled(right_pad, Style::default().fg(DIM).bg(Color::Reset)),
            ]);
            seek_line.render(
                Rect {
                    x: content_x,
                    y: row3_y,
                    width: content_w as u16,
                    height: 1,
                },
                buf,
            );
        }

        hit
    }
}

impl Widget for NowPlayingBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let _ = self.render_with_hitboxes(area, buf, None);
    }
}
