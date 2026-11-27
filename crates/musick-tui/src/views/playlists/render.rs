use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use musick_core::PlaylistSource;

use crate::text::Align;
use crate::theme::panel_block;
use crate::views::RenderCtx;
use crate::widgets::{ColumnDef, TableRow, TrackTable, render_filter_bar};

use super::{PlaylistsPane, PlaylistsView};

impl PlaylistsView {
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx<'_>) {
        if area.width < 20 || area.height < 5 {
            return;
        }
        let left_w = ((u32::from(area.width) * 34) / 100).clamp(20, 38) as u16;
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(left_w),
                Constraint::Length(1),
                Constraint::Min(20),
            ])
            .split(area);

        let filtered = self.filtered_playlists();
        let list_sub = if ctx.syncing {
            ctx.spinner.inline_text("Syncing")
        } else {
            format!("{} lists", filtered.len())
        };
        let left_block = panel_block(
            "Playlists",
            Some(&list_sub),
            self.pane == PlaylistsPane::List,
        );
        let left_inner = left_block.inner(panes[0]);
        frame.render_widget(left_block, panes[0]);

        let (filter_rect, list_rect) = if self.filter_active || !self.filter.is_empty() {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(left_inner);
            (Some(split[0]), split[1])
        } else {
            (None, left_inner)
        };

        if let Some(fr) = filter_rect {
            render_filter_bar(frame, fr, &self.filter, self.filter_active, "/ ", "");
        }

        let pl_rows: Vec<TableRow> = filtered
            .iter()
            .map(|pl| {
                let mut row = TableRow::new(vec![pl.title.clone(), format!("{}", pl.track_count)]);
                row.prefix_icon = Some(if pl.source == PlaylistSource::YtmSynced {
                    ctx.icons.cloud
                } else {
                    ctx.icons.local
                });
                row
            })
            .collect();
        let pl_cols = [
            ColumnDef {
                title: "Playlist",
                fixed_width: None,
                align: Align::Left,
            },
            ColumnDef {
                title: "#",
                fixed_width: Some(4),
                align: Align::Right,
            },
        ];
        frame.render_widget(
            TrackTable {
                columns: &pl_cols,
                rows: &pl_rows,
                cursor: self.list_cursor,
                scroll: Some(&self.list_scroll),
                focused: self.pane == PlaylistsPane::List,
                show_header: false,
                loading: false,
                loading_label: "",
                empty_text: if self.filter.trim().is_empty() {
                    "Press c to create or I to import"
                } else {
                    "No playlists match filter"
                },
                icons: ctx.icons,
                spinner: ctx.spinner,
            },
            list_rect,
        );

        let visible = self.visible_tracks();
        let (right_title, right_sub) = match self.selected_playlist() {
            Some(pl) => {
                let src = match pl.source {
                    PlaylistSource::YtmSynced => "YTM Synced",
                    PlaylistSource::Local => "Local · U upload",
                };
                let count_text = if self.is_track_filtered() {
                    format!("{}/{} tracks", visible.len(), self.selected_tracks.len())
                } else {
                    format!("{} tracks", self.selected_tracks.len())
                };
                (
                    pl.title.as_str(),
                    format!("{count_text} · {} · {src}", self.track_sort.label()),
                )
            }
            None => ("Tracks", "No playlist selected".to_string()),
        };

        let right_block = panel_block(
            right_title,
            Some(&right_sub),
            self.pane == PlaylistsPane::Tracks,
        );
        let right_inner = right_block.inner(panes[2]);
        frame.render_widget(right_block, panes[2]);

        let (track_filter_rect, track_table_rect) =
            if self.track_filter_active || !self.track_filter.is_empty() {
                let split = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(1)])
                    .split(right_inner);
                (Some(split[0]), split[1])
            } else {
                (None, right_inner)
            };

        if let Some(fr) = track_filter_rect {
            render_filter_bar(
                frame,
                fr,
                &self.track_filter,
                self.track_filter_active,
                "/ ",
                "",
            );
        }

        let playing_vid = ctx.snapshot.track.as_ref().map(|t| t.video_id.as_str());
        let tr_rows: Vec<TableRow> = visible
            .iter()
            .map(|(orig_idx, t)| {
                let mut r = TableRow::new(vec![
                    format!("{:>2}. {}", orig_idx + 1, t.title),
                    t.artist_names(),
                    t.duration_text(),
                ]);
                r.is_playing = playing_vid == Some(t.video_id.as_str());
                r.is_dim = !t.is_available;
                r
            })
            .collect();
        let tr_cols = [
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
                title: "Time",
                fixed_width: Some(6),
                align: Align::Right,
            },
        ];

        frame.render_widget(
            TrackTable {
                columns: &tr_cols,
                rows: &tr_rows,
                cursor: self.track_cursor,
                scroll: Some(&self.track_scroll),
                focused: self.pane == PlaylistsPane::Tracks,
                show_header: true,
                loading: false,
                loading_label: "",
                empty_text: if self.is_track_filtered() {
                    "No tracks match filter"
                } else {
                    "Playlist is empty — press P on any song to add tracks here"
                },
                icons: ctx.icons,
                spinner: ctx.spinner,
            },
            track_table_rect,
        );
    }
}
