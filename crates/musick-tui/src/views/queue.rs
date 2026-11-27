use ratatui::Frame;
use ratatui::layout::Rect;

use musick_core::{AppCommand, QueueSnapshot};

use crate::keymap::Action;
use crate::text::Align;
use crate::theme::panel_block;
use crate::views::{RenderCtx, ViewEffect};
use crate::widgets::{ColumnDef, TableRow, TrackTable};

pub struct QueueView {
    pub cursor: usize,
}

impl Default for QueueView {
    fn default() -> Self {
        Self::new()
    }
}

impl QueueView {
    pub fn new() -> Self {
        Self { cursor: 0 }
    }

    pub fn footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("Enter", "play"),
            ("J/K", "reorder"),
            ("N", "play next"),
            ("dd", "remove"),
            ("u", "undo"),
            ("D", "dedup"),
            ("C", "clear up"),
            ("P/f", "list/like"),
        ]
    }

    pub fn handle_action(&mut self, action: Action, queue: &QueueSnapshot) -> ViewEffect {
        let len = queue.tracks.len();
        if len > 0 {
            self.cursor = self.cursor.min(len - 1);
        } else {
            self.cursor = 0;
        }

        match action {
            Action::MoveDown => {
                if len > 0 {
                    self.cursor = (self.cursor + 1).min(len - 1);
                }
                ViewEffect::None
            }
            Action::MoveUp => {
                self.cursor = self.cursor.saturating_sub(1);
                ViewEffect::None
            }
            Action::HalfPageDown => {
                if len > 0 {
                    self.cursor = (self.cursor + 8).min(len - 1);
                }
                ViewEffect::None
            }
            Action::HalfPageUp => {
                self.cursor = self.cursor.saturating_sub(8);
                ViewEffect::None
            }
            Action::JumpTop => {
                self.cursor = 0;
                ViewEffect::None
            }
            Action::JumpBottom => {
                self.cursor = len.saturating_sub(1);
                ViewEffect::None
            }
            Action::Activate | Action::PlayDirect => {
                if self.cursor < len {
                    ViewEffect::Command(AppCommand::JumpToQueueIndex { index: self.cursor })
                } else {
                    ViewEffect::None
                }
            }
            Action::MoveItemDown => {
                if self.cursor + 1 < len {
                    let from = self.cursor;
                    let to = from + 1;
                    self.cursor = to;
                    ViewEffect::Command(AppCommand::MoveQueueItem { from, to })
                } else {
                    ViewEffect::None
                }
            }
            Action::MoveItemUp => {
                if self.cursor > 0 && self.cursor < len {
                    let from = self.cursor;
                    let to = from - 1;
                    self.cursor = to;
                    ViewEffect::Command(AppCommand::MoveQueueItem { from, to })
                } else {
                    ViewEffect::None
                }
            }
            Action::PlayNext => {
                if self.cursor < len {
                    ViewEffect::Command(AppCommand::PromoteQueueItemToNext { index: self.cursor })
                } else {
                    ViewEffect::None
                }
            }
            Action::DeleteItem => {
                if self.cursor < len {
                    ViewEffect::Command(AppCommand::RemoveQueueItem { index: self.cursor })
                } else {
                    ViewEffect::None
                }
            }
            Action::Undo => ViewEffect::Command(AppCommand::UndoQueue),
            Action::Deduplicate => ViewEffect::Command(AppCommand::DeduplicateQueue),
            Action::ClearUpcoming => ViewEffect::Command(AppCommand::ClearUpcomingQueue),
            Action::AddToPlaylist => queue
                .tracks
                .get(self.cursor)
                .map(|t| ViewEffect::AddToPlaylistPicker(t.clone()))
                .unwrap_or(ViewEffect::None),
            Action::ToggleLike => queue
                .tracks
                .get(self.cursor)
                .map(|t| ViewEffect::ToggleLike(t.clone()))
                .unwrap_or(ViewEffect::None),
            Action::StartRadio => queue
                .tracks
                .get(self.cursor)
                .map(|t| {
                    ViewEffect::Command(AppCommand::StartRadio {
                        video_id: Some(t.video_id.clone()),
                        playlist_id: None,
                        seed_track: Some(t.clone()),
                    })
                })
                .unwrap_or(ViewEffect::None),
            Action::CopyUrl => queue
                .tracks
                .get(self.cursor)
                .map(|t| ViewEffect::CopyUrl(t.url()))
                .unwrap_or(ViewEffect::None),
            _ => ViewEffect::None,
        }
    }

    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx<'_>) {
        if area.width < 20 || area.height < 5 {
            return;
        }
        let sub = if ctx.queue.summary.is_empty() {
            "Empty"
        } else {
            ctx.queue.summary.as_str()
        };
        let block = panel_block("Queue", Some(sub), false);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let cur_idx = ctx.queue.index;
        let prio_end = cur_idx.map(|c| c + ctx.queue.priority_count);

        let rows: Vec<TableRow> = ctx
            .queue
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let role_lbl = match (cur_idx, prio_end) {
                    (Some(c), _) if i == c => "NOW".to_string(),
                    (Some(c), _) if i < c => "DONE".to_string(),
                    (Some(c), Some(end)) if i > c && i <= end => "NEXT".to_string(),
                    _ => format!("{:>3}", i + 1),
                };
                let mut r = TableRow::new(vec![
                    t.title.clone(),
                    t.artist_names(),
                    role_lbl,
                    t.duration_text(),
                ]);
                r.is_playing = cur_idx == Some(i);
                r.is_dim = cur_idx.is_some_and(|c| i < c);
                r
            })
            .collect();

        let cols = [
            ColumnDef {
                title: "Title",
                fixed_width: None,
                align: Align::Left,
            },
            ColumnDef {
                title: "Artist",
                fixed_width: Some(18),
                align: Align::Left,
            },
            ColumnDef {
                title: "Tier",
                fixed_width: Some(5),
                align: Align::Right,
            },
            ColumnDef {
                title: "Time",
                fixed_width: Some(6),
                align: Align::Right,
            },
        ];

        frame.render_widget(
            TrackTable {
                columns: &cols,
                rows: &rows,
                cursor: self.cursor,
                focused: true,
                show_header: true,
                loading: false,
                loading_label: "",
                empty_text: "Queue is empty — play a track from 1 Home or press / to search",
                icons: ctx.icons,
                spinner: ctx.spinner,
            },
            inner,
        );
    }
}
