use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use musick_core::Lyrics;

use crate::text::{WrappedLyricRow, format_lyric_row, wrap_bilingual_lyric_lines};
use crate::theme::DIM;
use crate::widgets::BrailleSpinner;

pub struct LyricsBox<'a> {
    pub lyrics: &'a Lyrics,
    pub active_idx: Option<usize>,
    pub has_track: bool,
    pub show_translation: bool,
    pub translating: bool,
    pub blank_between: bool,
    pub pad_left: usize,
    pub scroll_offset: isize,
    pub spinner: BrailleSpinner,
}

impl LyricsBox<'_> {
    fn window_start(&self, rows: &[WrappedLyricRow], height: usize) -> isize {
        if self.lyrics.synced {
            let total = rows.len() as isize;
            let target_idx = self.active_idx.unwrap_or(0);
            let mut first_r: Option<isize> = None;
            let mut last_r: isize = 0;
            for (r_i, r) in rows.iter().enumerate() {
                if r.orig_idx == Some(target_idx) && !r.is_spacer {
                    if first_r.is_none() {
                        first_r = Some(r_i as isize);
                    }
                    last_r = r_i as isize;
                }
            }
            let first = first_r.unwrap_or(0);
            let span = (last_r - first + 1).max(1);
            let top_pad = ((height as isize - span) / 2).max(0);
            let min_off = -first;
            let max_off = (total - 1 - first).max(0);
            let clamped = self.scroll_offset.clamp(min_off, max_off);
            first - top_pad + clamped
        } else {
            let max_scroll = rows.len().saturating_sub(height) as isize;
            self.scroll_offset.clamp(0, max_scroll)
        }
    }
}

impl Widget for LyricsBox<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 6 || area.height == 0 {
            return;
        }

        if !self.has_track {
            Line::from(Span::styled(
                "  Play a song for synced lyrics",
                Style::default().fg(DIM).bg(Color::Reset),
            ))
            .render(
                Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
            return;
        }

        if self.translating && self.lyrics.is_empty() {
            self.spinner.loading_line("Translating lyrics…").render(
                Rect {
                    x: area.x + 1,
                    y: area.y,
                    width: area.width.saturating_sub(1),
                    height: 1,
                },
                buf,
            );
            return;
        }

        if self.lyrics.is_empty() {
            Line::from(Span::styled(
                "  No lyrics available",
                Style::default().fg(DIM).bg(Color::Reset),
            ))
            .render(
                Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
            return;
        }

        let w = usize::from(area.width);
        let h = usize::from(area.height);
        let rows = wrap_bilingual_lyric_lines(
            &self.lyrics.lines,
            w,
            self.show_translation,
            self.blank_between,
            self.pad_left,
        );
        let total = rows.len() as isize;
        let start = self.window_start(&rows, h);

        for v_i in 0..h {
            let r_i = start + v_i as isize;
            if r_i < 0 || r_i >= total {
                continue;
            }
            let row = &rows[r_i as usize];
            let is_act = row.orig_idx.is_some() && row.orig_idx == self.active_idx;
            let is_past = match (row.orig_idx, self.active_idx) {
                (Some(o), Some(a)) => o < a,
                _ => false,
            };
            let (txt, style) = format_lyric_row(row, self.pad_left, is_act, is_past);
            if !txt.is_empty() {
                Line::from(Span::styled(txt, style)).render(
                    Rect {
                        x: area.x,
                        y: area.y + v_i as u16,
                        width: area.width,
                        height: 1,
                    },
                    buf,
                );
            }
        }
    }
}
