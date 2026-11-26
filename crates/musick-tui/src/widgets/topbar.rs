use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use musick_core::AuthMode;

use crate::text::cell_width;
use crate::theme::{ACCENT, BORDER, DIM, IconSet, TEXT, WARN_GOLD};
use crate::widgets::BrailleSpinner;

pub const ROOT_TABS: &[(&str, &str)] = &[
    ("home", "1 Home"),
    ("playlists", "2 Playlists"),
    ("library", "3 Library"),
    ("queue", "4 Queue"),
    ("settings", "5 Settings"),
];

pub const ROOT_TABS_COMPACT: &[(&str, &str)] = &[
    ("home", "1 Home"),
    ("playlists", "2 Lists"),
    ("library", "3 Lib"),
    ("queue", "4 Queue"),
    ("settings", "5 Set"),
];

pub struct TopBar<'a> {
    pub active_tab: usize,
    pub icons: IconSet,
    pub auth_mode: AuthMode,
    pub syncing: bool,
    pub pending_sync: usize,
    pub spinner: BrailleSpinner,
    pub status_toast: Option<&'a str>,
}

impl Widget for TopBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let w = usize::from(area.width);
        let compact = w < 84;
        let tabs = if compact { ROOT_TABS_COMPACT } else { ROOT_TABS };
        let gap_w = if w >= 100 { 3 } else if w >= 84 { 2 } else { 1 };
        let gap = " ".repeat(gap_w);

        let mut spans: Vec<Span<'_>> = Vec::new();
        let mut x = 0usize;

        spans.push(Span::raw(" "));
        x += 1;

        let brand = format!("{} ", self.icons.brand);
        x += cell_width(&brand) + cell_width("musick");
        spans.push(Span::styled(
            brand,
            Style::default().fg(ACCENT).bg(Color::Reset),
        ));
        spans.push(Span::styled(
            "musick",
            Style::default()
                .fg(TEXT)
                .bg(Color::Reset)
                .add_modifier(Modifier::BOLD),
        ));

        spans.push(Span::raw(gap.clone()));
        x += gap_w;

        let mut underline_span: Option<(usize, usize)> = None;
        for (idx, &(_, label)) in tabs.iter().enumerate() {
            if idx > 0 {
                spans.push(Span::raw(gap.clone()));
                x += gap_w;
            }
            let start = x;
            let is_act = idx == self.active_tab;
            let style = if is_act {
                Style::default()
                    .fg(ACCENT)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(DIM).bg(Color::Reset)
            };
            spans.push(Span::styled(label, style));
            x += cell_width(label);
            if is_act {
                underline_span = Some((start, x));
            }
        }

        let (badge_text, dot_color) = if let Some(toast) = self.status_toast {
            (format!("{toast} "), ACCENT)
        } else if self.syncing {
            (format!("{} syncing ", self.spinner.frame()), ACCENT)
        } else {
            match self.auth_mode {
                AuthMode::Anonymous => ("no cookies ".to_string(), ACCENT),
                AuthMode::OAuth => {
                    if self.pending_sync > 0 {
                        (format!("oauth · {}↑ ", self.pending_sync), WARN_GOLD)
                    } else {
                        ("oauth ".to_string(), ACCENT)
                    }
                }
                AuthMode::Browser => {
                    if self.pending_sync > 0 {
                        (format!("browser · {}↑ ", self.pending_sync), WARN_GOLD)
                    } else {
                        ("browser ".to_string(), ACCENT)
                    }
                }
            }
        };

        let inner_right = w.saturating_sub(1);
        let dot_w = cell_width(self.icons.dot);
        let full_badge_w = cell_width(&badge_text) + dot_w;

        if inner_right > x + full_badge_w {
            let pad = inner_right - x - full_badge_w;
            spans.push(Span::raw(" ".repeat(pad)));
            spans.push(Span::styled(
                badge_text,
                Style::default().fg(DIM).bg(Color::Reset),
            ));
            spans.push(Span::styled(
                self.icons.dot,
                Style::default().fg(dot_color).bg(Color::Reset),
            ));
        } else if inner_right > x + dot_w {
            let pad = inner_right - x - dot_w;
            spans.push(Span::raw(" ".repeat(pad)));
            spans.push(Span::styled(
                self.icons.dot,
                Style::default().fg(dot_color).bg(Color::Reset),
            ));
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

        if area.height >= 2 {
            let line2 = match underline_span {
                Some((s, e)) if s < w => {
                    let e = e.min(w);
                    let mut l2 = Vec::new();
                    if s > 0 {
                        l2.push(Span::styled(
                            "─".repeat(s),
                            Style::default().fg(BORDER).bg(Color::Reset),
                        ));
                    }
                    if e > s {
                        l2.push(Span::styled(
                            "━".repeat(e - s),
                            Style::default().fg(ACCENT).bg(Color::Reset),
                        ));
                    }
                    if e < w {
                        l2.push(Span::styled(
                            "─".repeat(w - e),
                            Style::default().fg(BORDER).bg(Color::Reset),
                        ));
                    }
                    Line::from(l2)
                }
                _ => Line::from(Span::styled(
                    "─".repeat(w),
                    Style::default().fg(BORDER).bg(Color::Reset),
                )),
            };
            line2.render(
                Rect {
                    x: area.x,
                    y: area.y + 1,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
        }
    }
}
