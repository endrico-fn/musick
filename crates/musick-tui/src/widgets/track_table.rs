use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::text::{Align, align_cells};
use crate::theme::{ACCENT, BORDER_DIM, CURSOR_BG, CURSOR_BLUR_BG, DIM, IconSet, TEXT};
use crate::widgets::{
    BrailleSpinner, DEFAULT_SCROLLOFF, ScrollState, compute_scroll_offset, render_scrollbar,
};

#[derive(Debug, Clone, Copy)]
pub struct ColumnDef<'a> {
    pub title: &'a str,
    pub fixed_width: Option<usize>,
    pub align: Align,
}

#[derive(Debug, Clone)]
pub struct TableRow {
    pub cells: Vec<String>,
    pub is_playing: bool,
    pub is_dim: bool,
    pub prefix_icon: Option<&'static str>,
}

impl TableRow {
    pub fn new(cells: Vec<String>) -> Self {
        Self {
            cells,
            is_playing: false,
            is_dim: false,
            prefix_icon: None,
        }
    }
}

pub struct TrackTable<'a> {
    pub columns: &'a [ColumnDef<'a>],
    pub rows: &'a [TableRow],
    pub cursor: usize,
    pub scroll: Option<&'a ScrollState>,
    pub focused: bool,
    pub show_header: bool,
    pub loading: bool,
    pub loading_label: &'a str,
    pub empty_text: &'a str,
    pub icons: IconSet,
    pub spinner: BrailleSpinner,
}

impl TrackTable<'_> {
    fn compute_widths(&self, avail: usize) -> Vec<usize> {
        let gap_total = self.columns.len().saturating_sub(1);
        let usable = avail.saturating_sub(gap_total);
        let mut widths = vec![0usize; self.columns.len()];
        let mut fixed_sum = 0usize;
        let mut flex_indices = Vec::new();

        for (i, col) in self.columns.iter().enumerate() {
            if let Some(fw) = col.fixed_width {
                widths[i] = fw;
                fixed_sum += fw;
            } else {
                flex_indices.push(i);
            }
        }

        // Prevent overflow on narrow viewports by trimming fixed columns when needed
        let min_flex_total = flex_indices.len().saturating_mul(6);
        let max_fixed_budget = usable.saturating_sub(min_flex_total);
        if fixed_sum > max_fixed_budget && fixed_sum > 0 {
            let mut shrunk_sum = 0usize;
            for (i, col) in self.columns.iter().enumerate() {
                if let Some(fw) = col.fixed_width {
                    let scaled = (fw * max_fixed_budget) / fixed_sum;
                    widths[i] = scaled.max(3.min(fw));
                    shrunk_sum += widths[i];
                }
            }
            // If still over budget on extreme narrow widths, trim largest fixed columns
            while shrunk_sum > max_fixed_budget {
                let Some((idx, _)) = widths
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| self.columns[*i].fixed_width.is_some())
                    .max_by_key(|(_, w)| **w)
                else {
                    break;
                };
                if widths[idx] <= 1 {
                    break;
                }
                widths[idx] -= 1;
                shrunk_sum -= 1;
            }
            fixed_sum = shrunk_sum;
        }

        if !flex_indices.is_empty() {
            let rem = usable.saturating_sub(fixed_sum);
            let each = (rem / flex_indices.len()).max(1);
            let mut leftover = rem.saturating_sub(each * flex_indices.len());
            for &idx in &flex_indices {
                let extra = if leftover > 0 {
                    leftover -= 1;
                    1
                } else {
                    0
                };
                widths[idx] = each + extra;
            }
        }
        widths
    }
}

