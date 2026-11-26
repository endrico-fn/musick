use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use musick_core::{
    AppCommand, HomeFeed, SavedCollectionKind, SearchCategory, SearchItem, Suggestion, Track,
};

use crate::keymap::Action;
use crate::text::{Align, trunc_cells};
use crate::theme::{ACCENT, CURSOR_BG, DIM, TEXT, panel_block};
use crate::views::{RenderCtx, ViewEffect};
use crate::widgets::{
    CavaWidget, ColumnDef, CoverRenderer, LyricsBox, SectionTabs, TableRow, TrackTable,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HomePanelMode {
    #[default]
    Discover,
    SearchInput,
    SearchResults,
    Lyrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HomeDiscoverTab {
    #[default]
    Mixes,
    NewReleases,
}

impl HomeDiscoverTab {
    pub fn toggle(self) -> Self {
        match self {
            Self::Mixes => Self::NewReleases,
            Self::NewReleases => Self::Mixes,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Mixes => 0,
            Self::NewReleases => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HomeFocusPane {
    #[default]
    Main,
    Lyrics,
    Cava,
}

const DISCOVER_TABS: &[(&str, &str)] = &[("Mixes", "Mixes"), ("New Releases", "New Rel.")];

const SEARCH_TABS: &[(&str, &str)] = &[
    ("All", "All"),
    ("Songs", "Songs"),
    ("Videos", "Videos"),
    ("Albums", "Albums"),
    ("Artists", "Artists"),
    ("Playlists", "Lists"),
    ("Community", "Comm."),
];

pub struct HomeView {
    pub mode: HomePanelMode,
    pub pre_lyrics_mode: HomePanelMode,
    pub discover_tab: HomeDiscoverTab,
    pub focus_pane: HomeFocusPane,

    pub feed: HomeFeed,
    pub loading_home: bool,
    pub mixes_cursor: usize,
    pub new_rel_cursor: usize,

    pub search_query: String,
    pub search_category: SearchCategory,
    pub suggestions: Vec<Suggestion>,
    pub suggest_cursor: Option<usize>,
    pub search_results: Vec<SearchItem>,
    pub results_cursor: usize,
    pub loading_search: bool,

    pub lyrics_scroll: isize,
}

impl Default for HomeView {
    fn default() -> Self {
        Self::new()
    }
}

impl HomeView {
    pub fn new() -> Self {
        Self {
            mode: HomePanelMode::Discover,
            pre_lyrics_mode: HomePanelMode::Discover,
            discover_tab: HomeDiscoverTab::Mixes,
            focus_pane: HomeFocusPane::Main,
            feed: HomeFeed::default(),
            loading_home: true,
            mixes_cursor: 0,
            new_rel_cursor: 0,
            search_query: String::new(),
            search_category: SearchCategory::All,
            suggestions: Vec::new(),
            suggest_cursor: None,
            search_results: Vec::new(),
            results_cursor: 0,
            loading_search: false,
            lyrics_scroll: 0,
        }
    }

    pub fn is_insert_mode(&self) -> bool {
        self.mode == HomePanelMode::SearchInput
    }

    pub fn toggle_expanded_lyrics(&mut self) {
        if self.mode == HomePanelMode::Lyrics {
            self.mode = self.pre_lyrics_mode;
        } else {
            self.pre_lyrics_mode = if self.mode == HomePanelMode::SearchInput {
                HomePanelMode::Discover
            } else {
                self.mode
            };
            self.mode = HomePanelMode::Lyrics;
            self.lyrics_scroll = 0;
        }
        self.focus_pane = HomeFocusPane::Main;
    }

    pub fn set_home_feed(&mut self, feed: HomeFeed) {
        self.feed = feed;
        self.loading_home = false;
        self.mixes_cursor = self
            .mixes_cursor
            .min(self.feed.mixes.len().saturating_sub(1));
        self.new_rel_cursor = self
            .new_rel_cursor
            .min(self.feed.new_releases.len().saturating_sub(1));
    }

    pub fn set_search_results(&mut self, items: Vec<SearchItem>) {
        self.search_results = items;
        self.loading_search = false;
        self.results_cursor = 0;
        if self.mode == HomePanelMode::SearchInput || self.mode == HomePanelMode::SearchResults {
            self.mode = HomePanelMode::SearchResults;
        }
    }

    pub fn set_suggestions(&mut self, suggestions: Vec<Suggestion>) {
        self.suggestions = suggestions;
        self.suggest_cursor = None;
    }

    pub fn footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        match self.mode {
            HomePanelMode::Discover => &[
                ("/", "search"),
                ("Tab", "Mixes/New Rel"),
                ("Enter", "play/open"),
                ("N/a", "next/queue"),
                ("P/f/w", "list/like/save"),
                ("L", "lyrics"),
                ("t", "ID trans"),
                ("?", "help"),
            ],
            HomePanelMode::SearchInput => &[
                ("↑↓", "pick suggest"),
                ("Tab", "complete"),
                ("Enter", "search"),
                ("Esc", "cancel"),
            ],
            HomePanelMode::SearchResults => &[
                ("/", "search"),
                ("Tab", "category"),
                ("Enter", "play/open"),
                ("N/a", "next/queue"),
                ("P/f/w", "list/like/save"),
                ("r", "radio"),
                ("Esc", "mixes"),
            ],
            HomePanelMode::Lyrics => &[
                ("L/Esc", "close lyrics"),
                ("t", "ID trans"),
                ("j k", "scroll"),
                ("Space", "play/pause"),
                ("/", "search"),
            ],
        }
    }

    pub fn handle_action(&mut self, action: Action) -> ViewEffect {
        // 1. Search Input (Insert Mode)
        if self.mode == HomePanelMode::SearchInput {
            return self.handle_search_input_action(action);
        }

        // 2. Pane focus switching via Ctrl+w
        match action {
            Action::FocusPaneDown => {
                if self.focus_pane == HomeFocusPane::Main {
                    self.focus_pane = HomeFocusPane::Lyrics;
                }
                return ViewEffect::None;
            }
            Action::FocusPaneUp => {
                self.focus_pane = HomeFocusPane::Main;
                return ViewEffect::None;
            }
            Action::FocusPaneLeft => {
                if self.focus_pane == HomeFocusPane::Cava {
                    self.focus_pane = HomeFocusPane::Lyrics;
                }
                return ViewEffect::None;
            }
            Action::FocusPaneRight => {
                if self.focus_pane == HomeFocusPane::Lyrics {
                    self.focus_pane = HomeFocusPane::Cava;
                }
                return ViewEffect::None;
            }
            Action::StartSearch => {
                self.mode = HomePanelMode::SearchInput;
                self.focus_pane = HomeFocusPane::Main;
                self.suggest_cursor = None;
                return ViewEffect::TriggerAutocomplete {
                    query: self.search_query.clone(),
                };
            }
            Action::Escape => {
                if self.mode == HomePanelMode::Lyrics {
                    self.mode = self.pre_lyrics_mode;
                } else if self.mode == HomePanelMode::SearchResults {
                    self.mode = HomePanelMode::Discover;
                }
                self.focus_pane = HomeFocusPane::Main;
                return ViewEffect::None;
            }
            _ => {}
        }

        // 3. Expanded Lyrics scrolling or focused Lyrics pane scrolling
        if self.mode == HomePanelMode::Lyrics || self.focus_pane == HomeFocusPane::Lyrics {
            match action {
                Action::MoveDown => {
                    self.lyrics_scroll += 1;
                    return ViewEffect::None;
                }
                Action::MoveUp => {
                    self.lyrics_scroll -= 1;
                    return ViewEffect::None;
                }
                Action::HalfPageDown => {
                    self.lyrics_scroll += 5;
                    return ViewEffect::None;
                }
                Action::HalfPageUp => {
                    self.lyrics_scroll -= 5;
                    return ViewEffect::None;
                }
                Action::JumpTop => {
                    self.lyrics_scroll = -1000;
                    return ViewEffect::None;
                }
                Action::JumpBottom => {
                    self.lyrics_scroll = 1000;
                    return ViewEffect::None;
                }
                Action::CycleCoverSize => {
                    // `c` recenters lyrics
                    self.lyrics_scroll = 0;
                    return ViewEffect::None;
                }
                _ => {}
            }
        }

        match self.mode {
            HomePanelMode::Discover => self.handle_discover_action(action),
            HomePanelMode::SearchResults => self.handle_results_action(action),
            HomePanelMode::Lyrics | HomePanelMode::SearchInput => ViewEffect::None,
        }
    }

    fn handle_search_input_action(&mut self, action: Action) -> ViewEffect {
        match action {
            Action::InputChar(ch) => {
                self.search_query.push(ch);
                self.suggest_cursor = None;
                ViewEffect::TriggerAutocomplete {
                    query: self.search_query.clone(),
                }
            }
            Action::InputBackspace => {
                self.search_query.pop();
                self.suggest_cursor = None;
                ViewEffect::TriggerAutocomplete {
                    query: self.search_query.clone(),
                }
            }
            Action::InputDown => {
                if !self.suggestions.is_empty() {
                    self.suggest_cursor = Some(match self.suggest_cursor {
                        None => 0,
                        Some(i) => (i + 1).min(self.suggestions.len() - 1),
                    });
                }
                ViewEffect::None
            }
            Action::InputUp => {
                if let Some(i) = self.suggest_cursor {
                    self.suggest_cursor = if i == 0 { None } else { Some(i - 1) };
                }
                ViewEffect::None
            }
            Action::InputTab => {
                if let Some(idx) = self.suggest_cursor
                    && let Some(sug) = self.suggestions.get(idx)
                {
                    self.search_query = sug.text.clone();
                    self.suggest_cursor = None;
                    return ViewEffect::TriggerAutocomplete {
                        query: self.search_query.clone(),
                    };
                }
                ViewEffect::None
            }
            Action::InputSubmit => {
                if let Some(idx) = self.suggest_cursor
                    && let Some(sug) = self.suggestions.get(idx)
                {
                    self.search_query = sug.text.clone();
                }
                let q = self.search_query.trim().to_string();
                if q.is_empty() {
                    self.mode = HomePanelMode::Discover;
                    return ViewEffect::None;
                }
                self.mode = HomePanelMode::SearchResults;
                self.loading_search = true;
                self.search_results.clear();
                self.results_cursor = 0;
                ViewEffect::TriggerSearch {
                    category: self.search_category,
                    query: q,
                }
            }
            Action::InputCancel => {
                self.mode = if self.search_results.is_empty() {
                    HomePanelMode::Discover
                } else {
                    HomePanelMode::SearchResults
                };
                ViewEffect::None
            }
            _ => ViewEffect::None,
        }
    }

    fn handle_discover_action(&mut self, action: Action) -> ViewEffect {
        match action {
            Action::NextSubTab | Action::PrevSubTab => {
                self.discover_tab = self.discover_tab.toggle();
                ViewEffect::None
            }
            Action::MoveDown => {
                self.move_discover_cursor(1);
                ViewEffect::None
            }
            Action::MoveUp => {
                self.move_discover_cursor(-1);
                ViewEffect::None
            }
            Action::HalfPageDown => {
                self.move_discover_cursor(8);
                ViewEffect::None
            }
            Action::HalfPageUp => {
                self.move_discover_cursor(-8);
                ViewEffect::None
            }
            Action::JumpTop => {
                match self.discover_tab {
                    HomeDiscoverTab::Mixes => self.mixes_cursor = 0,
                    HomeDiscoverTab::NewReleases => self.new_rel_cursor = 0,
                }
                ViewEffect::None
            }
            Action::JumpBottom => {
                match self.discover_tab {
                    HomeDiscoverTab::Mixes => {
                        self.mixes_cursor = self.feed.mixes.len().saturating_sub(1);
                    }
                    HomeDiscoverTab::NewReleases => {
                        self.new_rel_cursor = self.feed.new_releases.len().saturating_sub(1);
                    }
                }
                ViewEffect::None
            }
            Action::Activate => match self.discover_tab {
                HomeDiscoverTab::Mixes => {
                    if self.feed.mixes.is_empty() {
                        return ViewEffect::None;
                    }
                    let idx = self.mixes_cursor.min(self.feed.mixes.len() - 1);
                    ViewEffect::Command(AppCommand::PlayTracks {
                        tracks: self.feed.mixes.clone(),
                        start_index: idx,
                        autoplay: true,
                    })
                }
                HomeDiscoverTab::NewReleases => {
                    if let Some(item) = self.feed.new_releases.get(self.new_rel_cursor) {
                        activate_search_item(item, &self.feed.new_releases, self.new_rel_cursor)
                    } else {
                        ViewEffect::None
                    }
                }
            },
            Action::PlayDirect => match self.discover_tab {
                HomeDiscoverTab::Mixes => {
                    if let Some(tr) = self.feed.mixes.get(self.mixes_cursor) {
                        ViewEffect::Command(AppCommand::PlayTracks {
                            tracks: vec![tr.clone()],
                            start_index: 0,
                            autoplay: true,
                        })
                    } else {
                        ViewEffect::None
                    }
                }
                HomeDiscoverTab::NewReleases => {
                    if let Some(item) = self.feed.new_releases.get(self.new_rel_cursor) {
                        play_direct_item(item)
                    } else {
                        ViewEffect::None
                    }
                }
            },
            Action::PlayNext => {
                if let Some(tr) = self.selected_discover_track() {
                    ViewEffect::Command(AppCommand::PlayNext { tracks: vec![tr] })
                } else {
                    ViewEffect::None
                }
            }
            Action::AppendQueue => {
                if let Some(tr) = self.selected_discover_track() {
                    ViewEffect::Command(AppCommand::AppendTracks { tracks: vec![tr] })
                } else {
                    ViewEffect::None
                }
            }
            Action::AddToPlaylist => {
                if let Some(tr) = self.selected_discover_track() {
                    ViewEffect::AddToPlaylistPicker(tr)
                } else {
                    ViewEffect::None
                }
            }
            Action::ToggleLike => {
                if let Some(tr) = self.selected_discover_track() {
                    ViewEffect::ToggleLike(tr)
                } else {
                    ViewEffect::None
                }
            }
            Action::ToggleSave => {
                if self.discover_tab == HomeDiscoverTab::NewReleases
                    && let Some(item) = self.feed.new_releases.get(self.new_rel_cursor)
                {
                    return save_effect_for_item(item);
                }
                ViewEffect::None
            }
            Action::StartRadio => {
                if let Some(tr) = self.selected_discover_track() {
                    ViewEffect::Command(AppCommand::StartRadio {
                        video_id: Some(tr.video_id.clone()),
                        playlist_id: None,
                        seed_track: Some(tr),
                    })
                } else {
                    ViewEffect::None
                }
            }
            Action::CopyUrl => match self.discover_tab {
                HomeDiscoverTab::Mixes => self
                    .feed
                    .mixes
                    .get(self.mixes_cursor)
                    .map(|t| ViewEffect::CopyUrl(t.url()))
                    .unwrap_or(ViewEffect::None),
                HomeDiscoverTab::NewReleases => self
                    .feed
                    .new_releases
                    .get(self.new_rel_cursor)
                    .map(|it| ViewEffect::CopyUrl(item_url(it)))
                    .unwrap_or(ViewEffect::None),
            },
            _ => ViewEffect::None,
        }
    }

    fn handle_results_action(&mut self, action: Action) -> ViewEffect {
        match action {
            Action::NextSubTab => {
                let cats = SearchCategory::ALL;
                let idx = cats
                    .iter()
                    .position(|c| *c == self.search_category)
                    .unwrap_or(0);
                self.search_category = cats[(idx + 1) % cats.len()];
                self.loading_search = true;
                self.search_results.clear();
                self.results_cursor = 0;
                ViewEffect::TriggerSearch {
                    category: self.search_category,
                    query: self.search_query.clone(),
                }
            }
            Action::PrevSubTab => {
                let cats = SearchCategory::ALL;
                let idx = cats
                    .iter()
                    .position(|c| *c == self.search_category)
                    .unwrap_or(0);
                self.search_category = cats[(idx + cats.len() - 1) % cats.len()];
                self.loading_search = true;
                self.search_results.clear();
                self.results_cursor = 0;
                ViewEffect::TriggerSearch {
                    category: self.search_category,
                    query: self.search_query.clone(),
                }
            }
            Action::MoveDown => {
                if !self.search_results.is_empty() {
                    self.results_cursor =
                        (self.results_cursor + 1).min(self.search_results.len() - 1);
                }
                ViewEffect::None
            }
            Action::MoveUp => {
                self.results_cursor = self.results_cursor.saturating_sub(1);
                ViewEffect::None
            }
            Action::HalfPageDown => {
                if !self.search_results.is_empty() {
                    self.results_cursor =
                        (self.results_cursor + 8).min(self.search_results.len() - 1);
                }
                ViewEffect::None
            }
            Action::HalfPageUp => {
                self.results_cursor = self.results_cursor.saturating_sub(8);
                ViewEffect::None
            }
            Action::JumpTop => {
                self.results_cursor = 0;
                ViewEffect::None
            }
            Action::JumpBottom => {
                self.results_cursor = self.search_results.len().saturating_sub(1);
                ViewEffect::None
            }
            Action::Activate => {
                if let Some(item) = self.search_results.get(self.results_cursor) {
                    activate_search_item(item, &self.search_results, self.results_cursor)
                } else {
                    ViewEffect::None
                }
            }
            Action::PlayDirect => {
                if let Some(item) = self.search_results.get(self.results_cursor) {
                    play_direct_item(item)
                } else {
                    ViewEffect::None
                }
            }
            Action::PlayNext => {
                if let Some(SearchItem::Track(tr)) = self.search_results.get(self.results_cursor) {
                    ViewEffect::Command(AppCommand::PlayNext {
                        tracks: vec![tr.clone()],
                    })
                } else {
                    ViewEffect::None
                }
            }
            Action::AppendQueue => {
                if let Some(SearchItem::Track(tr)) = self.search_results.get(self.results_cursor) {
                    ViewEffect::Command(AppCommand::AppendTracks {
                        tracks: vec![tr.clone()],
                    })
                } else {
                    ViewEffect::None
                }
            }
            Action::AddToPlaylist => {
                if let Some(SearchItem::Track(tr)) = self.search_results.get(self.results_cursor) {
                    ViewEffect::AddToPlaylistPicker(tr.clone())
                } else {
                    ViewEffect::None
                }
            }
            Action::ToggleLike => {
                if let Some(SearchItem::Track(tr)) = self.search_results.get(self.results_cursor) {
                    ViewEffect::ToggleLike(tr.clone())
                } else {
                    ViewEffect::None
                }
            }
            Action::ToggleSave => {
                if let Some(item) = self.search_results.get(self.results_cursor) {
                    save_effect_for_item(item)
                } else {
                    ViewEffect::None
                }
            }
            Action::StartRadio => {
                if let Some(SearchItem::Track(tr)) = self.search_results.get(self.results_cursor) {
                    ViewEffect::Command(AppCommand::StartRadio {
                        video_id: Some(tr.video_id.clone()),
                        playlist_id: None,
                        seed_track: Some(tr.clone()),
                    })
                } else {
                    ViewEffect::None
                }
            }
            Action::CopyUrl => self
                .search_results
                .get(self.results_cursor)
                .map(|it| ViewEffect::CopyUrl(item_url(it)))
                .unwrap_or(ViewEffect::None),
            _ => ViewEffect::None,
        }
    }

    fn move_discover_cursor(&mut self, delta: isize) {
        let (cur, len) = match self.discover_tab {
            HomeDiscoverTab::Mixes => (&mut self.mixes_cursor, self.feed.mixes.len()),
            HomeDiscoverTab::NewReleases => {
                (&mut self.new_rel_cursor, self.feed.new_releases.len())
            }
        };
        if len == 0 {
            *cur = 0;
            return;
        }
        let next = (*cur as isize + delta).clamp(0, (len - 1) as isize) as usize;
        *cur = next;
    }

    fn selected_discover_track(&self) -> Option<Track> {
        match self.discover_tab {
            HomeDiscoverTab::Mixes => self.feed.mixes.get(self.mixes_cursor).cloned(),
            HomeDiscoverTab::NewReleases => match self.feed.new_releases.get(self.new_rel_cursor) {
                Some(SearchItem::Track(t)) => Some(t.clone()),
                _ => None,
            },
        }
    }

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

        // 2×2 Home Layout (Top ~61%, Bottom ~39% min 5 rows)
        let top_h = ((u32::from(area.height) * 61) / 100)
            .max(6)
            .min(u32::from(area.height.saturating_sub(5))) as u16;
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(top_h), Constraint::Min(5)])
            .split(area);

        // Top Row: Cover (left) + Main Discover/Search/Lyrics Panel (right) with 1-cell gap
        let cover_w = if area.width >= 100 {
            30
        } else if area.width >= 80 {
            24
        } else {
            18
        };
        let top_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(cover_w),
                Constraint::Length(1),
                Constraint::Min(20),
            ])
            .split(rows[0]);

        // Top-Left: Now Playing Cover
        let cover_block = panel_block("Now Playing", None, false);
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

        // Top-Right: Discover (Mixes | New Releases) / Search / Expanded Lyrics
        self.render_main_panel(frame, top_cols[2], ctx);

        // Bottom Row: Lyrics (or Track Details when expanded) + Cava Spectrum
        let bot_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Length(1),
                Constraint::Min(10),
            ])
            .split(rows[1]);

        if self.mode == HomePanelMode::Lyrics {
            self.render_track_details_panel(frame, bot_cols[0], ctx);
        } else {
            let lyr_sub = if ctx.snapshot.translating_lyrics {
                Some(ctx.spinner.inline_text("ID"))
            } else if ctx.snapshot.show_translation && ctx.lyrics.has_translations() {
                Some("ID".to_string())
            } else if !ctx.lyrics.source.is_empty() {
                Some(ctx.lyrics.source.clone())
            } else {
                None
            };
            let lyr_block = panel_block(
                "Lyrics",
                lyr_sub.as_deref(),
                self.focus_pane == HomeFocusPane::Lyrics,
            );
            let lyr_inner = lyr_block.inner(bot_cols[0]);
            frame.render_widget(lyr_block, bot_cols[0]);
            frame.render_widget(
                LyricsBox {
                    lyrics: ctx.lyrics,
                    active_idx: ctx.position.active_lyric_idx,
                    has_track: ctx.snapshot.track.is_some(),
                    show_translation: ctx.snapshot.show_translation,
                    translating: ctx.snapshot.translating_lyrics,
                    blank_between: false,
                    pad_left: 1,
                    scroll_offset: self.lyrics_scroll,
                    spinner: ctx.spinner,
                },
                lyr_inner,
            );
        }

        // Bottom-Right: Cava Visualizer
        let cava_block = panel_block(
            "Cava",
            Some(ctx.cava.mode_label),
            self.focus_pane == HomeFocusPane::Cava,
        );
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

    fn render_main_panel(&self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx<'_>) {
        let focused = self.focus_pane == HomeFocusPane::Main;
        let (title, subtitle) = match self.mode {
            HomePanelMode::Discover => {
                let sub = match self.discover_tab {
                    HomeDiscoverTab::Mixes => {
                        if self.loading_home {
                            ctx.spinner.inline_text("Mixes")
                        } else {
                            format!("{} songs · / search", self.feed.mixes.len())
                        }
                    }
                    HomeDiscoverTab::NewReleases => {
                        if self.loading_home {
                            ctx.spinner.inline_text("New Releases")
                        } else {
                            format!("{} releases · / search", self.feed.new_releases.len())
                        }
                    }
                };
                ("Discover", Some(sub))
            }
            HomePanelMode::SearchInput => ("Search", Some("Enter search · Esc back".to_string())),
            HomePanelMode::SearchResults => (
                "Search",
                Some(if self.loading_search {
                    ctx.spinner.inline_text("Searching")
                } else {
                    format!("{} results", self.search_results.len())
                }),
            ),
            HomePanelMode::Lyrics => (
                "Lyrics",
                Some(if ctx.snapshot.show_translation {
                    "Expanded · ID · L close".to_string()
                } else {
                    "Expanded · L close · t ID".to_string()
                }),
            ),
        };

        let block = panel_block(title, subtitle.as_deref(), focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width < 8 || inner.height < 2 {
            return;
        }

        match self.mode {
            HomePanelMode::Lyrics => {
                frame.render_widget(
                    LyricsBox {
                        lyrics: ctx.lyrics,
                        active_idx: ctx.position.active_lyric_idx,
                        has_track: ctx.snapshot.track.is_some(),
                        show_translation: ctx.snapshot.show_translation,
                        translating: ctx.snapshot.translating_lyrics,
                        blank_between: true,
                        pad_left: 2,
                        scroll_offset: self.lyrics_scroll,
                        spinner: ctx.spinner,
                    },
                    inner,
                );
            }
            HomePanelMode::Discover => {
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
                    HomeDiscoverTab::Mixes => {
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
                                focused,
                                show_header: false,
                                loading: self.loading_home,
                                loading_label: "Loading Mixes…",
                                empty_text: "No mixes yet — press / to search YouTube Music",
                                icons: ctx.icons,
                                spinner: ctx.spinner,
                            },
                            parts[1],
                        );
                    }
                    HomeDiscoverTab::NewReleases => {
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
                                focused,
                                show_header: false,
                                loading: self.loading_home,
                                loading_label: "Loading New Releases…",
                                empty_text: "No new releases available — press / to search",
                                icons: ctx.icons,
                                spinner: ctx.spinner,
                            },
                            parts[1],
                        );
                    }
                }
            }
            HomePanelMode::SearchInput | HomePanelMode::SearchResults => {
                let parts = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(1),
                    ])
                    .split(inner);

                let typing = self.mode == HomePanelMode::SearchInput;
                let prompt_line = Line::from(vec![
                    Span::styled(
                        format!("{} ", ctx.icons.prompt),
                        Style::default().fg(ACCENT).bg(Color::Reset),
                    ),
                    Span::styled(
                        if self.search_query.is_empty() && !typing {
                            "Press / to search songs, lyrics, artists, URL…".to_string()
                        } else if typing {
                            format!("{}█", self.search_query)
                        } else {
                            self.search_query.clone()
                        },
                        Style::default()
                            .fg(if self.search_query.is_empty() && !typing {
                                DIM
                            } else {
                                TEXT
                            })
                            .bg(if typing { CURSOR_BG } else { Color::Reset }),
                    ),
                ]);
                frame.render_widget(Paragraph::new(prompt_line), parts[0]);

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
                            focused: self.suggest_cursor.is_some(),
                            show_header: false,
                            loading: false,
                            loading_label: "",
                            empty_text: "Type query, lyrics, or YouTube URL (prefixes: a: al: pl: c: v: s:)",
                            icons: ctx.icons,
                            spinner: ctx.spinner,
                        },
                        parts[2],
                    );
                } else {
                    let playing_vid = ctx.snapshot.track.as_ref().map(|t| t.video_id.as_str());
                    let rows: Vec<TableRow> = self
                        .search_results
                        .iter()
                        .map(|item| item_to_table_row(item, playing_vid, ctx.icons))
                        .collect();
                    let cols = [
                        ColumnDef {
                            title: "Title",
                            fixed_width: None,
                            align: Align::Left,
                        },
                        ColumnDef {
                            title: "Artist / Meta",
                            fixed_width: Some(20),
                            align: Align::Left,
                        },
                        ColumnDef {
                            title: "Info",
                            fixed_width: Some(8),
                            align: Align::Right,
                        },
                    ];
                    frame.render_widget(
                        TrackTable {
                            columns: &cols,
                            rows: &rows,
                            cursor: self.results_cursor,
                            focused,
                            show_header: false,
                            loading: self.loading_search,
                            loading_label: "Searching YouTube Music…",
                            empty_text: "No matching results found",
                            icons: ctx.icons,
                            spinner: ctx.spinner,
                        },
                        parts[2],
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
        let w = usize::from(inner.width.saturating_sub(2));
        let lines = if let Some(ref tr) = ctx.snapshot.track {
            let mut out = vec![
                Line::from(Span::styled(
                    format!(" {}", trunc_cells(&tr.title, w)),
                    Style::default()
                        .fg(TEXT)
                        .bg(Color::Reset)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!(" {}", trunc_cells(&tr.artist_names(), w)),
                    Style::default().fg(ACCENT).bg(Color::Reset),
                )),
            ];
            if let Some(ref alb) = tr.album {
                out.push(Line::from(Span::styled(
                    format!(" Album: {}", trunc_cells(alb, w.saturating_sub(7))),
                    Style::default().fg(DIM).bg(Color::Reset),
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
                format!(" {}", meta.join(" · ")),
                Style::default().fg(DIM).bg(Color::Reset),
            )));
            out
        } else {
            vec![Line::from(Span::styled(
                " Nothing playing",
                Style::default().fg(DIM).bg(Color::Reset),
            ))]
        };
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

fn item_to_table_row(
    item: &SearchItem,
    playing_vid: Option<&str>,
    icons: crate::theme::IconSet,
) -> TableRow {
    match item {
        SearchItem::Track(t) => {
            let mut r = TableRow::new(vec![t.title.clone(), t.artist_names(), t.duration_text()]);
            r.is_playing = playing_vid == Some(t.video_id.as_str());
            r.prefix_icon = Some(icons.icon_for_item(item));
            r
        }
        SearchItem::Album(a) => {
            let art = a
                .artists
                .iter()
                .map(|x| x.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let mut r = TableRow::new(vec![
                a.title.clone(),
                if art.is_empty() { a.subtitle() } else { art },
                a.year.clone().unwrap_or_else(|| a.kind.clone()),
            ]);
            r.prefix_icon = Some(icons.album);
            r
        }
        SearchItem::Artist(a) => {
            let mut r = TableRow::new(vec![
                a.name.clone(),
                a.subtitle.clone().unwrap_or_default(),
                "Artist".to_string(),
            ]);
            r.prefix_icon = Some(icons.artist);
            r
        }
        SearchItem::Playlist(p) => {
            let mut r = TableRow::new(vec![
                p.title.clone(),
                p.author.clone().unwrap_or_default(),
                p.count.clone().unwrap_or_else(|| "List".to_string()),
            ]);
            r.prefix_icon = Some(icons.playlist);
            r
        }
    }
}

fn activate_search_item(item: &SearchItem, all_items: &[SearchItem], idx: usize) -> ViewEffect {
    match item {
        SearchItem::Track(tr) => {
            let tracks: Vec<Track> = all_items
                .iter()
                .filter_map(|it| match it {
                    SearchItem::Track(t) => Some(t.clone()),
                    _ => None,
                })
                .collect();
            let start = tracks
                .iter()
                .position(|t| t.video_id == tr.video_id)
                .unwrap_or(idx.min(tracks.len().saturating_sub(1)));
            ViewEffect::Command(AppCommand::PlayTracks {
                tracks,
                start_index: start,
                autoplay: true,
            })
        }
        SearchItem::Album(al) => ViewEffect::OpenAlbum {
            browse_id: al.browse_id.clone(),
            title_hint: al.title.clone(),
        },
        SearchItem::Artist(ar) => ViewEffect::OpenArtist {
            browse_id: ar.browse_id.clone(),
            name_hint: ar.name.clone(),
        },
        SearchItem::Playlist(pl) => ViewEffect::OpenPlaylist {
            browse_id: pl.browse_id.clone(),
            title_hint: pl.title.clone(),
        },
    }
}

fn play_direct_item(item: &SearchItem) -> ViewEffect {
    match item {
        SearchItem::Track(tr) => ViewEffect::Command(AppCommand::PlayTracks {
            tracks: vec![tr.clone()],
            start_index: 0,
            autoplay: true,
        }),
        SearchItem::Album(al) => ViewEffect::PlayAlbumDirect {
            browse_id: al.browse_id.clone(),
        },
        SearchItem::Artist(ar) => ViewEffect::PlayArtistDirect {
            browse_id: ar.browse_id.clone(),
        },
        SearchItem::Playlist(pl) => ViewEffect::PlayPlaylistDirect {
            browse_id: pl.browse_id.clone(),
        },
    }
}

fn save_effect_for_item(item: &SearchItem) -> ViewEffect {
    match item {
        SearchItem::Album(al) => ViewEffect::ToggleSave {
            kind: SavedCollectionKind::Album,
            browse_id: al.browse_id.clone(),
            title: al.title.clone(),
            subtitle: Some(al.subtitle()),
            thumb_url: al.cover_url(),
        },
        SearchItem::Artist(ar) => ViewEffect::ToggleSave {
            kind: SavedCollectionKind::Artist,
            browse_id: ar.browse_id.clone(),
            title: ar.name.clone(),
            subtitle: ar.subtitle.clone(),
            thumb_url: ar.cover_url(),
        },
        SearchItem::Playlist(pl) => ViewEffect::ToggleSave {
            kind: SavedCollectionKind::Playlist,
            browse_id: pl.browse_id.clone(),
            title: pl.title.clone(),
            subtitle: pl.author.clone(),
            thumb_url: pl.cover_url(),
        },
        SearchItem::Track(_) => ViewEffect::None,
    }
}

fn item_url(item: &SearchItem) -> String {
    match item {
        SearchItem::Track(t) => t.url(),
        SearchItem::Album(a) => format!("https://music.youtube.com/browse/{}", a.browse_id),
        SearchItem::Artist(a) => format!("https://music.youtube.com/channel/{}", a.browse_id),
        SearchItem::Playlist(p) => {
            format!("https://music.youtube.com/playlist?list={}", p.browse_id)
        }
    }
}
