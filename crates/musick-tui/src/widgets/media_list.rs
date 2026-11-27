use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::text::{cell_width, split_row_cells};
use crate::theme::{ACCENT, CURSOR_BG, CURSOR_BLUR_BG, DIM, IconSet, TEXT};
use crate::widgets::{
    BrailleSpinner, CoverRenderer, ScrollState, compute_scroll_offset, render_scrollbar,
};

#[derive(Debug, Clone)]
pub struct MediaListItem {
    pub title: String,
    pub subtitle: String,
    pub meta_top: String,
    pub meta_bottom: String,
    pub cover_url: Option<String>,
    pub glyph: &'static str,
    pub is_playing: bool,
    pub is_dim: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn render_media_list(
    frame: &mut Frame<'_>,
    area: Rect,
    items: &[MediaListItem],
    cursor: usize,
    scroll: Option<&ScrollState>,
    focused: bool,
    loading: bool,
    loading_label: &str,
    empty_text: &str,
    icons: IconSet,
    spinner: BrailleSpinner,
    covers: &mut CoverRenderer,
) {
    if area.width < 8 || area.height == 0 {
        return;
    }

    if loading && items.is_empty() {
        frame.render_widget(
            Paragraph::new(spinner.loading_line(loading_label)),
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
        );
        return;
    }

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                empty_text,
                Style::default().fg(DIM).bg(Color::Reset),
            ))),
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
        );
        return;
    }

    let row_h: u16 = if area.height >= 2 { 2 } else { 1 };
    let visible_items = usize::from(area.height / row_h).max(1);
    let show_scrollbar = items.len() > visible_items && area.width >= 12;
    let content_w = if show_scrollbar {
        area.width.saturating_sub(2)
    } else {
        area.width
    };

    let show_covers = covers.is_enabled() && row_h == 2 && content_w >= 26;
    let mini_w = if show_covers {
        covers.mini_cover_width(2).min(content_w.saturating_sub(18))
    } else {
        0
    };
    let text_x = area.x + if show_covers { mini_w + 1 } else { 0 };
    let text_w = content_w.saturating_sub(if show_covers { mini_w + 1 } else { 0 });
    if text_w < 6 {
        return;
    }

    let clamped_cursor = cursor.min(items.len().saturating_sub(1));
    let scroll_start = match scroll {
        Some(s) => s.ensure_visible(clamped_cursor, visible_items, items.len(), 1),
        None => compute_scroll_offset(0, clamped_cursor, visible_items, items.len(), 1),
    };

    for (slot, item_idx) in (scroll_start..items.len()).take(visible_items).enumerate() {
        let item = &items[item_idx];
        let y = area.y + (slot as u16) * row_h;
        let is_selected = item_idx == clamped_cursor;
        let bg = if is_selected {
            if focused { CURSOR_BG } else { CURSOR_BLUR_BG }
        } else {
            Color::Reset
        };

        if show_covers {
            covers.render_mini_cover(
                frame,
                Rect {
                    x: area.x,
                    y,
                    width: mini_w,
                    height: 2,
                },
                item.cover_url.as_deref(),
                item.glyph,
                is_selected || item.is_playing,
            );
        }

        let lead = if is_selected {
            format!("{} ", icons.cursor)
        } else if item.is_playing {
            format!("{} ", icons.playing)
        } else if !show_covers {
            format!("{} ", item.glyph)
        } else {
            "  ".to_string()
        };
        let lead_w = cell_width(&lead);
        let lead_fg = if is_selected || item.is_playing {
            ACCENT
        } else {
            DIM
        };

        let (title_s, top_meta, top_gap) = split_row_cells(
            &item.title,
            &item.meta_top,
            usize::from(text_w),
            lead_w,
            0.35,
        );
        let title_fg = if item.is_playing {
            ACCENT
        } else if item.is_dim {
            if is_selected { TEXT } else { DIM }
        } else {
            TEXT
        };
        let mut title_style = Style::default().fg(title_fg).bg(bg);
        if item.is_playing || is_selected {
            title_style = title_style.add_modifier(Modifier::BOLD);
        }
        let top_meta_fg = if item.is_playing { ACCENT } else { DIM };
        let mut line1_spans = vec![
            Span::styled(lead, Style::default().fg(lead_fg).bg(bg)),
            Span::styled(title_s, title_style),
        ];
        if !top_meta.is_empty() {
            line1_spans.push(Span::styled(" ".repeat(top_gap), Style::default().bg(bg)));
            line1_spans.push(Span::styled(
                top_meta,
                Style::default().fg(top_meta_fg).bg(bg),
            ));
        }

        let row1_rect = Rect {
            x: text_x,
            y,
            width: text_w,
            height: 1,
        };
        let buf = frame.buffer_mut();
        if bg != Color::Reset {
            for cx in row1_rect.x..row1_rect.x + row1_rect.width {
                if let Some(cell) = buf.cell_mut((cx, row1_rect.y)) {
                    cell.set_bg(bg);
                }
            }
        }
        frame.render_widget(Paragraph::new(Line::from(line1_spans)), row1_rect);

        if row_h >= 2 {
            let sub_gutter = " ".repeat(lead_w);
            let (sub_s, bot_meta, bot_gap) = split_row_cells(
                &item.subtitle,
                &item.meta_bottom,
                usize::from(text_w),
                lead_w,
                0.35,
            );
            let mut line2_spans = vec![
                Span::styled(sub_gutter, Style::default().bg(bg)),
                Span::styled(sub_s, Style::default().fg(DIM).bg(bg)),
            ];
            if !bot_meta.is_empty() {
                line2_spans.push(Span::styled(" ".repeat(bot_gap), Style::default().bg(bg)));
                line2_spans.push(Span::styled(bot_meta, Style::default().fg(DIM).bg(bg)));
            }
            let row2_rect = Rect {
                x: text_x,
                y: y + 1,
                width: text_w,
                height: 1,
            };
            let buf = frame.buffer_mut();
            if bg != Color::Reset {
                for cx in row2_rect.x..row2_rect.x + row2_rect.width {
                    if let Some(cell) = buf.cell_mut((cx, row2_rect.y)) {
                        cell.set_bg(bg);
                    }
                }
            }
            frame.render_widget(Paragraph::new(Line::from(line2_spans)), row2_rect);
        }
    }

    if show_scrollbar {
        let sb_rows = usize::from(area.height).min(visible_items * usize::from(row_h));
        render_scrollbar(
            frame.buffer_mut(),
            area.x + area.width - 1,
            area.y,
            sb_rows,
            items.len() * usize::from(row_h),
            scroll_start * usize::from(row_h),
            focused,
        );
    }
}
