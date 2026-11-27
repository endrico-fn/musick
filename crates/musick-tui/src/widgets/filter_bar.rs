use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::text::cell_width;
use crate::theme::active_theme;
use crate::widgets::TextInput;

/// render a single-row inline `/` filter bar using `TextInput`
pub fn render_filter_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    input: &TextInput,
    active: bool,
    prefix: &str,
    placeholder: &str,
) {
    if area.width < 4 || area.height == 0 {
        return;
    }
    let pref_w = cell_width(prefix);
    let text_budget = usize::from(area.width).saturating_sub(pref_w);
    let mut spans = vec![Span::styled(
        prefix.to_string(),
        Style::default().fg(active_theme().accent).bg(Color::Reset),
    )];
    spans.extend(input.render_spans(text_budget, active, placeholder));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
