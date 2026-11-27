use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use musick_audio::resample_bars;
use musick_core::ThemeAccent;

use crate::theme::{DIM, theme_for};

const BLOCKS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

fn smoothstep(t: f32) -> f32 {
    let x = t.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn lerp_rgb(c0: (u8, u8, u8), c1: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let s = smoothstep(t);
    (
        (f32::from(c0.0) + (f32::from(c1.0) - f32::from(c0.0)) * s).round() as u8,
        (f32::from(c0.1) + (f32::from(c1.1) - f32::from(c0.1)) * s).round() as u8,
        (f32::from(c0.2) + (f32::from(c1.2) - f32::from(c0.2)) * s).round() as u8,
    )
}

fn gradient_color(accent: ThemeAccent, ratio: f32, tip_boost: f32) -> Color {
    let tokens = theme_for(accent);
    let stops = tokens.cava_stops;
    let r = ratio.clamp(0.0, 1.0);
    let mut rgb = stops[stops.len() - 1].1;
    for win in stops.windows(2) {
        let (p0, c0) = win[0];
        let (p1, c1) = win[1];
        if r <= p1 {
            let local = (r - p0) / (p1 - p0).max(1e-6);
            rgb = lerp_rgb(c0, c1, local);
            break;
        }
    }
    if tip_boost > 0.0 {
        rgb = lerp_rgb(rgb, tokens.cava_tip_rgb, tip_boost);
    }
    Color::Rgb(rgb.0, rgb.1, rgb.2)
}

pub struct CavaWidget<'a> {
    pub bars: &'a [f32],
    pub accent: ThemeAccent,
    pub idle: bool,
}

impl Widget for CavaWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 2 {
            return;
        }
        let w = usize::from(area.width);
        let h = usize::from(area.height);
        let bar_w = if w >= 96 { 2 } else { 1 };
        let gap = 1usize;
        let step = bar_w + gap;
        let count = ((w + gap) / step).max(1);
        let vals = resample_bars(self.bars, count);
        let total_used = count * bar_w + count.saturating_sub(1) * gap;
        let pad_left = w.saturating_sub(total_used) / 2;

        let max_units = (h * 8) as f32;
        let base_floor = gradient_color(self.accent, 0.0, 0.0);
        let bar_units: Vec<i32> = vals
            .iter()
            .map(|&v| (v.clamp(0.0, 1.0) * max_units).round() as i32)
            .collect();

        for row_from_top in 0..h {
            let row_from_bottom = h - 1 - row_from_top;
            let cell_mid = (row_from_bottom as f32 + 0.5) / h as f32;

            let mut spans = Vec::with_capacity(count * 2 + 1);
            if pad_left > 0 {
                spans.push(Span::raw(" ".repeat(pad_left)));
            }

            for (idx, (&amp, &units)) in vals.iter().zip(bar_units.iter()).enumerate() {
                if idx > 0 {
                    spans.push(Span::raw(" "));
                }
                let cell_units = units - (row_from_bottom as i32) * 8;
                let (glyph, color) = if row_from_bottom == 0 && cell_units <= 0 {
                    (BLOCKS[1], if self.idle { DIM } else { base_floor })
                } else if cell_units <= 0 {
                    (" ", Color::Reset)
                } else if cell_units >= 8 {
                    let is_top = ((row_from_bottom as i32 + 1) * 8 >= units) && units >= 6;
                    let t = (0.78 * cell_mid + 0.22 * amp).clamp(0.0, 1.0);
                    (
                        BLOCKS[8],
                        gradient_color(self.accent, t, if is_top { 0.16 * t } else { 0.0 }),
                    )
                } else {
                    let pos_t = (row_from_bottom as f32 + cell_units as f32 / 8.0) / h as f32;
                    let t = (0.72 * pos_t + 0.28 * amp).clamp(0.0, 1.0);
                    (
                        BLOCKS[cell_units as usize],
                        gradient_color(self.accent, t, if units >= 4 { 0.16 * t } else { 0.0 }),
                    )
                };
                let bar_str = glyph.repeat(bar_w);
                spans.push(Span::styled(
                    bar_str,
                    Style::default().fg(color).bg(Color::Reset),
                ));
            }

            Line::from(spans).render(
                Rect {
                    x: area.x,
                    y: area.y + row_from_top as u16,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
        }
    }
}
