use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::text::cell_width;
use crate::theme::{ACCENT, BORDER_DIM, DIM};

pub struct SectionTabs<'a> {
    pub tabs: &'a [(&'a str, &'a str)], // (full_label, short_label)
    pub active: usize,
    pub right_hint: Option<&'a str>,
}

impl Widget for SectionTabs<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.tabs.is_empty() {
            return;
        }
        let w = usize::from(area.width);
        let full_total: usize = self.tabs.iter().map(|(f, _)| cell_width(f)).sum::<usize>()
            + self.tabs.len().saturating_sub(1) * 3;
        let use_short = full_total + 4 > w;

        let mut spans = Vec::new();
        let mut used = 0usize;
        for (idx, &(full, short)) in self.tabs.iter().enumerate() {
            if idx > 0 {
                let sep = if use_short { " │ " } else { " · " };
                if used + cell_width(sep) >= w {
                    break;
                }
                spans.push(Span::styled(
                    sep,
                    Style::default().fg(BORDER_DIM).bg(Color::Reset),
                ));
                used += cell_width(sep);
            }
            let lbl = if use_short { short } else { full };
            let lw = cell_width(lbl);
            if used + lw > w {
                break;
            }
            let style = if idx == self.active {
                Style::default()
                    .fg(ACCENT)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default().fg(DIM).bg(Color::Reset)
            };
            spans.push(Span::styled(lbl, style));
            used += lw;
        }

        if let Some(hint) = self.right_hint {
            let hw = cell_width(hint);
            if w > used + hw + 1 {
                let pad = w - used - hw;
                spans.push(Span::raw(" ".repeat(pad)));
                spans.push(Span::styled(
                    hint,
                    Style::default().fg(DIM).bg(Color::Reset),
                ));
            }
        }

        Line::from(spans).render(
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
            buf,
        );
    }
}
