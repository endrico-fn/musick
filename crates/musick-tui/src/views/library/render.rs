use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use musick_core::{PlaylistSource, SavedCollection, SavedCollectionKind};

use crate::text::Align;
use crate::theme::panel_block;
use crate::views::RenderCtx;
use crate::widgets::{
    ColumnDef, CoverRenderer, MediaListItem, SectionTabs, TableRow, TrackTable, render_filter_bar,
    render_media_list,
};

use super::{LibraryTab, LibraryView};

const LIBRARY_TAB_LABELS: &[(&str, &str)] = &[
    ("User Playlists", "Playlists"),
    ("Liked Songs", "Liked"),
    ("Saved Albums", "Albums"),
    ("Saved Artists", "Artists"),
    ("Saved Playlists", "Saved Pl"),
    ("Recent", "Recent"),
];

impl LibraryView {
    pub fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        ctx: &RenderCtx<'_>,
        covers: &mut CoverRenderer,
    ) {
        if area.width < 20 || area.height < 5 {
            return;
        }
        let sub = format!("{} items", self.current_len());
        let block = panel_block("Library", Some(&sub), false);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let show_filter = self.filter_active || !self.filter.is_empty();
        let constraints = if show_filter {
            vec![
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ]
        } else {
            vec![Constraint::Length(1), Constraint::Min(1)]
        };
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        frame.render_widget(
            SectionTabs {
                tabs: LIBRARY_TAB_LABELS,
                active: self.active_tab.index(),
                right_hint: Some("Tab switch"),
            },
            parts[0],
        );

        let body_rect = if show_filter {
            render_filter_bar(frame, parts[1], &self.filter, self.filter_active, "/ ", "");
            parts[2]
        } else {
            parts[1]
        };

        let q = self.filter.trim().to_lowercase();
        let playing_vid = ctx.snapshot.track.as_ref().map(|t| t.video_id.as_str());

        match self.active_tab {
            LibraryTab::UserPlaylists => {
                let rows: Vec<TableRow> = self
                    .user_playlists
                    .iter()
                    .filter(|p| q.is_empty() || p.title.to_lowercase().contains(&q))
                    .map(|p| {
                        let mut r = TableRow::new(vec![
                            p.title.clone(),
                            if p.source == PlaylistSource::YtmSynced {
                                "YTM Synced".to_string()
                            } else {
                                "Local".to_string()
                            },
                            format!("{} tracks", p.track_count),
                        ]);
                        r.prefix_icon = Some(if p.source == PlaylistSource::YtmSynced {
                            ctx.icons.cloud
                        } else {
                            ctx.icons.local
                        });
                        r
                    })
                    .collect();
                let cols = [
                    ColumnDef {
                        title: "Playlist",
                        fixed_width: None,
                        align: Align::Left,
                    },
                    ColumnDef {
                        title: "Source",
                        fixed_width: Some(12),
                        align: Align::Left,
                    },
                    ColumnDef {
                        title: "Count",
                        fixed_width: Some(10),
                        align: Align::Right,
                    },
                ];
                frame.render_widget(
                    TrackTable {
                        columns: &cols,
                        rows: &rows,
                        cursor: self.cursor,
                        scroll: Some(&self.scroll),
                        focused: true,
                        show_header: true,
                        loading: false,
                        loading_label: "",
                        empty_text: "No user playlists — open 2 Playlists and press c to create",
                        icons: ctx.icons,
                        spinner: ctx.spinner,
                    },
                    body_rect,
                );
            }
            LibraryTab::LikedSongs | LibraryTab::Recent => {
                let is_liked = self.active_tab == LibraryTab::LikedSongs;
                let list = if is_liked {
                    &self.liked_songs
                } else {
                    &self.recent_tracks
                };
                let glyph = if is_liked {
                    ctx.icons.liked
                } else {
                    ctx.icons.song
                };
                let items: Vec<MediaListItem> = list
                    .iter()
                    .filter(|t| Self::matches_track(t, &q))
                    .map(|t| {
                        let mut sub = t.artist_names();
                        if let Some(ref alb) = t.album
                            && !alb.is_empty()
                            && !alb.eq_ignore_ascii_case(&t.title)
                        {
                            sub = format!("{sub} · {alb}");
                        }
                        let meta_top = t.plays.clone().unwrap_or_else(|| {
                            if is_liked {
                                ctx.icons.liked.to_string()
                            } else {
                                String::new()
                            }
                        });
                        MediaListItem {
                            title: t.title.clone(),
                            subtitle: sub,
                            meta_top,
                            meta_bottom: t.duration_text(),
                            cover_url: t.cover_url(),
                            glyph,
                            is_playing: playing_vid == Some(t.video_id.as_str()),
                            is_dim: !t.is_available,
                        }
                    })
                    .collect();
                let empty = if is_liked {
                    "No liked songs yet — press f on any song to like it"
                } else {
                    "No recent plays yet (enable Save History in Settings to persist across restarts)"
                };
                render_media_list(
                    frame,
                    body_rect,
                    &items,
                    self.cursor,
                    Some(&self.scroll),
                    true,
                    false,
                    "",
                    empty,
                    ctx.icons,
                    ctx.spinner,
                    covers,
                );
            }
            LibraryTab::SavedAlbums | LibraryTab::SavedArtists | LibraryTab::SavedPlaylists => {
                let (list, glyph, empty) = match self.active_tab {
                    LibraryTab::SavedAlbums => (
                        &self.saved_albums,
                        ctx.icons.album,
                        "No saved albums — press w on an album to save it",
                    ),
                    LibraryTab::SavedArtists => (
                        &self.saved_artists,
                        ctx.icons.artist,
                        "No saved artists — press w on an artist to follow/save",
                    ),
                    _ => (
                        &self.saved_playlists,
                        ctx.icons.playlist,
                        "No saved community playlists — press w on a playlist to save it",
                    ),
                };
                let filtered: Vec<&SavedCollection> = list
                    .iter()
                    .filter(|c| Self::matches_collection(c, &q))
                    .collect();

                let items: Vec<MediaListItem> = filtered
                    .iter()
                    .map(|c| MediaListItem {
                        title: c.title.clone(),
                        subtitle: c.subtitle.clone().unwrap_or_default(),
                        meta_top: match c.kind {
                            SavedCollectionKind::Album => "Album".to_string(),
                            SavedCollectionKind::Artist => "Artist".to_string(),
                            SavedCollectionKind::Playlist => "Playlist".to_string(),
                        },
                        meta_bottom: String::new(),
                        cover_url: c.thumb_url.clone(),
                        glyph,
                        is_playing: false,
                        is_dim: false,
                    })
                    .collect();
                render_media_list(
                    frame,
                    body_rect,
                    &items,
                    self.cursor,
                    Some(&self.scroll),
                    true,
                    false,
                    "",
                    empty,
                    ctx.icons,
                    ctx.spinner,
                    covers,
                );
            }
        }
    }
}
