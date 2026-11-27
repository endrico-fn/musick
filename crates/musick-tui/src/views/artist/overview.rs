use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use musick_core::{AlbumSummary, Artist, ArtistSummary, SavedCollectionKind};

use crate::keymap::Action;
use crate::text::{cell_width, trunc_cells};
use crate::theme::{DIM, TEXT, accent_bold_style, active_theme, dim_style};
use crate::views::{RenderCtx, ViewEffect};
use crate::widgets::CoverRenderer;

use super::{ArtistView, compare_release_years};

#[derive(Debug, Clone)]
pub(super) enum OverviewSideItem {
    Release(AlbumSummary),
    Similar(ArtistSummary),
}

impl ArtistView {
    pub(super) fn overview_side_items(&self) -> Vec<OverviewSideItem> {
        let Some(ref art) = self.artist else {
            return Vec::new();
        };
        let mut releases: Vec<&AlbumSummary> =
            art.albums.iter().chain(art.singles.iter()).collect();
        releases.sort_by(|a, b| compare_release_years(a, b, true));
        let mut items: Vec<OverviewSideItem> = releases
            .into_iter()
            .take(5)
            .cloned()
            .map(OverviewSideItem::Release)
            .collect();
        items.extend(
            art.related
                .iter()
                .take(4)
                .cloned()
                .map(OverviewSideItem::Similar),
        );
        items
    }

    pub(super) fn handle_overview_side_action(&mut self, action: Action) -> ViewEffect {
        let items = self.overview_side_items();
        let len = items.len();
        if len > 0 {
            self.side_cursor = self.side_cursor.min(len - 1);
        } else {
            self.side_cursor = 0;
        }
        match action {
            Action::MoveDown => {
                if len > 0 {
                    self.side_cursor = (self.side_cursor + 1).min(len - 1);
                }
                ViewEffect::None
            }
            Action::MoveUp => {
                self.side_cursor = self.side_cursor.saturating_sub(1);
                ViewEffect::None
            }
            Action::HalfPageDown => {
                if len > 0 {
                    self.side_cursor = (self.side_cursor + 4).min(len - 1);
                }
                ViewEffect::None
            }
            Action::HalfPageUp => {
                self.side_cursor = self.side_cursor.saturating_sub(4);
                ViewEffect::None
            }
            Action::JumpTop => {
                self.side_cursor = 0;
                ViewEffect::None
            }
            Action::JumpBottom => {
                self.side_cursor = len.saturating_sub(1);
                ViewEffect::None
            }
            Action::Activate => match items.get(self.side_cursor) {
                Some(OverviewSideItem::Release(al)) => ViewEffect::OpenAlbum {
                    browse_id: al.browse_id.clone(),
                    title_hint: al.title.clone(),
                },
                Some(OverviewSideItem::Similar(ar)) => ViewEffect::OpenArtist {
                    browse_id: ar.browse_id.clone(),
                    name_hint: ar.name.clone(),
                },
                None => ViewEffect::None,
            },
            Action::PlayDirect => match items.get(self.side_cursor) {
                Some(OverviewSideItem::Release(al)) => ViewEffect::PlayAlbumDirect {
                    browse_id: al.browse_id.clone(),
                },
                Some(OverviewSideItem::Similar(ar)) => ViewEffect::PlayArtistDirect {
                    browse_id: ar.browse_id.clone(),
                },
                None => ViewEffect::None,
            },
            Action::PlayNext => match items.get(self.side_cursor) {
                Some(OverviewSideItem::Release(al)) => ViewEffect::QueueAlbum {
                    browse_id: al.browse_id.clone(),
                    play_next: true,
                },
                _ => ViewEffect::None,
            },
            Action::AppendQueue => match items.get(self.side_cursor) {
                Some(OverviewSideItem::Release(al)) => ViewEffect::QueueAlbum {
                    browse_id: al.browse_id.clone(),
                    play_next: false,
                },
                _ => ViewEffect::None,
            },
            Action::ToggleSave => match items.get(self.side_cursor) {
                Some(OverviewSideItem::Release(al)) => ViewEffect::ToggleSave {
                    kind: SavedCollectionKind::Album,
                    browse_id: al.browse_id.clone(),
                    title: al.title.clone(),
                    subtitle: Some(al.subtitle()),
                    thumb_url: al.cover_url(),
                },
                Some(OverviewSideItem::Similar(ar)) => ViewEffect::ToggleSave {
                    kind: SavedCollectionKind::Artist,
                    browse_id: ar.browse_id.clone(),
                    title: ar.name.clone(),
                    subtitle: ar.subtitle.clone(),
                    thumb_url: ar.cover_url(),
                },
                None => self.follow_profile_effect(),
            },
            Action::CopyUrl => match items.get(self.side_cursor) {
                Some(OverviewSideItem::Release(al)) => ViewEffect::CopyUrl(format!(
                    "https://music.youtube.com/browse/{}",
                    al.browse_id
                )),
                Some(OverviewSideItem::Similar(ar)) => ViewEffect::CopyUrl(format!(
                    "https://music.youtube.com/channel/{}",
                    ar.browse_id
                )),
                None => ViewEffect::CopyUrl(format!(
                    "https://music.youtube.com/channel/{}",
                    self.browse_id
                )),
            },
            _ => ViewEffect::None,
        }
    }

