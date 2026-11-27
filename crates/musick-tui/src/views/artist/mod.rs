mod actions;
mod overview;
mod render;

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

use musick_core::{AlbumSummary, Artist, SavedCollectionKind, SearchItem, Track};

use crate::views::ViewEffect;
use crate::widgets::{ScrollState, TextInput};

pub(super) const SPLIT_BREAKPOINT: u16 = 106;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArtistTab {
    #[default]
    Overview,
    Songs,
    Albums,
    Singles,
    Videos,
    Playlists,
    Similar,
}

impl ArtistTab {
    pub const ALL: &'static [Self] = &[
        Self::Overview,
        Self::Songs,
        Self::Albums,
        Self::Singles,
        Self::Videos,
        Self::Playlists,
        Self::Similar,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArtistOverviewPane {
    #[default]
    TopSongs,
    Side,
}

pub struct ArtistView {
    pub browse_id: String,
    pub title_hint: String,
    pub artist: Option<Artist>,
    pub loading: bool,
    pub saved: bool,
    pub active_tab: ArtistTab,
    pub overview_pane: ArtistOverviewPane,
    pub(super) overview_split_active: Cell<bool>,
    pub cursor: usize,
    pub side_cursor: usize,
    pub scroll: ScrollState,
    pub sort_newest: bool,
    cached_album_order: Vec<usize>,
    cached_single_order: Vec<usize>,
    pub filter_active: bool,
    pub filter: TextInput,
}

impl ArtistView {
    pub fn new(browse_id: String, title_hint: String, saved: bool) -> Self {
        Self {
            browse_id,
            title_hint,
            artist: None,
            loading: true,
            saved,
            active_tab: ArtistTab::Overview,
            overview_pane: ArtistOverviewPane::TopSongs,
            overview_split_active: Cell::new(false),
            cursor: 0,
            side_cursor: 0,
            scroll: ScrollState::new(),
            sort_newest: true,
            cached_album_order: Vec::new(),
            cached_single_order: Vec::new(),
            filter_active: false,
            filter: TextInput::new(),
        }
    }

    pub fn with_tab(mut self, tab: ArtistTab) -> Self {
        self.active_tab = tab;
        self
    }

    pub fn is_insert_mode(&self) -> bool {
        self.filter_active
    }

    pub fn has_filter(&self) -> bool {
        !self.filter.is_empty()
    }

    pub fn set_artist(&mut self, artist: Artist) {
        self.title_hint = artist.summary.name.clone();
        self.artist = Some(artist);
        self.loading = false;
        self.cursor = 0;
        self.side_cursor = 0;
        self.overview_pane = ArtistOverviewPane::TopSongs;
        self.scroll.reset();
        self.rebuild_release_sort_cache();
    }

    pub(super) fn rebuild_release_sort_cache(&mut self) {
        let Some(ref art) = self.artist else {
            self.cached_album_order.clear();
            self.cached_single_order.clear();
            return;
        };
        self.cached_album_order = sorted_release_indices(&art.albums, self.sort_newest);
        self.cached_single_order = sorted_release_indices(&art.singles, self.sort_newest);
    }

