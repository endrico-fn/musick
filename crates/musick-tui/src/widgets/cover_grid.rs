use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use musick_core::CoverSize;

use crate::text::trunc_cells;
use crate::theme::{ACCENT, BORDER_DIM, CURSOR_BG, DIM, TEXT};
use crate::widgets::{BrailleSpinner, CoverRenderer};

#[derive(Debug, Clone)]
pub struct GridCardItem {
    pub title: String,
    pub subtitle: String,
    pub cover_url: Option<String>,
    pub glyph: &'static str,
}

pub fn grid_card_dims(size: CoverSize) -> (u16, u16) {
    // (card_width, card_height) including border + 2 text lines
    match size {
        CoverSize::S => (16, 8),
        CoverSize::M => (20, 10),
        CoverSize::L => (24, 12),
    }
}

pub fn grid_columns_for(width: u16, size: CoverSize) -> usize {
    let (cw, _) = grid_card_dims(size);
    let step = cw + 1;
    usize::from((width + 1) / step).max(1)
}

#[allow(clippy::too_many_arguments)]
pub fn render_cover_grid(
    frame: &mut Frame<'_>,
    area: Rect,
    items: &[GridCardItem],
    cursor: usize,
    size: CoverSize,
    focused: bool,
    loading: bool,
    empty_text: &str,
    covers: &mut CoverRenderer,
    spinner: BrailleSpinner,
) {
    if area.width < 12 || area.height < 4 {
        return;
    }

    if loading && items.is_empty() {
        frame.render_widget(
            Paragraph::new(spinner.loading_line("Loading…")),
            Rect {
                x: area.x + 1,
                y: area.y,
                width: area.width.saturating_sub(1),
                height: 1,
            },
        );
        return;
    }

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(empty_text).style(Style::default().fg(DIM).bg(Color::Reset))),
            Rect {
                x: area.x + 1,
                y: area.y,
                width: area.width.saturating_sub(1),
                height: 1,
            },
        );
        return;
    }

    let (card_w, card_h) = grid_card_dims(size);
    let cols = grid_columns_for(area.width, size);
    let visible_rows = usize::from(area.height / card_h).max(1);

    let clamped = cursor.min(items.len().saturating_sub(1));
    let cur_row = clamped / cols;
    let first_row = cur_row.saturating_sub(visible_rows.saturating_sub(1));

    for vis_r in 0..visible_rows {
        let row_idx = first_row + vis_r;
        let y = area.y + (vis_r as u16) * card_h;
        if y + card_h > area.y + area.height {
            break;
        }
        for c in 0..cols {
            let item_idx = row_idx * cols + c;
            let Some(item) = items.get(item_idx) else {
                break;
            };
            let x = area.x + (c as u16) * (card_w + 1);
            if x + card_w > area.x + area.width {
                break;
            }

            let is_sel = item_idx == clamped;
            let card_rect = Rect {
                x,
                y,
                width: card_w,
                height: card_h,
            };

            let border_color = if is_sel && focused {
                ACCENT
            } else {
                BORDER_DIM
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .border_style(Style::default().fg(border_color).bg(Color::Reset))
                .style(Style::default().bg(Color::Reset));
            let inner = block.inner(card_rect);
            frame.render_widget(block, card_rect);

            if inner.height >= 3 && inner.width >= 4 {
                let art_h = inner.height.saturating_sub(2);
                let art_rect = Rect {
                    x: inner.x,
                    y: inner.y,
                    width: inner.width,
                    height: art_h,
                };
                covers.render_cover(
                    frame,
                    art_rect,
                    item.cover_url.as_deref(),
                    false,
                    item.glyph,
                    spinner,
                );

                let bg = if is_sel && focused {
                    CURSOR_BG
                } else {
                    Color::Reset
                };
                let title_line = Line::from(trunc_cells(&item.title, usize::from(inner.width)))
                    .style(
                        Style::default()
                            .fg(if is_sel { ACCENT } else { TEXT })
                            .bg(bg)
                            .add_modifier(if is_sel {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    );
                frame.render_widget(
                    Paragraph::new(title_line),
                    Rect {
                        x: inner.x,
                        y: inner.y + art_h,
                        width: inner.width,
                        height: 1,
                    },
                );

                let sub_line = Line::from(trunc_cells(&item.subtitle, usize::from(inner.width)))
                    .style(Style::default().fg(DIM).bg(Color::Reset));
                frame.render_widget(
                    Paragraph::new(sub_line),
                    Rect {
                        x: inner.x,
                        y: inner.y + art_h + 1,
                        width: inner.width,
                        height: 1,
                    },
                );
            }
        }
    }
}
