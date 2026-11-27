use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use musick_core::{Artist, Track};

use crate::text::{cell_width, trunc_cells, wrap_cells};
use crate::theme::{
    BORDER_DIM, accent_bold_style, accent_style, base_style, dim_style, panel_block,
};
use crate::views::RenderCtx;
use crate::views::detail_hero::{DetailCover, render_detail_cover, split_detail_hero};
use crate::widgets::{CoverRenderer, MediaListItem, render_filter_bar, render_media_list};

use super::{ArtistOverviewPane, ArtistTab, ArtistView, SPLIT_BREAKPOINT};

impl ArtistView {
    pub fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        ctx: &RenderCtx<'_>,
        covers: &mut CoverRenderer,
    ) {
        if area.width < 20 || area.height < 8 {
            return;
        }
        let hero = split_detail_hero(area, covers);

        let cover_url = self.artist.as_ref().and_then(|a| a.summary.cover_url());
        render_detail_cover(
            frame,
            DetailCover {
                area: hero.cover_area,
                title: "Artist",
                url: cover_url.as_deref(),
                entity_loading: self.loading,
                placeholder_glyph: ctx.icons.artist,
            },
            ctx.spinner,
            covers,
        );

        let info_sub = if self.saved {
            "Saved ✓ · W unfollow · x shuffle · i bio"
        } else {
            "W follow · r radio · x shuffle · i bio"
        };
        let info_block = panel_block("Profile", Some(info_sub), false);
        let info_inner = info_block.inner(hero.info_area);
        frame.render_widget(info_block, hero.info_area);

        if self.loading && self.artist.is_none() {
            frame.render_widget(
                Paragraph::new(ctx.spinner.loading_line("Loading artist profile…")),
                info_inner,
            );
        } else if let Some(ref art) = self.artist {
            let w = usize::from(info_inner.width);
            let max_lines = usize::from(info_inner.height);
            let mut lines = vec![Line::from(Span::styled(
                trunc_cells(&art.summary.name, w),
                base_style().add_modifier(Modifier::BOLD),
            ))];
            let mut stats = Vec::new();
            if let Some(ref l) = art.listeners {
                stats.push(l.clone());
            }
            if let Some(ref s) = art.subscribers {
                if s.to_ascii_lowercase().contains("subscriber") {
                    stats.push(s.clone());
                } else {
                    stats.push(format!("{s} subscribers"));
                }
            }
            if let Some(ref v) = art.views
                && !v.trim().is_empty()
            {
                stats.push(v.clone());
            }
            if !stats.is_empty() && lines.len() < max_lines {
                lines.push(Line::from(Span::styled(
                    trunc_cells(&stats.join(" · "), w),
                    accent_style(),
                )));
            }
            let counts = catalog_summary_counts(art);
            if !counts.is_empty() && lines.len() < max_lines && max_lines >= 4 {
                lines.push(Line::from(Span::styled(
                    trunc_cells(&counts, w),
                    dim_style(),
                )));
            }
            if let Some(ref desc) = art.description {
                let prose: String = desc.split_whitespace().collect::<Vec<_>>().join(" ");
                if !prose.is_empty() && lines.len() < max_lines {
                    let rem_rows = max_lines.saturating_sub(lines.len());
                    if rem_rows > 0 {
                        let wrapped = wrap_cells(&prose, w);
                        let clipped = wrapped.len() > rem_rows;
                        for (idx, mut ln) in wrapped.into_iter().take(rem_rows).enumerate() {
                            if clipped && idx + 1 == rem_rows {
                                let trimmed = ln.trim_end_matches(['.', ',', ' ']);
                                ln = trunc_cells(&format!("{trimmed}… [i]"), w);
                            }
                            lines.push(Line::from(Span::styled(ln, dim_style())));
                        }
                    }
                }
            }
            frame.render_widget(Paragraph::new(lines), info_inner);
        }

        let body_block = panel_block(
            "Discography & Tracks",
            Some("Tab switch · / filter · Bksp back"),
            false,
        );
        let body_inner = body_block.inner(hero.body_area);
        frame.render_widget(body_block, hero.body_area);

        if body_inner.height < 2 || body_inner.width < 10 {
            return;
        }
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
        let body_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(body_inner);

        let split_active = self.active_tab == ArtistTab::Overview
            && area.width >= SPLIT_BREAKPOINT
            && body_inner.width >= 60;
        self.overview_split_active.set(split_active);

        let right_hint = if matches!(self.active_tab, ArtistTab::Albums | ArtistTab::Singles) {
            Some(if self.sort_newest {
                "Newest ▾"
            } else {
                "Oldest ▴"
            })
        } else if split_active {
            Some("h/l pane")
        } else {
            None
        };
        self.render_artist_tabs(frame, body_split[0], right_hint);

        let content_area = if show_filter {
            render_filter_bar(
                frame,
                body_split[1],
                &self.filter,
                self.filter_active,
                "/ ",
                "",
            );
            body_split[2]
        } else {
            body_split[1]
        };

        if self.loading && self.artist.is_none() {
            frame.render_widget(
                Paragraph::new(ctx.spinner.loading_line("Fetching catalog…")),
                content_area,
            );
            return;
        }

        let Some(ref art) = self.artist else {
            return;
        };
        let playing_vid = ctx.snapshot.track.as_ref().map(|t| t.video_id.as_str());
        let q = self.filter.trim().to_lowercase();

        match self.active_tab {
            ArtistTab::Overview => {
                let top_songs = self.filtered_tracks(&art.songs, Some(10));
                if split_active && content_area.width >= 60 {
                    let cols = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Percentage(60),
                            Constraint::Length(3),
                            Constraint::Min(22),
                        ])
                        .split(content_area);

                    let left_focused = self.overview_pane == ArtistOverviewPane::TopSongs;
                    let left_split = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(1), Constraint::Min(1)])
                        .split(cols[0]);
                    let left_hdr_style = if left_focused {
                        accent_bold_style()
                    } else {
                        dim_style().add_modifier(Modifier::BOLD)
                    };
                    let left_title = if left_focused {
                        "▸ Top Songs"
                    } else {
                        "  Top Songs"
                    };
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(left_title, left_hdr_style))),
                        left_split[0],
                    );
                    self.render_track_media_list(
                        frame,
                        left_split[1],
                        &top_songs,
                        playing_vid,
                        ctx,
                        covers,
                        true,
                        left_focused,
                    );

                    let div_x = cols[1].x + 1;
                    let buf = frame.buffer_mut();
                    for dy in cols[1].y..cols[1].y + cols[1].height {
                        if let Some(cell) = buf.cell_mut((div_x, dy)) {
                            cell.set_symbol("│");
                            cell.set_fg(BORDER_DIM);
                            cell.set_bg(Color::Reset);
                        }
                    }

                    self.render_overview_side(frame, cols[2], art, ctx, covers, !left_focused);
                } else {
                    self.render_track_media_list(
                        frame,
                        content_area,
                        &top_songs,
                        playing_vid,
                        ctx,
                        covers,
                        true,
                        true,
                    );
                }
            }
            ArtistTab::Songs => {
                let tracks = self.filtered_tracks(&art.songs, None);
                self.render_track_media_list(
                    frame,
                    content_area,
                    &tracks,
                    playing_vid,
                    ctx,
                    covers,
                    true,
                    true,
                );
            }
            ArtistTab::Videos => {
                let tracks = self.filtered_tracks(&art.videos, None);
                self.render_track_media_list(
                    frame,
                    content_area,
                    &tracks,
                    playing_vid,
                    ctx,
                    covers,
                    false,
                    true,
                );
            }
            ArtistTab::Albums | ArtistTab::Singles => {
                let items: Vec<MediaListItem> = self
                    .filtered_releases(self.active_tab == ArtistTab::Singles)
                    .into_iter()
                    .map(|al| {
                        let art_names = al
                            .artists
                            .iter()
                            .map(|a| a.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        MediaListItem {
                            title: al.title.clone(),
                            subtitle: if art_names.is_empty() {
                                al.subtitle()
                            } else {
                                art_names
                            },
                            meta_top: al.kind.clone(),
                            meta_bottom: al.year.clone().unwrap_or_default(),
                            cover_url: al.cover_url(),
                            glyph: ctx.icons.album,
                            is_playing: false,
                            is_dim: false,
                        }
                    })
                    .collect();
                render_media_list(
                    frame,
                    content_area,
                    &items,
                    self.cursor,
                    Some(&self.scroll),
                    true,
                    false,
                    "",
                    if self.filter.is_empty() {
                        "No releases found"
                    } else {
                        "No releases match filter"
                    },
                    ctx.icons,
                    ctx.spinner,
                    covers,
                );
            }
            ArtistTab::Playlists => {
                let items: Vec<MediaListItem> = art
                    .playlists
                    .iter()
                    .filter(|p| {
                        q.is_empty()
                            || p.title.to_lowercase().contains(&q)
                            || p.author
                                .as_deref()
                                .is_some_and(|a| a.to_lowercase().contains(&q))
                    })
                    .map(|p| MediaListItem {
                        title: p.title.clone(),
                        subtitle: p.author.clone().unwrap_or_else(|| "Playlist".to_string()),
                        meta_top: "Playlist".to_string(),
                        meta_bottom: p.count.clone().unwrap_or_default(),
                        cover_url: p.cover_url(),
                        glyph: ctx.icons.playlist,
                        is_playing: false,
                        is_dim: false,
                    })
                    .collect();
                render_media_list(
                    frame,
                    content_area,
                    &items,
                    self.cursor,
                    Some(&self.scroll),
                    true,
                    false,
                    "",
                    if self.filter.is_empty() {
                        "No playlists featured"
                    } else {
                        "No playlists match filter"
                    },
                    ctx.icons,
                    ctx.spinner,
                    covers,
                );
            }
            ArtistTab::Similar => {
                let items: Vec<MediaListItem> = art
                    .related
                    .iter()
                    .filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q))
                    .map(|r| MediaListItem {
                        title: r.name.clone(),
                        subtitle: r.subtitle.clone().unwrap_or_else(|| "Artist".to_string()),
                        meta_top: "Artist".to_string(),
                        meta_bottom: String::new(),
                        cover_url: r.cover_url(),
                        glyph: ctx.icons.artist,
                        is_playing: false,
                        is_dim: false,
                    })
                    .collect();
                render_media_list(
                    frame,
                    content_area,
                    &items,
                    self.cursor,
                    Some(&self.scroll),
                    true,
                    false,
                    "",
                    if self.filter.is_empty() {
                        "No similar artists found"
                    } else {
                        "No similar artists match filter"
                    },
                    ctx.icons,
                    ctx.spinner,
                    covers,
                );
            }
        }
    }

    fn render_artist_tabs(&self, frame: &mut Frame<'_>, area: Rect, right_hint: Option<&str>) {
        if area.height == 0 || area.width < 6 {
            return;
        }
        let compact = area.width < 88;
        let tab_labels: Vec<String> = match (&self.artist, compact) {
            (Some(art), false) => vec![
                "Overview".to_string(),
                count_label("Songs", art.songs.len()),
                count_label("Albums", art.albums.len()),
                count_label("Singles", art.singles.len()),
                count_label("Videos", art.videos.len()),
                count_label("Playlists", art.playlists.len()),
                count_label("Similar", art.related.len()),
            ],
            (_, false) => vec![
                "Overview".into(),
                "Songs".into(),
                "Albums".into(),
                "Singles".into(),
                "Videos".into(),
                "Playlists".into(),
                "Similar".into(),
            ],
            (_, true) => vec![
                "Overv.".into(),
                "Songs".into(),
                "Albums".into(),
                "Singles".into(),
                "Videos".into(),
                "Lists".into(),
                "Sim.".into(),
            ],
        };

        let mut spans = Vec::new();
        let mut used = 0usize;
        let active_idx = self.active_tab.index();
        for (i, label) in tab_labels.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" · ", dim_style()));
                used += 3;
            }
            let style = if i == active_idx {
                accent_bold_style().add_modifier(Modifier::UNDERLINED)
            } else {
                dim_style()
            };
            used += cell_width(label);
            spans.push(Span::styled(label.clone(), style));
        }

        if let Some(hint) = right_hint {
            let hint_w = cell_width(hint);
            let total_w = usize::from(area.width);
            if total_w > used + hint_w + 1 {
                spans.push(Span::raw(" ".repeat(total_w - used - hint_w)));
                spans.push(Span::styled(hint, dim_style()));
            }
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    #[allow(clippy::too_many_arguments)]
    fn render_track_media_list(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        tracks: &[&Track],
        playing_vid: Option<&str>,
        ctx: &RenderCtx<'_>,
        covers: &mut CoverRenderer,
        numbered: bool,
        focused: bool,
    ) {
        let items: Vec<MediaListItem> = tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let title = if numbered {
                    format!("{:>2}. {}", i + 1, t.title)
                } else {
                    t.title.clone()
                };
                let subtitle = match t.album.as_deref().filter(|a| !a.is_empty()) {
                    Some(alb) if !alb.eq_ignore_ascii_case(&t.title) => {
                        format!("{} · {alb}", t.artist_names())
                    }
                    _ => t.artist_names(),
                };
                MediaListItem {
                    title,
                    subtitle,
                    meta_top: t.plays.clone().unwrap_or_default(),
                    meta_bottom: t.duration_text(),
                    cover_url: t.cover_url(),
                    glyph: if t.is_video() {
                        ctx.icons.video
                    } else {
                        ctx.icons.song
                    },
                    is_playing: playing_vid == Some(t.video_id.as_str()),
                    is_dim: !t.is_available,
                }
            })
            .collect();
        render_media_list(
            frame,
            area,
            &items,
            self.cursor,
            Some(&self.scroll),
            focused,
            false,
            "",
            if self.filter.is_empty() {
                "No tracks available"
            } else {
                "No tracks match filter"
            },
            ctx.icons,
            ctx.spinner,
            covers,
        );
    }
}

fn count_label(name: &str, count: usize) -> String {
    if count > 0 {
        format!("{name} ({count})")
    } else {
        name.to_string()
    }
}

fn catalog_summary_counts(art: &Artist) -> String {
    let mut parts = Vec::new();
    if !art.songs.is_empty() {
        parts.push(format!("{} songs", art.songs.len()));
    }
    if !art.albums.is_empty() {
        parts.push(format!("{} albums", art.albums.len()));
    }
    if !art.singles.is_empty() {
        parts.push(format!("{} singles", art.singles.len()));
    }
    if !art.videos.is_empty() {
        parts.push(format!("{} videos", art.videos.len()));
    }
    parts.join(" · ")
}
