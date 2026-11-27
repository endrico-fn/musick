use musick_core::{AppCommand, SavedCollectionKind, SearchItem, Track};

use crate::keymap::Action;
use crate::theme::IconSet;
use crate::views::ViewEffect;
use crate::widgets::{MediaListItem, TableRow};

use super::{DiscoverTab, PlayerView};

impl PlayerView {
    pub(super) fn handle_discover_action(&mut self, action: Action) -> ViewEffect {
        match action {
            Action::NextSubTab => {
                self.discover_tab = self.discover_tab.next();
                ViewEffect::None
            }
            Action::PrevSubTab => {
                self.discover_tab = self.discover_tab.prev();
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
                    DiscoverTab::Mixes => self.mixes_cursor = 0,
                    DiscoverTab::NewReleases => self.new_rel_cursor = 0,
                    DiscoverTab::Featured => self.featured_cursor = 0,
                }
                ViewEffect::None
            }
            Action::JumpBottom => {
                match self.discover_tab {
                    DiscoverTab::Mixes => {
                        self.mixes_cursor = self.feed.mixes.len().saturating_sub(1);
                    }
                    DiscoverTab::NewReleases => {
                        self.new_rel_cursor = self.feed.new_releases.len().saturating_sub(1);
                    }
                    DiscoverTab::Featured => {
                        self.featured_cursor = self.featured_items.len().saturating_sub(1);
                    }
                }
                ViewEffect::None
            }
            Action::Activate => match self.discover_tab {
                DiscoverTab::Mixes => {
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
                DiscoverTab::NewReleases => {
                    if let Some(item) = self.feed.new_releases.get(self.new_rel_cursor) {
                        activate_search_item(item, &self.feed.new_releases, self.new_rel_cursor)
                    } else {
                        ViewEffect::None
                    }
                }
                DiscoverTab::Featured => {
                    if let Some(item) = self.featured_items.get(self.featured_cursor) {
                        activate_search_item(item, &self.featured_items, self.featured_cursor)
                    } else {
                        ViewEffect::None
                    }
                }
            },
            Action::PlayDirect => match self.discover_tab {
                DiscoverTab::Mixes => {
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
                DiscoverTab::NewReleases => {
                    if let Some(item) = self.feed.new_releases.get(self.new_rel_cursor) {
                        play_direct_item(item)
                    } else {
                        ViewEffect::None
                    }
                }
                DiscoverTab::Featured => {
                    if let Some(item) = self.featured_items.get(self.featured_cursor) {
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
            Action::ToggleSave => match self.discover_tab {
                DiscoverTab::NewReleases => self
                    .feed
                    .new_releases
                    .get(self.new_rel_cursor)
                    .map(save_effect_for_item)
                    .unwrap_or(ViewEffect::None),
                DiscoverTab::Featured => self
                    .featured_items
                    .get(self.featured_cursor)
                    .map(save_effect_for_item)
                    .unwrap_or(ViewEffect::None),
                DiscoverTab::Mixes => ViewEffect::None,
            },
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
                DiscoverTab::Mixes => self
                    .feed
                    .mixes
                    .get(self.mixes_cursor)
                    .map(|t| ViewEffect::CopyUrl(t.url()))
                    .unwrap_or(ViewEffect::None),
                DiscoverTab::NewReleases => self
                    .feed
                    .new_releases
                    .get(self.new_rel_cursor)
                    .map(|it| ViewEffect::CopyUrl(item_url(it)))
                    .unwrap_or(ViewEffect::None),
                DiscoverTab::Featured => self
                    .featured_items
                    .get(self.featured_cursor)
                    .map(|it| ViewEffect::CopyUrl(item_url(it)))
                    .unwrap_or(ViewEffect::None),
            },
            _ => ViewEffect::None,
        }
    }

    fn move_discover_cursor(&mut self, delta: isize) {
        let (cur, len) = match self.discover_tab {
            DiscoverTab::Mixes => (&mut self.mixes_cursor, self.feed.mixes.len()),
            DiscoverTab::NewReleases => (&mut self.new_rel_cursor, self.feed.new_releases.len()),
            DiscoverTab::Featured => (&mut self.featured_cursor, self.featured_items.len()),
        };
        if len == 0 {
            *cur = 0;
            return;
        }
        let next = (*cur as isize + delta).clamp(0, (len - 1) as isize) as usize;
        *cur = next;
    }

    pub(super) fn selected_discover_track(&self) -> Option<Track> {
        match self.discover_tab {
            DiscoverTab::Mixes => self.feed.mixes.get(self.mixes_cursor).cloned(),
            DiscoverTab::NewReleases => match self.feed.new_releases.get(self.new_rel_cursor) {
                Some(SearchItem::Track(t)) => Some(t.clone()),
                _ => None,
            },
            DiscoverTab::Featured => match self.featured_items.get(self.featured_cursor) {
                Some(SearchItem::Track(t)) => Some(t.clone()),
                _ => None,
            },
        }
    }
}

pub(super) fn item_to_media_item(
    item: &SearchItem,
    playing_vid: Option<&str>,
    icons: IconSet,
) -> MediaListItem {
    match item {
        SearchItem::Track(t) => {
            let mut sub = t.artist_names();
            if let Some(ref alb) = t.album
                && !alb.is_empty()
                && !alb.eq_ignore_ascii_case(&t.title)
            {
                sub = format!("{sub} · {alb}");
            }
            let meta_top = t.plays.clone().unwrap_or_else(|| {
                if t.is_video() {
                    "Video".to_string()
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
                glyph: icons.icon_for_item(item),
                is_playing: playing_vid == Some(t.video_id.as_str()),
                is_dim: !t.is_available,
            }
        }
        SearchItem::Album(a) => {
            let art = a
                .artists
                .iter()
                .map(|x| x.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            MediaListItem {
                title: a.title.clone(),
                subtitle: if art.is_empty() { a.subtitle() } else { art },
                meta_top: a.kind.clone(),
                meta_bottom: a.year.clone().unwrap_or_default(),
                cover_url: a.cover_url(),
                glyph: icons.album,
                is_playing: false,
                is_dim: false,
            }
        }
        SearchItem::Artist(a) => MediaListItem {
            title: a.name.clone(),
            subtitle: a.subtitle.clone().unwrap_or_else(|| "Artist".to_string()),
            meta_top: "Artist".to_string(),
            meta_bottom: String::new(),
            cover_url: a.cover_url(),
            glyph: icons.artist,
            is_playing: false,
            is_dim: false,
        },
        SearchItem::Playlist(p) => MediaListItem {
            title: p.title.clone(),
            subtitle: p
                .author
                .clone()
                .unwrap_or_else(|| "Community playlist".to_string()),
            meta_top: "Playlist".to_string(),
            meta_bottom: p.count.clone().unwrap_or_default(),
            cover_url: p.cover_url(),
            glyph: icons.playlist,
            is_playing: false,
            is_dim: false,
        },
    }
}

pub(super) fn item_to_table_row(
    item: &SearchItem,
    playing_vid: Option<&str>,
    icons: IconSet,
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

pub(super) fn activate_search_item(
    item: &SearchItem,
    all_items: &[SearchItem],
    idx: usize,
) -> ViewEffect {
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

pub(super) fn play_direct_item(item: &SearchItem) -> ViewEffect {
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

pub(super) fn save_effect_for_item(item: &SearchItem) -> ViewEffect {
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

pub(super) fn item_url(item: &SearchItem) -> String {
    match item {
        SearchItem::Track(t) => t.url(),
        SearchItem::Album(a) => format!("https://music.youtube.com/browse/{}", a.browse_id),
        SearchItem::Artist(a) => format!("https://music.youtube.com/channel/{}", a.browse_id),
        SearchItem::Playlist(p) => {
            format!("https://music.youtube.com/playlist?list={}", p.browse_id)
        }
    }
}
