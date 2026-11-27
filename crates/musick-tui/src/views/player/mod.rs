mod discover;
mod render;
mod search;

use std::collections::HashSet;

use musick_core::{DiscoverFeed, SearchCategory, SearchItem, Suggestion, Track, search_item_key};

use crate::keymap::Action;
use crate::views::ViewEffect;
use crate::widgets::{ScrollState, TextInput};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayerPanelMode {
    #[default]
    Discover,
    SearchInput,
    SearchResults,
    Lyrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscoverTab {
    #[default]
    Mixes,
    NewReleases,
    Featured,
}

impl DiscoverTab {
    pub const ALL: &'static [Self] = &[Self::Mixes, Self::NewReleases, Self::Featured];

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn index(self) -> usize {
        match self {
            Self::Mixes => 0,
            Self::NewReleases => 1,
            Self::Featured => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayerFocusPane {
    #[default]
    Main,
    Lyrics,
}

pub struct PlayerView {
    pub mode: PlayerPanelMode,
    pub pre_lyrics_mode: PlayerPanelMode,
    pub discover_tab: DiscoverTab,
    pub focus_pane: PlayerFocusPane,

    pub feed: DiscoverFeed,
    pub featured_items: Vec<SearchItem>,
    pub loading_discover: bool,
    pub mixes_cursor: usize,
    pub new_rel_cursor: usize,
    pub featured_cursor: usize,
    pub mixes_scroll: ScrollState,
    pub new_rel_scroll: ScrollState,
    pub featured_scroll: ScrollState,

    pub search_input: TextInput,
    pub search_category: SearchCategory,
    pub suggestions: Vec<Suggestion>,
    pub suggest_cursor: Option<usize>,
    pub suggest_scroll: ScrollState,
    pub search_results: Vec<SearchItem>,
    pub results_cursor: usize,
    pub results_scroll: ScrollState,
    pub loading_search: bool,
    pub search_error: Option<String>,

    pub lyrics_scroll: isize,
}

impl Default for PlayerView {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerView {
    pub fn new() -> Self {
        Self {
            mode: PlayerPanelMode::Discover,
            pre_lyrics_mode: PlayerPanelMode::Discover,
            discover_tab: DiscoverTab::Mixes,
            focus_pane: PlayerFocusPane::Main,
            feed: DiscoverFeed::default(),
            featured_items: Vec::new(),
            loading_discover: true,
            mixes_cursor: 0,
            new_rel_cursor: 0,
            featured_cursor: 0,
            mixes_scroll: ScrollState::new(),
            new_rel_scroll: ScrollState::new(),
            featured_scroll: ScrollState::new(),
            search_input: TextInput::new(),
            search_category: SearchCategory::All,
            suggestions: Vec::new(),
            suggest_cursor: None,
            suggest_scroll: ScrollState::new(),
            search_results: Vec::new(),
            results_cursor: 0,
            results_scroll: ScrollState::new(),
            loading_search: false,
            search_error: None,
            lyrics_scroll: 0,
        }
    }

    pub fn is_insert_mode(&self) -> bool {
        self.mode == PlayerPanelMode::SearchInput
    }

    pub fn toggle_expanded_lyrics(&mut self) {
        if self.mode == PlayerPanelMode::Lyrics {
            self.mode = self.pre_lyrics_mode;
        } else {
            self.pre_lyrics_mode = if self.mode == PlayerPanelMode::SearchInput {
                PlayerPanelMode::Discover
            } else {
                self.mode
            };
            self.mode = PlayerPanelMode::Lyrics;
            self.lyrics_scroll = 0;
        }
        self.focus_pane = PlayerFocusPane::Main;
    }

    pub fn set_discover_feed(&mut self, feed: DiscoverFeed) {
        let mut seen: HashSet<(&'static str, String)> = HashSet::new();
        let mut featured = Vec::new();
        for item in feed
            .featured
            .iter()
            .cloned()
            .chain(feed.playlists.iter().cloned().map(SearchItem::Playlist))
            .chain(feed.artists.iter().cloned().map(SearchItem::Artist))
        {
            let (k, id) = search_item_key(&item);
            if seen.insert((k, id.to_string())) {
                featured.push(item);
            }
        }
        self.featured_items = featured;
        self.feed = feed;
        self.loading_discover = false;
        self.mixes_cursor = self
            .mixes_cursor
            .min(self.feed.mixes.len().saturating_sub(1));
        self.new_rel_cursor = self
            .new_rel_cursor
            .min(self.feed.new_releases.len().saturating_sub(1));
        self.featured_cursor = self
            .featured_cursor
            .min(self.featured_items.len().saturating_sub(1));
    }

    pub fn set_search_results(&mut self, items: Vec<SearchItem>) {
        self.search_results = items;
        self.loading_search = false;
        self.search_error = None;
        self.results_cursor = 0;
        if self.mode == PlayerPanelMode::SearchInput || self.mode == PlayerPanelMode::SearchResults
        {
            self.mode = PlayerPanelMode::SearchResults;
        }
    }

    pub fn set_search_error(&mut self, msg: String) {
        self.loading_search = false;
        self.search_results.clear();
        self.search_error = Some(msg);
        if self.mode == PlayerPanelMode::SearchInput {
            self.mode = PlayerPanelMode::SearchResults;
        }
    }

    pub fn set_suggestions(&mut self, suggestions: Vec<Suggestion>) {
        self.suggestions = suggestions;
        self.suggest_cursor = None;
    }

    pub fn hovered_item(&self) -> Option<SearchItem> {
        match self.mode {
            PlayerPanelMode::Discover => match self.discover_tab {
                DiscoverTab::Mixes => self
                    .feed
                    .mixes
                    .get(self.mixes_cursor)
                    .cloned()
                    .map(SearchItem::Track),
                DiscoverTab::NewReleases => {
                    self.feed.new_releases.get(self.new_rel_cursor).cloned()
                }
                DiscoverTab::Featured => self.featured_items.get(self.featured_cursor).cloned(),
            },
            PlayerPanelMode::SearchResults => self.search_results.get(self.results_cursor).cloned(),
            _ => None,
        }
    }

    pub fn hovered_track(&self) -> Option<Track> {
        match self.hovered_item() {
            Some(SearchItem::Track(t)) => Some(t),
            _ => None,
        }
    }

    pub fn footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        match self.mode {
            PlayerPanelMode::Discover => match self.hovered_item() {
                Some(SearchItem::Artist(_)) => &[
                    ("Enter", "profile"),
                    ("a", "albums"),
                    ("x/r", "shuf/mix"),
                    ("W", "follow"),
                    ("e", "menu"),
                    ("/", "search"),
                    ("Q", "quit"),
                ],
                Some(SearchItem::Playlist(_)) => &[
                    ("Enter", "open"),
                    ("x/r", "shuf/mix"),
                    ("N/q", "next/queue"),
                    ("w/P", "save/copy"),
                    ("e", "menu"),
                    ("/", "search"),
                    ("Q", "quit"),
                ],
                Some(SearchItem::Album(_)) => &[
                    ("Enter", "open"),
                    ("A", "artist"),
                    ("x/r", "shuf/mix"),
                    ("N/q", "next/queue"),
                    ("w", "save"),
                    ("e", "menu"),
                    ("Q", "quit"),
                ],
                _ => &[
                    ("/", "search"),
                    ("Tab", "sub-tab"),
                    ("Enter", "play"),
                    ("N/q", "next/queue"),
                    ("A/a", "artist/album"),
                    ("e", "menu"),
                    ("L", "lyrics"),
                    ("?", "help"),
                ],
            },
            PlayerPanelMode::SearchInput => &[
                ("↑↓", "pick suggest"),
                ("Tab", "complete"),
                ("Enter", "search"),
                ("Esc", "cancel"),
            ],
            PlayerPanelMode::SearchResults => match self.hovered_item() {
                Some(SearchItem::Artist(_)) => &[
                    ("Enter", "profile"),
                    ("a", "albums"),
                    ("x/r", "shuf/mix"),
                    ("W", "follow"),
                    ("e", "menu"),
                    ("Tab", "cat"),
                    ("Esc", "discover"),
                ],
                Some(SearchItem::Playlist(_)) => &[
                    ("Enter", "open"),
                    ("x/r", "shuf/mix"),
                    ("N/q", "next/queue"),
                    ("w/P", "save/copy"),
                    ("e", "menu"),
                    ("Tab", "cat"),
                    ("Esc", "discover"),
                ],
                Some(SearchItem::Album(_)) => &[
                    ("Enter", "open"),
                    ("A", "artist"),
                    ("x/r", "shuf/mix"),
                    ("N/q", "next/queue"),
                    ("w", "save"),
                    ("e", "menu"),
                    ("Esc", "discover"),
                ],
                _ => &[
                    ("/", "search"),
                    ("Tab", "category"),
                    ("Enter", "play"),
                    ("N/q", "next/queue"),
                    ("A/a", "artist/album"),
                    ("e", "menu"),
                    ("r", "mix"),
                    ("Esc", "discover"),
                ],
            },
            PlayerPanelMode::Lyrics => &[
                ("L/Esc", "close"),
                ("t", "ID trans"),
                ("A/a", "artist/album"),
                ("j k", "scroll"),
                ("zz", "center"),
                (", .", "offset"),
                ("Space", "play/pause"),
            ],
        }
    }

    pub fn handle_action(&mut self, action: Action) -> ViewEffect {
        if self.mode == PlayerPanelMode::SearchInput {
            return self.handle_search_input_action(action);
        }

        match action {
            Action::FocusPaneDown => {
                if self.focus_pane == PlayerFocusPane::Main {
                    self.focus_pane = PlayerFocusPane::Lyrics;
                }
                return ViewEffect::None;
            }
            Action::FocusPaneUp => {
                self.focus_pane = PlayerFocusPane::Main;
                return ViewEffect::None;
            }
            Action::FocusPaneLeft | Action::FocusPaneRight => {
                return ViewEffect::None;
            }
            Action::StartSearch => {
                self.mode = PlayerPanelMode::SearchInput;
                self.focus_pane = PlayerFocusPane::Main;
                self.suggest_cursor = None;
                return ViewEffect::TriggerAutocomplete {
                    query: self.search_input.as_str().to_string(),
                };
            }
            Action::RefreshDiscover => {
                return ViewEffect::RefreshDiscover;
            }
            Action::Escape => {
                if self.mode == PlayerPanelMode::Lyrics {
                    self.mode = self.pre_lyrics_mode;
                } else if self.mode == PlayerPanelMode::SearchResults {
                    self.mode = PlayerPanelMode::Discover;
                }
                self.focus_pane = PlayerFocusPane::Main;
                return ViewEffect::None;
            }
            Action::CenterLyrics => {
                self.lyrics_scroll = 0;
                return ViewEffect::None;
            }
            _ => {}
        }

        if self.mode == PlayerPanelMode::Lyrics || self.focus_pane == PlayerFocusPane::Lyrics {
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
                _ => {}
            }
        }

        match self.mode {
            PlayerPanelMode::Discover => self.handle_discover_action(action),
            PlayerPanelMode::SearchResults => self.handle_results_action(action),
            PlayerPanelMode::Lyrics | PlayerPanelMode::SearchInput => ViewEffect::None,
        }
    }
}