    pub(super) fn render_overview_side(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        art: &Artist,
        ctx: &RenderCtx<'_>,
        covers: &mut CoverRenderer,
        side_focused: bool,
    ) {
        if area.width < 10 || area.height == 0 {
            return;
        }
        let w = usize::from(area.width);
        let mut y = area.y;
        let max_y = area.y + area.height;

        let hdr_style = if side_focused {
            accent_bold_style()
        } else {
            dim_style().add_modifier(Modifier::BOLD)
        };
        let hdr_label = if side_focused {
            "▸ Latest & Top Releases"
        } else {
            "Latest & Top Releases"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                trunc_cells(hdr_label, w),
                hdr_style,
            ))),
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            },
        );
        y += 1;

        let mut releases: Vec<&AlbumSummary> =
            art.albums.iter().chain(art.singles.iter()).collect();
        releases.sort_by(|a, b| compare_release_years(a, b, true));
        let top_releases: Vec<&AlbumSummary> = releases.into_iter().take(5).collect();
        let total_side = top_releases.len() + art.related.len().min(4);
        let sel_side = if total_side > 0 {
            Some(self.side_cursor.min(total_side - 1))
        } else {
            None
        };

        let theme = active_theme();
        let mut list_start = 0usize;
        if let Some(latest) = top_releases.first()
            && y + 2 <= max_y
        {
            list_start = 1;
            let is_sel = side_focused && sel_side == Some(0);
            let row_bg = if is_sel {
                theme.cursor_bg
            } else {
                Color::Reset
            };
            let show_mini = covers.is_enabled() && area.width >= 18;
            let mini_w = if show_mini {
                covers
                    .mini_cover_width(2)
                    .min(area.width.saturating_sub(10))
            } else {
                0
            };
            if show_mini {
                covers.render_mini_cover(
                    frame,
                    Rect {
                        x: area.x,
                        y,
                        width: mini_w,
                        height: 2,
                    },
                    latest.cover_url().as_deref(),
                    ctx.icons.album,
                    is_sel,
                );
            }
            let tx = area.x + if show_mini { mini_w + 1 } else { 0 };
            let tw = usize::from(
                area.width
                    .saturating_sub(if show_mini { mini_w + 1 } else { 0 }),
            );
            let lead = if is_sel {
                format!("{} ", ctx.icons.cursor)
            } else {
                String::new()
            };
            let lead_w = cell_width(&lead);
            let buf = frame.buffer_mut();
            if row_bg != Color::Reset {
                for ry in y..y + 2 {
                    for cx in tx..tx + tw as u16 {
                        if let Some(cell) = buf.cell_mut((cx, ry)) {
                            cell.set_bg(row_bg);
                        }
                    }
                }
            }
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![
                        Span::styled(lead, Style::default().fg(theme.accent).bg(row_bg)),
                        Span::styled(
                            trunc_cells(&latest.title, tw.saturating_sub(lead_w)),
                            Style::default()
                                .fg(if is_sel { theme.accent } else { TEXT })
                                .bg(row_bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(Span::styled(
                        trunc_cells(&latest.subtitle(), tw),
                        Style::default().fg(DIM).bg(row_bg),
                    )),
                ]),
                Rect {
                    x: tx,
                    y,
                    width: tw as u16,
                    height: 2,
                },
            );
            y += if show_mini { 3 } else { 2 };
        }

        let mut side_idx = list_start;
        for al in top_releases.iter().skip(list_start) {
            if y >= max_y {
                break;
            }
            let is_sel = sel_side == Some(side_idx);
            let bg = if is_sel {
                if side_focused {
                    theme.cursor_bg
                } else {
                    theme.cursor_blur_bg
                }
            } else {
                Color::Reset
            };
            let yr = al.year.as_deref().unwrap_or(&al.kind);
            let suffix = format!(" ({yr})");
            let lead = if is_sel && side_focused {
                format!("{} ", ctx.icons.cursor)
            } else {
                format!("{} ", ctx.icons.album)
            };
            let title_budget = w.saturating_sub(cell_width(&suffix) + cell_width(&lead));
            render_side_row(
                frame,
                Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                },
                bg,
                vec![
                    Span::styled(lead, Style::default().fg(theme.accent).bg(bg)),
                    Span::styled(
                        trunc_cells(&al.title, title_budget),
                        Style::default()
                            .fg(if is_sel && side_focused {
                                theme.accent
                            } else {
                                TEXT
                            })
                            .bg(bg),
                    ),
                    Span::styled(suffix, Style::default().fg(DIM).bg(bg)),
                ],
            );
            y += 1;
            side_idx += 1;
        }

        if !art.related.is_empty() && y + 2 <= max_y {
            y += 1;
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    trunc_cells("Fans Also Like", w),
                    hdr_style,
                ))),
                Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                },
            );
            y += 1;
            for rel in art.related.iter().take(4) {
                if y >= max_y {
                    break;
                }
                let is_sel = sel_side == Some(side_idx);
                let bg = if is_sel {
                    if side_focused {
                        theme.cursor_bg
                    } else {
                        theme.cursor_blur_bg
                    }
                } else {
                    Color::Reset
                };
                let lead = if is_sel && side_focused {
                    format!("{} ", ctx.icons.cursor)
                } else {
                    format!("{} ", ctx.icons.artist)
                };
                let lead_w = cell_width(&lead);
                render_side_row(
                    frame,
                    Rect {
                        x: area.x,
                        y,
                        width: area.width,
                        height: 1,
                    },
                    bg,
                    vec![
                        Span::styled(
                            lead,
                            Style::default()
                                .fg(if is_sel && side_focused {
                                    theme.accent
                                } else {
                                    DIM
                                })
                                .bg(bg),
                        ),
                        Span::styled(
                            trunc_cells(&rel.name, w.saturating_sub(lead_w)),
                            Style::default()
                                .fg(if is_sel && side_focused {
                                    theme.accent
                                } else {
                                    TEXT
                                })
                                .bg(bg),
                        ),
                    ],
                );
                y += 1;
                side_idx += 1;
            }
        }
    }
}

fn render_side_row(frame: &mut Frame<'_>, rect: Rect, bg: Color, spans: Vec<Span<'static>>) {
    if bg != Color::Reset {
        let buf = frame.buffer_mut();
        for cx in rect.x..rect.x + rect.width {
            if let Some(cell) = buf.cell_mut((cx, rect.y)) {
                cell.set_bg(bg);
            }
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), rect);
}
