use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::theme::{ACCENT, DIM};

pub const BRAILLE_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const BRAILLE_STATIC: &str = "⣿";

/// Shared 80ms Braille dot loading indicator replacing all legacy skeleton placeholders.
#[derive(Debug, Clone, Copy, Default)]
pub struct BrailleSpinner {
    pub tick: u64,
    pub reduced_motion: bool,
}

impl BrailleSpinner {
    pub fn new(reduced_motion: bool) -> Self {
        Self {
            tick: 0,
            reduced_motion,
        }
    }

    pub fn advance(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn frame(&self) -> &'static str {
        if self.reduced_motion {
            BRAILLE_STATIC
        } else {
            BRAILLE_FRAMES[(self.tick as usize) % BRAILLE_FRAMES.len()]
        }
    }

    pub fn loading_line<'a>(&self, label: &'a str) -> Line<'a> {
        Line::from(vec![
            Span::styled(
                format!("{} ", self.frame()),
                Style::default().fg(ACCENT).bg(Color::Reset),
            ),
            Span::styled(label, Style::default().fg(DIM).bg(Color::Reset)),
        ])
    }

    pub fn inline_text(&self, label: &str) -> String {
        format!("{} {label}", self.frame())
    }
}
