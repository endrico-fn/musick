use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::text::{Align, align_cells, format_bytes, trunc_cells};
use crate::theme::{
    ACCENT, BORDER_DIM, CURSOR_BG, DIM, TEXT, accent_bold_style, dim_style, panel_block,
};
use crate::views::RenderCtx;
use crate::widgets::{render_filter_bar, render_scrollbar};

use super::SettingsView;
use super::rows::{SettingRow, format_setting_value};

impl SettingsView {
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx<'_>) {
        if area.width < 20 || area.height < 5 {
            return;
        }
        let cache_summary = format!(
            "Auth: {} · Cache: {}",
            ctx.auth_mode.as_str(),
            format_bytes(ctx.cache_stats.total_bytes)
        );
        let block = panel_block("Settings", Some(&cache_summary), false);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width < 10 || inner.height < 2 {
            return;
        }

        let show_filter = self.filter_active || self.has_filter();
        let show_desc = inner.height >= 4;
        let mut constraints = Vec::new();
        if show_filter {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Min(1));
        if show_desc {
            constraints.push(Constraint::Length(2));
        }
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let mut part_idx = 0;
        if show_filter {
            render_filter_bar(
                frame,
                parts[part_idx],
                &self.filter,
                self.filter_active,
                "/ ",
                "",
            );
            part_idx += 1;
        }
        let list_area = parts[part_idx];
        part_idx += 1;

        let visible = self.visible_rows();
        let selected_idx = self.cursor.min(visible.len().saturating_sub(1));
        let selected_row = visible.get(selected_idx).copied();

        enum DisplayLine {
            Header(&'static str),
            Item(usize, SettingRow),
        }
        let mut display: Vec<DisplayLine> = Vec::new();
        let mut last_group: Option<&'static str> = None;
        let mut selected_display_idx = 0usize;
        for (idx, &row) in visible.iter().enumerate() {
            let grp = row.group();
            if last_group != Some(grp) {
                display.push(DisplayLine::Header(grp));
                last_group = Some(grp);
            }
            if idx == selected_idx {
                selected_display_idx = display.len();
            }
            display.push(DisplayLine::Item(idx, row));
        }

        if display.is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "No settings match filter — press Esc to clear",
                    dim_style(),
                ))),
                list_area,
            );
        } else {
            let vis_h = usize::from(list_area.height);
            let total_lines = display.len();
            let mut scroll =
                self.scroll
                    .ensure_visible(selected_display_idx, vis_h, total_lines, 2);
            if selected_display_idx == 1 && scroll == 1 {
                scroll = 0;
            }

            let has_sb = total_lines > vis_h && list_area.width > 12;
            let content_w = usize::from(list_area.width).saturating_sub(if has_sb { 2 } else { 0 });
            let val_w = 24usize.min(content_w / 2);
            let label_w = content_w.saturating_sub(val_w + 4);

            for (row_offset, entry) in display.iter().skip(scroll).take(vis_h).enumerate() {
                let row_rect = Rect {
                    x: list_area.x,
                    y: list_area.y + row_offset as u16,
                    width: content_w as u16,
                    height: 1,
                };
                match entry {
                    DisplayLine::Header(group) => {
                        let title = format!("── {group} ");
                        let fill_len = content_w.saturating_sub(title.chars().count());
                        let line = Line::from(vec![
                            Span::styled(title, accent_bold_style()),
                            Span::styled(
                                "─".repeat(fill_len),
                                Style::default().fg(BORDER_DIM).bg(Color::Reset),
                            ),
                        ]);
                        frame.render_widget(Paragraph::new(line), row_rect);
                    }
                    DisplayLine::Item(idx, row) => {
                        let is_sel = *idx == selected_idx;
                        let (val, is_dim) = format_setting_value(*row, ctx);
                        let bg = if is_sel { CURSOR_BG } else { Color::Reset };
                        let pointer = if is_sel { "▸ " } else { "  " };
                        let label_style = if is_sel {
                            Style::default()
                                .fg(ACCENT)
                                .bg(bg)
                                .add_modifier(Modifier::BOLD)
                        } else if is_dim {
                            Style::default().fg(DIM).bg(bg)
                        } else {
                            Style::default().fg(TEXT).bg(bg)
                        };
                        let val_style = if is_sel {
                            Style::default()
                                .fg(TEXT)
                                .bg(bg)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(DIM).bg(bg)
                        };
                        let line = Line::from(vec![
                            Span::styled(pointer, label_style),
                            Span::styled(
                                align_cells(row.label(), label_w, Align::Left),
                                label_style,
                            ),
                            Span::styled("  ", Style::default().bg(bg)),
                            Span::styled(align_cells(&val, val_w, Align::Right), val_style),
                        ]);
                        frame.render_widget(Paragraph::new(line), row_rect);
                    }
                }
            }

            if has_sb {
                render_scrollbar(
                    frame.buffer_mut(),
                    list_area.x + list_area.width - 1,
                    list_area.y,
                    vis_h,
                    total_lines,
                    scroll,
                    true,
                );
            }
        }

        if show_desc {
            let desc_area = parts[part_idx];
            let w = usize::from(desc_area.width);
            let divider = Line::from(Span::styled(
                "─".repeat(w),
                Style::default().fg(BORDER_DIM).bg(Color::Reset),
            ));
            let desc_text = selected_row
                .map(|r| format!("{}: {}", r.group(), r.description()))
                .unwrap_or_default();
            let desc_line = Line::from(Span::styled(trunc_cells(&desc_text, w), dim_style()));
            frame.render_widget(Paragraph::new(vec![divider, desc_line]), desc_area);
        }
    }
}
