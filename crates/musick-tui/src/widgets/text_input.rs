use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthChar;

use crate::keymap::Action;
use crate::theme::{DIM, TEXT, active_theme};

/// single-line editable utf-8 text buffer with char-index cursor
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    text: String,
    cursor: usize,
}

impl TextInput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        Self { text, cursor }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn trim(&self) -> &str {
        self.text.trim()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.chars().count();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    fn byte_index_at(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.text.len())
    }

    pub fn insert_char(&mut self, ch: char) {
        let idx = self.byte_index_at(self.cursor);
        self.text.insert(idx, ch);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let end = self.byte_index_at(self.cursor);
        let start = self.byte_index_at(self.cursor - 1);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
        true
    }

    pub fn delete(&mut self) -> bool {
        let len = self.text.chars().count();
        if self.cursor >= len {
            return false;
        }
        let start = self.byte_index_at(self.cursor);
        let end = self.byte_index_at(self.cursor + 1);
        self.text.replace_range(start..end, "");
        true
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        let len = self.text.chars().count();
        self.cursor = (self.cursor + 1).min(len);
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.text.chars().count();
    }

    pub fn delete_word_back(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let chars: Vec<char> = self.text.chars().collect();
        let mut new_cur = self.cursor;
        while new_cur > 0 && chars[new_cur - 1].is_whitespace() {
            new_cur -= 1;
        }
        while new_cur > 0 && !chars[new_cur - 1].is_whitespace() {
            new_cur -= 1;
        }
        let start = self.byte_index_at(new_cur);
        let end = self.byte_index_at(self.cursor);
        self.text.replace_range(start..end, "");
        self.cursor = new_cur;
        true
    }

    /// apply an editing action, returning `(handled, text_changed)`
    pub fn handle_action(&mut self, action: &Action) -> (bool, bool) {
        match *action {
            Action::InputChar(ch) => {
                self.insert_char(ch);
                (true, true)
            }
            Action::InputBackspace => (true, self.backspace()),
            Action::InputDelete => (true, self.delete()),
            Action::InputLeft => {
                self.move_left();
                (true, false)
            }
            Action::InputRight => {
                self.move_right();
                (true, false)
            }
            Action::InputHome => {
                self.move_home();
                (true, false)
            }
            Action::InputEnd => {
                self.move_end();
                (true, false)
            }
            Action::InputClearLine => {
                let changed = !self.text.is_empty();
                self.clear();
                (true, changed)
            }
            Action::InputDeleteWord => (true, self.delete_word_back()),
            _ => (false, false),
        }
    }

    /// horizontally slice around cursor to fit within `max_cols` display columns
    pub fn visible_slice(&self, max_cols: usize) -> (String, String, String) {
        if max_cols == 0 {
            return (String::new(), String::new(), String::new());
        }
        let chars: Vec<(char, usize)> = self
            .text
            .chars()
            .map(|c| (c, UnicodeWidthChar::width(c).unwrap_or(0)))
            .collect();
        let len = chars.len();
        let cur = self.cursor.min(len);
        let cursor_w = if cur < len {
            chars[cur].1.max(1)
        } else {
            1 // trailing block cursor
        };

        // determine starting char index so cursor always fits in `max_cols`
        let mut start = 0usize;
        let mut used_before: usize = chars[..cur].iter().map(|(_, w)| *w).sum();
        while start < cur && used_before + cursor_w > max_cols {
            used_before = used_before.saturating_sub(chars[start].1);
            start += 1;
        }

        let mut before = String::new();
        let mut col = 0usize;
        for &(c, w) in &chars[start..cur] {
            if col + w + cursor_w > max_cols {
                break;
            }
            before.push(c);
            col += w;
        }

        let cursor_glyph = if cur < len {
            col += chars[cur].1;
            chars[cur].0.to_string()
        } else if col < max_cols {
            col += 1;
            " ".to_string()
        } else {
            String::new()
        };

        let mut after = String::new();
        if cur < len {
            for &(c, w) in &chars[cur + 1..] {
                if col + w > max_cols {
                    break;
                }
                after.push(c);
                col += w;
            }
        }

        (before, cursor_glyph, after)
    }

    pub fn render_spans(
        &self,
        max_cols: usize,
        active: bool,
        placeholder: &str,
    ) -> Vec<Span<'static>> {
        if self.text.is_empty() && !active {
            return vec![Span::styled(
                crate::text::trunc_cells(placeholder, max_cols),
                Style::default().fg(DIM).bg(Color::Reset),
            )];
        }
        if !active {
            return vec![Span::styled(
                crate::text::trunc_cells(&self.text, max_cols),
                Style::default().fg(TEXT).bg(Color::Reset),
            )];
        }
        let theme = active_theme();
        let (before, cursor_glyph, after) = self.visible_slice(max_cols);
        let mut spans = Vec::with_capacity(3);
        if !before.is_empty() {
            spans.push(Span::styled(
                before,
                Style::default()
                    .fg(TEXT)
                    .bg(theme.cursor_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        if !cursor_glyph.is_empty() {
            spans.push(Span::styled(
                cursor_glyph,
                Style::default()
                    .fg(theme.btn_primary_fg)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        if !after.is_empty() {
            spans.push(Span::styled(
                after,
                Style::default()
                    .fg(TEXT)
                    .bg(theme.cursor_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        spans
    }
}
