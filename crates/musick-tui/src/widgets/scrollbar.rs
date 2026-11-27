use ratatui::buffer::Buffer;
use ratatui::style::Color;

use crate::theme::{ACCENT, DIM, SCROLL_TRACK};

pub const SCROLL_THUMB_GLYPH: &str = "▎";
pub const SCROLL_TRACK_GLYPH: &str = "▏";

pub fn render_scrollbar(
    buf: &mut Buffer,
    x: u16,
    y_start: u16,
    visible_rows: usize,
    total_rows: usize,
    scroll_start: usize,
    active: bool,
) {
    if visible_rows == 0 || total_rows <= visible_rows {
        return;
    }
    let thumb_h = ((visible_rows * visible_rows) / total_rows).clamp(1, visible_rows);
    let max_scroll = total_rows.saturating_sub(visible_rows).max(1);
    let thumb_top = (scroll_start * visible_rows.saturating_sub(thumb_h)) / max_scroll;
    let thumb_fg = if active { ACCENT } else { DIM };
    for r in 0..visible_rows {
        if let Some(cell) = buf.cell_mut((x, y_start + r as u16)) {
            let is_thumb = r >= thumb_top && r < thumb_top + thumb_h;
            cell.set_symbol(if is_thumb {
                SCROLL_THUMB_GLYPH
            } else {
                SCROLL_TRACK_GLYPH
            });
            cell.set_fg(if is_thumb { thumb_fg } else { SCROLL_TRACK });
            cell.set_bg(Color::Reset);
        }
    }
}