    pub(super) fn filtered_tracks<'a>(
        &self,
        tracks: &'a [Track],
        cap: Option<usize>,
    ) -> Vec<&'a Track> {
        let q = self.filter.trim().to_lowercase();
        let slice = match cap {
            Some(n) => &tracks[..tracks.len().min(n)],
            None => tracks,
        };
        slice
            .iter()
            .filter(|t| {
                q.is_empty()
                    || t.title.to_lowercase().contains(&q)
                    || t.artist_names().to_lowercase().contains(&q)
                    || t.album
                        .as_deref()
                        .is_some_and(|a| a.to_lowercase().contains(&q))
            })
            .collect()
    }

    pub(super) fn filtered_releases(&self, singles: bool) -> Vec<&AlbumSummary> {
        let Some(ref art) = self.artist else {
            return Vec::new();
        };
        let (list, order) = if singles {
            (&art.singles, &self.cached_single_order)
        } else {
            (&art.albums, &self.cached_album_order)
        };
        let q = self.filter.trim().to_lowercase();
        let ordered: Vec<&AlbumSummary> = if order.len() == list.len() {
            order.iter().filter_map(|&i| list.get(i)).collect()
        } else {
            list.iter().collect()
        };
        if q.is_empty() {
            ordered
        } else {
            ordered
                .into_iter()
                .filter(|al| {
                    al.title.to_lowercase().contains(&q)
                        || al.year.as_deref().is_some_and(|y| y.contains(&q))
                        || al.kind.to_lowercase().contains(&q)
                })
                .collect()
        }
    }

    pub fn hovered_track(&self) -> Option<Track> {
        if self.active_tab == ArtistTab::Overview
            && self.overview_pane == ArtistOverviewPane::Side
            && self.overview_split_active.get()
        {
            return None;
        }
        self.selected_track()
    }

    pub fn hovered_item(&self) -> Option<SearchItem> {
        let art = self.artist.as_ref()?;
        if self.active_tab == ArtistTab::Overview
            && self.overview_pane == ArtistOverviewPane::Side
            && self.overview_split_active.get()
        {
            return match self.overview_side_items().get(self.side_cursor).cloned() {
                Some(overview::OverviewSideItem::Release(al)) => Some(SearchItem::Album(al)),
                Some(overview::OverviewSideItem::Similar(ar)) => Some(SearchItem::Artist(ar)),
                None => None,
            };
        }
        let q = self.filter.trim().to_lowercase();
        match self.active_tab {
            ArtistTab::Overview | ArtistTab::Songs | ArtistTab::Videos => {
                self.selected_track().map(SearchItem::Track)
            }
            ArtistTab::Albums => self
                .filtered_releases(false)
                .get(self.cursor)
                .map(|al| SearchItem::Album((*al).clone())),
            ArtistTab::Singles => self
                .filtered_releases(true)
                .get(self.cursor)
                .map(|al| SearchItem::Album((*al).clone())),
            ArtistTab::Playlists => art
                .playlists
                .iter()
                .filter(|p| {
                    q.is_empty()
                        || p.title.to_lowercase().contains(&q)
                        || p.author
                            .as_deref()
                            .is_some_and(|a| a.to_lowercase().contains(&q))
                })
                .nth(self.cursor)
                .cloned()
                .map(SearchItem::Playlist),
            ArtistTab::Similar => art
                .related
                .iter()
                .filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q))
                .nth(self.cursor)
                .cloned()
                .map(SearchItem::Artist),
        }
    }

    pub fn footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        if self.filter_active {
            &[("Enter", "apply filter"), ("Esc", "close filter")]
        } else if self.active_tab == ArtistTab::Overview && self.overview_split_active.get() {
            &[
                ("Tab", "sub-tab"),
                ("h/l", "pane"),
                ("Enter/p", "open/play"),
                ("N/q", "next/queue"),
                ("r/x", "radio/shuf"),
                ("e/i", "menu/bio"),
                ("Bksp", "back"),
            ]
        } else {
            &[
                ("Tab", "sub-tab"),
                ("/", "filter"),
                ("Enter/p", "open/play"),
                ("N/q", "next/queue"),
                ("o/r/x", "sort/radio/shuf"),
                ("e/i", "menu/bio"),
                ("Bksp", "back"),
            ]
        }
    }

    pub(super) fn current_len(&self) -> usize {
        let Some(ref art) = self.artist else {
            return 0;
        };
        let q = self.filter.trim().to_lowercase();
        match self.active_tab {
            ArtistTab::Overview => self.filtered_tracks(&art.songs, Some(10)).len(),
            ArtistTab::Songs => self.filtered_tracks(&art.songs, None).len(),
            ArtistTab::Albums => self.filtered_releases(false).len(),
            ArtistTab::Singles => self.filtered_releases(true).len(),
            ArtistTab::Videos => self.filtered_tracks(&art.videos, None).len(),
            ArtistTab::Playlists => art
                .playlists
                .iter()
                .filter(|p| {
                    q.is_empty()
                        || p.title.to_lowercase().contains(&q)
                        || p.author
                            .as_deref()
                            .is_some_and(|a| a.to_lowercase().contains(&q))
                })
                .count(),
            ArtistTab::Similar => art
                .related
                .iter()
                .filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q))
                .count(),
        }
    }

    pub(super) fn follow_profile_effect(&self) -> ViewEffect {
        let (title, subtitle, thumb) = match &self.artist {
            Some(a) => (
                a.summary.name.clone(),
                a.summary.subtitle.clone(),
                a.summary.cover_url(),
            ),
            None => (self.title_hint.clone(), None, None),
        };
        ViewEffect::ToggleSave {
            kind: SavedCollectionKind::Artist,
            browse_id: self.browse_id.clone(),
            title,
            subtitle,
            thumb_url: thumb,
        }
    }
}

pub(super) fn compare_release_years(
    a: &AlbumSummary,
    b: &AlbumSummary,
    newest: bool,
) -> std::cmp::Ordering {
    let ya = a.year.as_deref().filter(|s| !s.trim().is_empty());
    let yb = b.year.as_deref().filter(|s| !s.trim().is_empty());
    match (ya, yb) {
        (Some(a_y), Some(b_y)) => {
            let ord = if newest { b_y.cmp(a_y) } else { a_y.cmp(b_y) };
            ord.then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
    }
}

fn sorted_release_indices(items: &[AlbumSummary], newest: bool) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..items.len()).collect();
    indices.sort_by(|&ia, &ib| compare_release_years(&items[ia], &items[ib], newest));
    indices
}

pub(crate) fn shuffle_tracks_in_place(tracks: &mut [Track]) {
    if tracks.len() <= 1 {
        return;
    }
    let mut state = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9_7f4a_7c15);
    if state == 0 {
        state = 0x517c_c1b7_2722_0a95;
    }
    for i in (1..tracks.len()).rev() {
        // xorshift64*
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let r = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
        let j = (r as usize) % (i + 1);
        tracks.swap(i, j);
    }
}
