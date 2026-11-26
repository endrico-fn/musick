use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use musick_core::{PlaybackSnapshot, PositionTick, RepeatMode, format_duration, format_duration_allow_zero};

use crate::text::{cell_width, trunc_cells};
use crate::theme::{
    ACCENT, BORDER, BTN_BG, BTN_PRIMARY_FG, DIM, ERR, IconSet, TEXT, WARN_GOLD, panel_block,
};
use crate::widgets::BrailleSpinner;

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
        if self.snapshot.autoplay_active {
            badges.push("Radio".to_string());
        }
        if self.snapshot.upcoming_count > 0 {
            badges.push(format!("+{} up next", self.snapshot.upcoming_count));
        }
        badges.join(" · ")
    }
}

impl Widget for NowPlayingBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 3 || area.width < 12 {
            return;
        }
        let subtitle = self.subtitle_text();
        let block = panel_block(
            "Player",
            if subtitle.is_empty() {
                None
            } else {
                Some(subtitle.as_str())
            },
            self.focused,
        );
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 8 || inner.height == 0 {
            return;
        }
        let content_x = inner.x + 1;
        let content_w = inner.width.saturating_sub(2) as usize;
        if content_w < 6 {
            return;
        }

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

        // Row 2: Transport controls + Volume bar (gold overdrive > 100%)
        if inner.height >= 2 {
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

            let idle_btn = Style::default().fg(TEXT).bg(BTN_BG);
            let active_btn = Style::default()
                .fg(BTN_PRIMARY_FG)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD);

            let prev_lbl = format!(" {} ", self.icons.prev);
            let play_lbl = format!(" {play_glyph} ");
            let next_lbl = format!(" {} ", self.icons.next);
            let shf_lbl = format!(" {} ", self.icons.shuffle);
            let rpt_lbl = format!(" {rpt_glyph} ");

            let mut left_w = cell_width(&prev_lbl)
                + 1
                + cell_width(&play_lbl)
                + 1
                + cell_width(&next_lbl)
                + 1
                + cell_width(&shf_lbl)
                + 1
                + cell_width(&rpt_lbl);

            let mut spans = vec![
                Span::styled(prev_lbl, idle_btn),
                Span::raw(" "),
                Span::styled(play_lbl, active_btn),
                Span::raw(" "),
                Span::styled(next_lbl, idle_btn),
                Span::raw(" "),
                Span::styled(
                    shf_lbl,
                    if self.snapshot.shuffle {
                        active_btn
                    } else {
                        idle_btn
                    },
                ),
                Span::raw(" "),
                Span::styled(
                    rpt_lbl,
                    if self.snapshot.repeat != RepeatMode::Off {
                        active_btn
                    } else {
                        idle_btn
                    },
                ),
            ];

            let vol = self.snapshot.volume.clamp(0, 130);
            let filled = ((f64::from(vol) / 10.0).round() as usize).min(10);
            let vol_bar = format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled));
            let vol_color = if vol > 100 { WARN_GOLD } else { ACCENT };
            let vol_pct_color = if vol > 100 { WARN_GOLD } else { DIM };
            let vol_prefix = format!("{} ", self.icons.volume);
            let vol_pct = format!("{vol:>4}%");
            let vol_total_w = cell_width(&vol_prefix) + 1 + 10 + 1 + cell_width(&vol_pct);

            if content_w > left_w + vol_total_w {
                let pad = content_w - left_w - vol_total_w;
                spans.push(Span::raw(" ".repeat(pad)));
                spans.push(Span::styled(
                    vol_prefix,
                    Style::default().fg(DIM).bg(Color::Reset),
                ));
                spans.push(Span::styled(
                    "▕",
                    Style::default().fg(BORDER).bg(Color::Reset),
                ));
                spans.push(Span::styled(
                    vol_bar,
                    Style::default().fg(vol_color).bg(Color::Reset),
                ));
                spans.push(Span::styled(
                    "▏",
                    Style::default().fg(BORDER).bg(Color::Reset),
                ));
                spans.push(Span::styled(
                    vol_pct,
                    Style::default().fg(vol_pct_color).bg(Color::Reset),
                ));
            } else {
                let compact_vol = format!(" {vol}%");
                left_w += cell_width(&compact_vol);
                if content_w >= left_w {
                    spans.push(Span::styled(
                        compact_vol,
                        Style::default().fg(vol_pct_color).bg(Color::Reset),
                    ));
                }
            }

            Line::from(spans).render(
                Rect {
                    x: content_x,
                    y: inner.y + 1,
                    width: content_w as u16,
                    height: 1,
                },
                buf,
            );
        }

        // Row 3: Seekbar ` 1:47 ━━━━━━━━●──────────  3:20`
        if inner.height >= 3 {
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
            let labels_w = cell_width(&left_pad) + cell_width(&right_pad);
            let bar_w = content_w.saturating_sub(labels_w).max(4);
            let ratio = if dur > 0.0 {
                (pos / dur).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let knob = ((ratio * (bar_w.saturating_sub(1)) as f64).round() as usize)
                .min(bar_w.saturating_sub(1));
            let tail = bar_w.saturating_sub(knob + 1);

            let seek_line = Line::from(vec![
                Span::styled(left_pad, Style::default().fg(DIM).bg(Color::Reset)),
                Span::styled("━".repeat(knob), Style::default().fg(ACCENT).bg(Color::Reset)),
                Span::styled("●", Style::default().fg(ACCENT).bg(Color::Reset)),
                Span::styled("─".repeat(tail), Style::default().fg(BORDER).bg(Color::Reset)),
                Span::styled(right_pad, Style::default().fg(DIM).bg(Color::Reset)),
            ]);
            seek_line.render(
                Rect {
                    x: content_x,
                    y: inner.y + 2,
                    width: content_w as u16,
                    height: 1,
                },
                buf,
            );
        }
    }
}