impl Widget for TrackTable<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height == 0 {
            return;
        }

        if self.loading && self.rows.is_empty() {
            let line = self.spinner.loading_line(self.loading_label);
            line.render(
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

        if self.rows.is_empty() {
            Line::from(Span::styled(
                self.empty_text,
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

        let header_rows = if self.show_header {
            u16::from(area.height >= 1) + u16::from(area.height >= 2)
        } else {
            0
        };
        let visible_rows_est = usize::from(area.height.saturating_sub(header_rows));
        let show_scrollbar =
            visible_rows_est > 0 && self.rows.len() > visible_rows_est && area.width >= 12;
        let content_width = if show_scrollbar {
            area.width.saturating_sub(2)
        } else {
            area.width
        };

        let gutter = 2usize; // "▸ " or "  "
        let avail = usize::from(content_width).saturating_sub(gutter);
        let col_widths = self.compute_widths(avail);

        let mut y = area.y;
        let max_y = area.y + area.height;

        if self.show_header && y < max_y {
            let mut h_spans = vec![Span::raw("  ")];
            for (i, col) in self.columns.iter().enumerate() {
                if i > 0 {
                    h_spans.push(Span::raw(" "));
                }
                let text = align_cells(col.title, col_widths[i], col.align);
                h_spans.push(Span::styled(
                    text,
                    Style::default()
                        .fg(DIM)
                        .bg(Color::Reset)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            Line::from(h_spans).render(
                Rect {
                    x: area.x,
                    y,
                    width: content_width,
                    height: 1,
                },
                buf,
            );
            y += 1;

            if y < max_y {
                Line::from(Span::styled(
                    "─".repeat(usize::from(area.width)),
                    Style::default().fg(BORDER_DIM).bg(Color::Reset),
                ))
                .render(
                    Rect {
                        x: area.x,
                        y,
                        width: area.width,
                        height: 1,
                    },
                    buf,
                );
                y += 1;
            }
        }

        let visible_rows = usize::from(max_y.saturating_sub(y));
        if visible_rows == 0 {
            return;
        }

        let clamped_cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        let scroll_start = match self.scroll {
            Some(s) => s.ensure_visible(
                clamped_cursor,
                visible_rows,
                self.rows.len(),
                DEFAULT_SCROLLOFF,
            ),
            None => compute_scroll_offset(
                0,
                clamped_cursor,
                visible_rows,
                self.rows.len(),
                DEFAULT_SCROLLOFF,
            ),
        };

        for (offset, row_idx) in (scroll_start..self.rows.len())
            .take(visible_rows)
            .enumerate()
        {
            let row = &self.rows[row_idx];
            let is_selected = row_idx == clamped_cursor;
            let bg = if is_selected {
                if self.focused {
                    CURSOR_BG
                } else {
                    CURSOR_BLUR_BG
                }
            } else {
                Color::Reset
            };

            let lead = if is_selected {
                format!("{} ", self.icons.cursor)
            } else if row.is_playing {
                format!("{} ", self.icons.playing)
            } else if let Some(icon) = row.prefix_icon {
                format!("{icon} ")
            } else {
                "  ".to_string()
            };
            let lead_fg = if is_selected || row.is_playing {
                ACCENT
            } else {
                DIM
            };

            let mut spans = vec![Span::styled(lead, Style::default().fg(lead_fg).bg(bg))];

            for (i, col) in self.columns.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled(" ", Style::default().bg(bg)));
                }
                let raw = row.cells.get(i).map(String::as_str).unwrap_or("");
                let formatted = align_cells(raw, col_widths[i], col.align);
                let fg = if row.is_playing && i == 0 {
                    ACCENT
                } else if row.is_dim || i > 0 {
                    if is_selected && i == 0 { TEXT } else { DIM }
                } else {
                    TEXT
                };
                let mut style = Style::default().fg(fg).bg(bg);
                if (row.is_playing || is_selected) && i == 0 {
                    style = style.add_modifier(Modifier::BOLD);
                }
                spans.push(Span::styled(formatted, style));
            }

            let row_rect = Rect {
                x: area.x,
                y: y + offset as u16,
                width: content_width,
                height: 1,
            };
            if bg != Color::Reset {
                for cx in row_rect.x..row_rect.x + row_rect.width {
                    if let Some(cell) = buf.cell_mut((cx, row_rect.y)) {
                        cell.set_bg(bg);
                    }
                }
            }
            Line::from(spans).render(row_rect, buf);
        }

        if show_scrollbar {
            render_scrollbar(
                buf,
                area.x + area.width - 1,
                y,
                visible_rows,
                self.rows.len(),
                scroll_start,
                self.focused,
            );
        }
    }
}
