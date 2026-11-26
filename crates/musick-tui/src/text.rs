use ratatui::style::{Color, Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use musick_core::LyricLine;

use crate::theme::{ACCENT, DIM, TEXT, TR_ACTIVE, TR_PAST, TR_UPCOMING};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

pub fn cell_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Truncate text to at most `width` terminal display columns, appending `…` when clipped.
pub fn trunc_cells(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if cell_width(text) <= width {
        return text.to_string();
    }
    if width == 1 {
        return text
            .chars()
            .next()
            .filter(|c| c.width().unwrap_or(0) <= 1)
            .map(|c| c.to_string())
            .unwrap_or_else(|| "…".to_string());
    }
    let limit = width - 1;
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let cw = ch.width().unwrap_or(0);
        if used + cw > limit {
            break;
        }
        out.push(ch);
        used += cw;
    }
    out.push('…');
    out
}

/// Truncate and pad text to exactly `width` terminal display columns.
pub fn align_cells(text: &str, width: usize, align: Align) -> String {
    let s = trunc_cells(text, width);
    let pad = width.saturating_sub(cell_width(&s));
    match align {
        Align::Left => format!("{s}{}", " ".repeat(pad)),
        Align::Right => format!("{}{s}", " ".repeat(pad)),
    }
}

/// Fit left and right labels into `total_w` columns (excluding `gutter`).
pub fn split_row_cells(
    left: &str,
    right: &str,
    total_w: usize,
    gutter: usize,
    right_cap_ratio: f64,
) -> (String, String, usize) {
    let avail = total_w.saturating_sub(gutter).max(4);
    if right.is_empty() {
        let l = trunc_cells(left, avail);
        let gap = avail.saturating_sub(cell_width(&l));
        return (l, String::new(), gap);
    }
    let r_max = ((avail as f64) * right_cap_ratio).floor().max(4.0) as usize;
    let r_str = trunc_cells(right, r_max);
    let l_max = avail.saturating_sub(cell_width(&r_str) + 2).max(3);
    let l_str = trunc_cells(left, l_max);
    let gap = avail
        .saturating_sub(cell_width(&l_str) + cell_width(&r_str))
        .max(1);
    (l_str, r_str, gap)
}

fn hard_break_word(word: &str, max_w: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut cur = String::new();
    let mut used = 0usize;
    for ch in word.chars() {
        let cw = ch.width().unwrap_or(0);
        if used + cw > max_w && !cur.is_empty() {
            chunks.push(std::mem::take(&mut cur));
            used = 0;
        }
        cur.push(ch);
        used += cw;
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    chunks
}

/// Word-wrap text so every returned line fits within `width` display columns (CJK/emoji safe).
pub fn wrap_cells(text: &str, width: usize) -> Vec<String> {
    let max_w = width.max(4);
    if cell_width(text) <= max_w {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;

    for word in text.split(' ') {
        let ww = cell_width(word);
        if cur.is_empty() {
            if ww <= max_w {
                cur.push_str(word);
                cur_w = ww;
            } else {
                let mut broken = hard_break_word(word, max_w);
                if let Some(last) = broken.pop() {
                    lines.extend(broken);
                    cur_w = cell_width(&last);
                    cur = last;
                }
            }
        } else if cur_w + 1 + ww <= max_w {
            cur.push(' ');
            cur.push_str(word);
            cur_w += 1 + ww;
        } else {
            lines.push(std::mem::take(&mut cur));
            if ww <= max_w {
                cur.push_str(word);
                cur_w = ww;
            } else {
                let mut broken = hard_break_word(word, max_w);
                if let Some(last) = broken.pop() {
                    lines.extend(broken);
                    cur_w = cell_width(&last);
                    cur = last;
                }
            }
        }
    }

    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(trunc_cells(text, max_w));
    }
    lines
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedLyricRow {
    pub orig_idx: Option<usize>,
    pub text: String,
    pub is_translation: bool,
    pub is_spacer: bool,
}

pub fn wrap_bilingual_lyric_lines(
    lines: &[LyricLine],
    width: usize,
    show_translation: bool,
    blank_between: bool,
    pad_left: usize,
) -> Vec<WrappedLyricRow> {
    let wrap_w = width.saturating_sub(pad_left).max(8);
    let mut rows = Vec::new();
    let has_tr = show_translation && lines.iter().any(|l| !l.translation.trim().is_empty());
    let last_i = lines.len().saturating_sub(1);

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.text.trim();
        let raw = if trimmed.is_empty() {
            "···"
        } else {
            trimmed
        };
        for part in wrap_cells(raw, wrap_w) {
            rows.push(WrappedLyricRow {
                orig_idx: Some(idx),
                text: part,
                is_translation: false,
                is_spacer: false,
            });
        }

        if has_tr {
            let tr = line.translation.trim();
            if !tr.is_empty() && !tr.eq_ignore_ascii_case(raw) {
                for part in wrap_cells(tr, wrap_w) {
                    rows.push(WrappedLyricRow {
                        orig_idx: Some(idx),
                        text: part,
                        is_translation: true,
                        is_spacer: false,
                    });
                }
            }
            if blank_between && idx < last_i {
                rows.push(WrappedLyricRow {
                    orig_idx: None,
                    text: String::new(),
                    is_translation: false,
                    is_spacer: true,
                });
            }
        }
    }
    rows
}

pub fn format_lyric_row(
    row: &WrappedLyricRow,
    pad_left: usize,
    is_active: bool,
    is_past: bool,
) -> (String, Style) {
    if row.is_spacer {
        return (String::new(), Style::default().bg(Color::Reset));
    }
    let txt = format!("{}{}", " ".repeat(pad_left), row.text);
    let style = if row.is_translation {
        if is_active {
            Style::default().fg(TR_ACTIVE).bg(Color::Reset)
        } else if is_past {
            Style::default().fg(TR_PAST).bg(Color::Reset)
        } else {
            Style::default().fg(TR_UPCOMING).bg(Color::Reset)
        }
    } else if is_active {
        Style::default()
            .fg(ACCENT)
            .bg(Color::Reset)
            .add_modifier(Modifier::BOLD)
    } else if is_past {
        Style::default().fg(DIM).bg(Color::Reset)
    } else {
        Style::default().fg(TEXT).bg(Color::Reset)
    };
    (txt, style)
}

pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}
