use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use musick_core::{SearchCategory, Track};

use crate::text::{Align, trunc_cells};
use crate::theme::{accent_style, base_style, cover_panel_block, dim_style, panel_block};
use crate::views::RenderCtx;
use crate::widgets::{
    CavaWidget, ColumnDef, CoverRenderer, LyricsBox, MediaListItem, SectionTabs, TableRow,
    TrackTable, render_filter_bar, render_media_list,
};

use super::discover::{item_to_media_item, item_to_table_row};
use super::{DiscoverTab, PlayerFocusPane, PlayerPanelMode, PlayerView};

const DISCOVER_TABS: &[(&str, &str)] = &[
    ("Mixes", "Mixes"),
    ("New Releases", "New Rel."),
    ("Featured", "Featured"),
];

const SEARCH_TABS: &[(&str, &str)] = &[
    ("All", "All"),
    ("Songs", "Songs"),
    ("Videos", "Videos"),
    ("Albums", "Albums"),
    ("Artists", "Artists"),
    ("Playlists", "Lists"),
    ("Community", "Comm."),
];

impl PlayerView {
    pub fn render(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        ctx: &RenderCtx<'_>,
        covers: &mut CoverRenderer,
    ) {
        if area.width < 20 || area.height < 8 {
            return;
        }

        let show_cover = covers.is_enabled();
        let base_top_h = ((u32::from(area.height) * 61) / 100)
            .max(6)
            .min(u32::from(area.height.saturating_sub(5))) as u16;
        let top_h = if show_cover {
            let max_cover_w = ((u32::from(area.width) * 44) / 100)
                .min(u32::from(area.width.saturating_sub(24)))
                .max(12) as u16;
            let max_square_h = (((f64::from(max_cover_w.saturating_sub(1))) / covers.cell_aspect())
                .floor() as u16)
                .saturating_add(1)
                .max(6);
            base_top_h.min(max_square_h).max(6)
        } else {
            base_top_h
        };

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(top_h), Constraint::Min(5)])
            .split(area);

        if show_cover {
            let max_cover_w = ((u32::from(area.width) * 44) / 100)
                .min(u32::from(area.width.saturating_sub(24)))
                .max(12) as u16;
            let cover_w = covers.square_panel_width(top_h, max_cover_w);
            let top_cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(cover_w),
                    Constraint::Length(1),
                    Constraint::Min(20),
                ])
                .split(rows[0]);

            let cover_block = cover_panel_block("Cover", None, false);
            let cover_inner = cover_block.inner(top_cols[0]);
            frame.render_widget(cover_block, top_cols[0]);
            let cover_url = ctx.snapshot.track.as_ref().and_then(Track::cover_url);
            let cover_loading = cover_url.as_deref().is_some_and(|u| !covers.has_image(u));
            covers.render_cover(
                frame,
                cover_inner,
                cover_url.as_deref(),
                cover_loading,
                ctx.icons.song,
                ctx.spinner,
            );
            self.render_main_panel(frame, top_cols[2], ctx, covers);
        } else {
            self.render_main_panel(frame, rows[0], ctx, covers);
        }

        let bot_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Length(1),
                Constraint::Min(10),
            ])
            .split(rows[1]);

        if self.mode == PlayerPanelMode::Lyrics {
            self.render_track_details_panel(frame, bot_cols[0], ctx);
        } else {
            let lyr_sub = self.lyrics_subtitle(ctx);
            let lyr_block = panel_block(
                "Lyrics",
                lyr_sub.as_deref(),
                self.focus_pane == PlayerFocusPane::Lyrics,
            );
            let lyr_inner = lyr_block.inner(bot_cols[0]);
            frame.render_widget(lyr_block, bot_cols[0]);
            frame.render_widget(
                LyricsBox {
                    lyrics: ctx.lyrics,
                    active_idx: ctx.position.active_lyric_idx,
                    has_track: ctx.snapshot.track.is_some(),
                    loading: ctx.snapshot.loading_lyrics,
                    show_translation: ctx.snapshot.show_translation,
                    translating: ctx.snapshot.translating_lyrics,
                    blank_between: false,
                    pad_left: 0,
                    scroll_offset: self.lyrics_scroll,
                    spinner: ctx.spinner,
                },
                lyr_inner,
            );
        }

        let cava_block = panel_block("Cava", Some(ctx.cava.mode_label), false);
        let cava_inner = cava_block.inner(bot_cols[2]);
        frame.render_widget(cava_block, bot_cols[2]);
        frame.render_widget(
            CavaWidget {
                bars: &ctx.cava.bars,
                palette: ctx.settings.display.cava_palette,
                idle: !ctx.cava.live,
            },
            cava_inner,
        );
    }

    fn lyrics_subtitle(&self, ctx: &RenderCtx<'_>) -> Option<String> {
        let mut parts = Vec::new();
        if ctx.snapshot.loading_lyrics {
            parts.push(ctx.spinner.inline_text("Loading"));
        } else if ctx.snapshot.translating_lyrics {
            parts.push(ctx.spinner.inline_text("ID"));
        } else if ctx.snapshot.show_translation && ctx.lyrics.has_translations() {
            parts.push("ID".to_string());
        } else if !ctx.lyrics.source.is_empty() {
            parts.push(ctx.lyrics.source.clone());
        }
        if ctx.snapshot.lyrics_offset_secs.abs() >= 0.01 {
            parts.push(format!("{:+.2}s", ctx.snapshot.lyrics_offset_secs));
        }
        if self.lyrics_scroll != 0 {
            parts.push("manual · zz".to_string());
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" · "))
        }
    }

    fn render_main_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        ctx: &RenderCtx<'_>,
        covers: &mut CoverRenderer,
    ) {
        let focused = self.focus_pane == PlayerFocusPane::Main;
        let (title, subtitle) = match self.mode {
            PlayerPanelMode::Discover => {
                let sub = match self.discover_tab {
                    DiscoverTab::Mixes => {
                        if self.loading_discover {
                            ctx.spinner.inline_text("Mixes")
                        } else {
                            format!("{} songs · / search", self.feed.mixes.len())
                        }
                    }
                    DiscoverTab::NewReleases => {
                        if self.loading_discover {
                            ctx.spinner.inline_text("New Releases")
                        } else {
                            format!("{} releases · / search", self.feed.new_releases.len())
                        }
                    }
                    DiscoverTab::Featured => {
                        if self.loading_discover {
                            ctx.spinner.inline_text("Featured")
                        } else {
                            format!("{} picks · / search", self.featured_items.len())
                        }
                    }
                };
                ("Discover", Some(sub))
            }
            PlayerPanelMode::SearchInput => ("Search", Some("Enter search · Esc back".to_string())),
            PlayerPanelMode::SearchResults => (
                "Search",
                Some(if self.loading_search {
                    ctx.spinner.inline_text("Searching")
                } else {
                    format!("{} results", self.search_results.len())
                }),
            ),
            PlayerPanelMode::Lyrics => {
                let mut parts = vec!["Expanded".to_string()];
                if ctx.snapshot.show_translation {
                    parts.push("ID".to_string());
                }
                if ctx.snapshot.lyrics_offset_secs.abs() >= 0.01 {
                    parts.push(format!("{:+.2}s", ctx.snapshot.lyrics_offset_secs));
                }
                if self.lyrics_scroll != 0 {
                    parts.push("manual · zz".to_string());
                }
                parts.push("L close".to_string());
                ("Lyrics", Some(parts.join(" · ")))
            }
        };

        let block = panel_block(title, subtitle.as_deref(), false);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width < 8 || inner.height < 2 {
            return;
        }

        match self.mode {
            PlayerPanelMode::Lyrics => {
                frame.render_widget(
                    LyricsBox {
                        lyrics: ctx.lyrics,
                        active_idx: ctx.position.active_lyric_idx,
                        has_track: ctx.snapshot.track.is_some(),
                        loading: ctx.snapshot.loading_lyrics,
                        show_translation: ctx.snapshot.show_translation,
                        translating: ctx.snapshot.translating_lyrics,
                        blank_between: true,
                        pad_left: 1,
                        scroll_offset: self.lyrics_scroll,
                        spinner: ctx.spinner,
                    },
                    inner,
                );
            }
            PlayerPanelMode::Discover => {
                let parts = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(1)])
                    .split(inner);

                frame.render_widget(
                    SectionTabs {
                        tabs: DISCOVER_TABS,
                        active: self.discover_tab.index(),
                        right_hint: Some("Tab switch"),
                    },
                    parts[0],
                );

                let playing_vid = ctx.snapshot.track.as_ref().map(|t| t.video_id.as_str());
                match self.discover_tab {
                    DiscoverTab::Mixes => {
                        let rows: Vec<TableRow> = self
                            .feed
                            .mixes
                            .iter()
                            .map(|t| {
                                let mut r = TableRow::new(vec![
                                    t.title.clone(),
                                    t.artist_names(),
                                    t.plays.clone().unwrap_or_else(|| t.duration_text()),
                                ]);
                                r.is_playing = playing_vid == Some(t.video_id.as_str());
                                r.prefix_icon = Some(if t.is_video() {
                                    ctx.icons.video
                                } else {
                                    ctx.icons.song
                                });
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
                                title: "Info",
                                fixed_width: Some(7),
                                align: Align::Right,
                            },
                        ];
                        frame.render_widget(
                            TrackTable {
                                columns: &cols,
                                rows: &rows,
                                cursor: self.mixes_cursor,
                                scroll: Some(&self.mixes_scroll),
                                focused,
                                show_header: false,
                                loading: self.loading_discover,
                                loading_label: "Loading Mixes…",
                                empty_text: "No mixes yet — press / to search YouTube Music",
                                icons: ctx.icons,
                                spinner: ctx.spinner,
                            },
                            parts[1],
                        );
                    }
                    DiscoverTab::NewReleases => {
                        let rows: Vec<TableRow> = self
                            .feed
                            .new_releases
                            .iter()
                            .map(|item| item_to_table_row(item, playing_vid, ctx.icons))
                            .collect();
                        let cols = [
                            ColumnDef {
                                title: "Release",
                                fixed_width: None,
                                align: Align::Left,
                            },
                            ColumnDef {
                                title: "Subtitle",
                                fixed_width: Some(20),
                                align: Align::Left,
                            },
                            ColumnDef {
                                title: "Type",
                                fixed_width: Some(8),
                                align: Align::Right,
                            },
                        ];
                        frame.render_widget(
                            TrackTable {
                                columns: &cols,
                                rows: &rows,
                                cursor: self.new_rel_cursor,
                                scroll: Some(&self.new_rel_scroll),
                                focused,
                                show_header: false,
                                loading: self.loading_discover,
                                loading_label: "Loading New Releases…",
                                empty_text: "No new releases available — press / to search",
                                icons: ctx.icons,
                                spinner: ctx.spinner,
                            },
                            parts[1],
                        );
                    }
                    DiscoverTab::Featured => {
                        let items: Vec<MediaListItem> = self
                            .featured_items
                            .iter()
                            .map(|item| item_to_media_item(item, playing_vid, ctx.icons))
                            .collect();
                        render_media_list(
                            frame,
                            parts[1],
                            &items,
                            self.featured_cursor,
                            Some(&self.featured_scroll),
                            focused,
                            self.loading_discover,
                            "Loading Featured…",
                            "No featured picks yet — press gr to refresh",
                            ctx.icons,
                            ctx.spinner,
                            covers,
                        );
                    }
                }
            }
            PlayerPanelMode::SearchInput | PlayerPanelMode::SearchResults => {
                let parts = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(1),
                    ])
                    .split(inner);

                let typing = self.mode == PlayerPanelMode::SearchInput;
                let prompt_prefix = format!("{} ", ctx.icons.prompt);
                render_filter_bar(
                    frame,
                    parts[0],
                    &self.search_input,
                    typing,
                    &prompt_prefix,
                    "Press / to search songs, lyrics, artists, URL…",
                );

                let cat_idx = SearchCategory::ALL
                    .iter()
                    .position(|c| *c == self.search_category)
                    .unwrap_or(0);
                frame.render_widget(
                    SectionTabs {
                        tabs: SEARCH_TABS,
                        active: cat_idx,
                        right_hint: None,
                    },
                    parts[1],
                );

                if typing {
                    let rows: Vec<TableRow> = self
                        .suggestions
                        .iter()
                        .map(|s| {
                            TableRow::new(vec![
                                s.text.clone(),
                                if s.from_history {
                                    "History".to_string()
                                } else {
                                    s.kind_hint.clone()
                                },
                            ])
                        })
                        .collect();
                    let cols = [
                        ColumnDef {
                            title: "Suggestion",
                            fixed_width: None,
                            align: Align::Left,
                        },
                        ColumnDef {
                            title: "Source",
                            fixed_width: Some(10),
                            align: Align::Right,
                        },
                    ];
                    frame.render_widget(
                        TrackTable {
                            columns: &cols,
                            rows: &rows,
                            cursor: self.suggest_cursor.unwrap_or(0),
                            scroll: Some(&self.suggest_scroll),
                            focused: self.suggest_cursor.is_some(),
                            show_header: false,
                            loading: false,
                            loading_label: "",
                            empty_text:
                                "Type query, lyrics, or YouTube URL (prefixes: a: al: pl: c: v: s:)",
                            icons: ctx.icons,
                            spinner: ctx.spinner,
                        },
                        parts[2],
                    );
                } else {
                    let playing_vid = ctx.snapshot.track.as_ref().map(|t| t.video_id.as_str());
                    let items: Vec<MediaListItem> = self
                        .search_results
                        .iter()
                        .map(|item| item_to_media_item(item, playing_vid, ctx.icons))
                        .collect();
                    render_media_list(
                        frame,
                        parts[2],
                        &items,
                        self.results_cursor,
                        Some(&self.results_scroll),
                        focused,
                        self.loading_search,
                        "Searching YouTube Music…",
                        "No matching results found",
                        ctx.icons,
                        ctx.spinner,
                        covers,
                    );
                }
            }
        }
    }

    fn render_track_details_panel(&self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx<'_>) {
        let block = panel_block("Track Details", None, false);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width < 6 || inner.height == 0 {
            return;
        }
        let w = usize::from(inner.width);
        let lines = if let Some(ref tr) = ctx.snapshot.track {
            let mut out = vec![
                Line::from(Span::styled(
                    trunc_cells(&tr.title, w),
                    base_style().add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    trunc_cells(&tr.artist_names(), w),
                    accent_style(),
                )),
            ];
            if let Some(ref alb) = tr.album {
                out.push(Line::from(Span::styled(
                    format!("Album: {}", trunc_cells(alb, w.saturating_sub(7))),
                    dim_style(),
                )));
            }
            let mut meta = vec![tr.duration_text()];
            if !ctx.snapshot.audio_info.is_empty() {
                meta.push(ctx.snapshot.audio_info.clone());
            }
            if let Some(ref plays) = tr.plays {
                meta.push(format!("{plays} plays"));
            }
            out.push(Line::from(Span::styled(
                trunc_cells(&meta.join(" · "), w),
                dim_style(),
            )));
            out
        } else {
            vec![Line::from(Span::styled("Nothing playing", dim_style()))]
        };
        frame.render_widget(Paragraph::new(lines), inner);
    }
}
