use musick_core::{AppCommand, SavedCollectionKind, Track};

use crate::keymap::Action;
use crate::views::ViewEffect;
use crate::widgets::ActiveModal;

use super::{ArtistOverviewPane, ArtistTab, ArtistView, shuffle_tracks_in_place};

impl ArtistView {
    pub fn handle_action(&mut self, action: Action) -> ViewEffect {
        if self.filter_active {
            let (handled, changed) = self.filter.handle_action(&action);
            if handled {
                if changed {
                    self.cursor = 0;
                    self.scroll.reset();
                }
                return ViewEffect::None;
            }
            if matches!(action, Action::InputSubmit | Action::InputCancel) {
                self.filter_active = false;
            }
            return ViewEffect::None;
        }

        match action {
            Action::StartSearch => {
                self.filter_active = true;
                self.overview_pane = ArtistOverviewPane::TopSongs;
                return ViewEffect::None;
            }
            Action::Escape => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.cursor = 0;
                    self.scroll.reset();
                }
                return ViewEffect::None;
            }
            Action::NextSubTab => {
                self.active_tab = self.active_tab.next();
                self.overview_pane = ArtistOverviewPane::TopSongs;
                self.cursor = 0;
                self.scroll.reset();
                return ViewEffect::None;
            }
            Action::PrevSubTab => {
                self.active_tab = self.active_tab.prev();
                self.overview_pane = ArtistOverviewPane::TopSongs;
                self.cursor = 0;
                self.scroll.reset();
                return ViewEffect::None;
            }
            Action::CycleSort => {
                self.sort_newest = !self.sort_newest;
                self.rebuild_release_sort_cache();
                self.cursor = 0;
                self.scroll.reset();
                return ViewEffect::None;
            }
            Action::ToggleArtistFollow => {
                return self.follow_profile_effect();
            }
            Action::ShowArtistBio => {
                if let Some(ref art) = self.artist
                    && let Some(ref desc) = art.description
                    && !desc.trim().is_empty()
                {
                    return ViewEffect::OpenModal(ActiveModal::TextViewer {
                        title: format!("{} — Bio", art.summary.name),
                        body: desc.trim().to_string(),
                        scroll: 0,
                    });
                }
                return ViewEffect::StatusToast("No biography available".into());
            }
            Action::ArtistShuffle => {
                if let Some(ref art) = self.artist {
                    if let Some(ref sid) = art.shuffle_id {
                        return ViewEffect::Command(AppCommand::StartRadio {
                            video_id: None,
                            playlist_id: Some(sid.clone()),
                            seed_track: None,
                        });
                    }
                    if !art.songs.is_empty() {
                        let mut shuffled = art.songs.clone();
                        shuffle_tracks_in_place(&mut shuffled);
                        return ViewEffect::Command(AppCommand::PlayTracks {
                            tracks: shuffled,
                            start_index: 0,
                            autoplay: true,
                        });
                    }
                }
                return ViewEffect::None;
            }
            Action::CopyCollectionUrl => {
                return ViewEffect::CopyUrl(format!(
                    "https://music.youtube.com/channel/{}",
                    self.browse_id
                ));
            }
            Action::StartRadio => {
                if let Some(ref art) = self.artist {
                    if let Some(ref rid) = art.radio_id {
                        return ViewEffect::Command(AppCommand::StartRadio {
                            video_id: None,
                            playlist_id: Some(rid.clone()),
                            seed_track: None,
                        });
                    }
                    if let Some(tr) = art.songs.first() {
                        return ViewEffect::Command(AppCommand::StartRadio {
                            video_id: Some(tr.video_id.clone()),
                            playlist_id: None,
                            seed_track: Some(tr.clone()),
                        });
                    }
                }
                return ViewEffect::None;
            }
            _ => {}
        }

        if self.active_tab == ArtistTab::Overview && self.overview_split_active.get() {
            match action {
                Action::FocusPaneRight | Action::MoveRight => {
                    if !self.overview_side_items().is_empty() {
                        self.overview_pane = ArtistOverviewPane::Side;
                        let side_len = self.overview_side_items().len();
                        self.side_cursor = self.side_cursor.min(side_len.saturating_sub(1));
                    }
                    return ViewEffect::None;
                }
                Action::FocusPaneLeft | Action::MoveLeft => {
                    self.overview_pane = ArtistOverviewPane::TopSongs;
                    return ViewEffect::None;
                }
                _ => {}
            }

            if self.overview_pane == ArtistOverviewPane::Side {
                return self.handle_overview_side_action(action);
            }
        }

        let len = self.current_len();
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
            Action::ToggleSave => self.toggle_save_hovered_or_profile(),
            Action::Activate => self.activate_item(),
            Action::PlayDirect => self.play_direct(),
            Action::PlayNext => self
                .selected_track()
                .map(|t| ViewEffect::Command(AppCommand::PlayNext { tracks: vec![t] }))
                .unwrap_or(ViewEffect::None),
            Action::AppendQueue => self
                .selected_track()
                .map(|t| ViewEffect::Command(AppCommand::AppendTracks { tracks: vec![t] }))
                .unwrap_or(ViewEffect::None),
            Action::AddToPlaylist => self
                .selected_track()
                .map(ViewEffect::AddToPlaylistPicker)
                .unwrap_or(ViewEffect::None),
            Action::ToggleLike => self
                .selected_track()
                .map(ViewEffect::ToggleLike)
                .unwrap_or(ViewEffect::None),
            Action::CopyUrl => ViewEffect::CopyUrl(self.selected_url()),
            _ => ViewEffect::None,
        }
    }

    fn toggle_save_hovered_or_profile(&self) -> ViewEffect {
        let Some(ref art) = self.artist else {
            return self.follow_profile_effect();
        };
        let q = self.filter.trim().to_lowercase();
        match self.active_tab {
            ArtistTab::Albums => self
                .filtered_releases(false)
                .get(self.cursor)
                .map(|al| ViewEffect::ToggleSave {
                    kind: SavedCollectionKind::Album,
                    browse_id: al.browse_id.clone(),
                    title: al.title.clone(),
                    subtitle: Some(al.subtitle()),
                    thumb_url: al.cover_url(),
                })
                .unwrap_or_else(|| self.follow_profile_effect()),
            ArtistTab::Singles => self
                .filtered_releases(true)
                .get(self.cursor)
                .map(|al| ViewEffect::ToggleSave {
                    kind: SavedCollectionKind::Album,
                    browse_id: al.browse_id.clone(),
                    title: al.title.clone(),
                    subtitle: Some(al.subtitle()),
                    thumb_url: al.cover_url(),
                })
                .unwrap_or_else(|| self.follow_profile_effect()),
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
                .map(|pl| ViewEffect::ToggleSave {
                    kind: SavedCollectionKind::Playlist,
                    browse_id: pl.browse_id.clone(),
                    title: pl.title.clone(),
                    subtitle: pl.author.clone(),
                    thumb_url: pl.cover_url(),
                })
                .unwrap_or_else(|| self.follow_profile_effect()),
            ArtistTab::Similar => art
                .related
                .iter()
                .filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q))
                .nth(self.cursor)
                .map(|rel| ViewEffect::ToggleSave {
                    kind: SavedCollectionKind::Artist,
                    browse_id: rel.browse_id.clone(),
                    title: rel.name.clone(),
                    subtitle: rel.subtitle.clone(),
                    thumb_url: rel.cover_url(),
                })
                .unwrap_or_else(|| self.follow_profile_effect()),
            ArtistTab::Overview | ArtistTab::Songs | ArtistTab::Videos => {
                self.follow_profile_effect()
            }
        }
    }

    fn selected_url(&self) -> String {
        if let Some(ref art) = self.artist {
            let q = self.filter.trim().to_lowercase();
            match self.active_tab {
                ArtistTab::Overview => {
                    if let Some(t) = self.filtered_tracks(&art.songs, Some(10)).get(self.cursor) {
                        return t.url();
                    }
                }
                ArtistTab::Songs => {
                    if let Some(t) = self.filtered_tracks(&art.songs, None).get(self.cursor) {
                        return t.url();
                    }
                }
                ArtistTab::Videos => {
                    if let Some(t) = self.filtered_tracks(&art.videos, None).get(self.cursor) {
                        return t.url();
                    }
                }
                ArtistTab::Albums => {
                    if let Some(al) = self.filtered_releases(false).get(self.cursor) {
                        return format!("https://music.youtube.com/browse/{}", al.browse_id);
                    }
                }
                ArtistTab::Singles => {
                    if let Some(al) = self.filtered_releases(true).get(self.cursor) {
                        return format!("https://music.youtube.com/browse/{}", al.browse_id);
                    }
                }
                ArtistTab::Playlists => {
                    if let Some(pl) = art
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
                    {
                        return format!("https://music.youtube.com/playlist?list={}", pl.browse_id);
                    }
                }
                ArtistTab::Similar => {
                    if let Some(rel) = art
                        .related
                        .iter()
                        .filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q))
                        .nth(self.cursor)
                    {
                        return format!("https://music.youtube.com/channel/{}", rel.browse_id);
                    }
                }
            }
        }
        format!("https://music.youtube.com/channel/{}", self.browse_id)
    }

    pub(super) fn selected_track(&self) -> Option<Track> {
        let art = self.artist.as_ref()?;
        match self.active_tab {
            ArtistTab::Overview => self
                .filtered_tracks(&art.songs, Some(10))
                .get(self.cursor)
                .map(|t| (*t).clone()),
            ArtistTab::Songs => self
                .filtered_tracks(&art.songs, None)
                .get(self.cursor)
                .map(|t| (*t).clone()),
            ArtistTab::Videos => self
                .filtered_tracks(&art.videos, None)
                .get(self.cursor)
                .map(|t| (*t).clone()),
            _ => None,
        }
    }

    fn activate_item(&self) -> ViewEffect {
        let Some(ref art) = self.artist else {
            return ViewEffect::None;
        };
        let q = self.filter.trim().to_lowercase();
        match self.active_tab {
            ArtistTab::Overview | ArtistTab::Songs => {
                let cap = if self.active_tab == ArtistTab::Overview {
                    Some(10)
                } else {
                    None
                };
                let tracks: Vec<Track> = self
                    .filtered_tracks(&art.songs, cap)
                    .into_iter()
                    .cloned()
                    .collect();
                if tracks.is_empty() {
                    return ViewEffect::None;
                }
                ViewEffect::Command(AppCommand::PlayTracks {
                    start_index: self.cursor.min(tracks.len() - 1),
                    tracks,
                    autoplay: true,
                })
            }
            ArtistTab::Videos => {
                let tracks: Vec<Track> = self
                    .filtered_tracks(&art.videos, None)
                    .into_iter()
                    .cloned()
                    .collect();
                if tracks.is_empty() {
                    return ViewEffect::None;
                }
                ViewEffect::Command(AppCommand::PlayTracks {
                    start_index: self.cursor.min(tracks.len() - 1),
                    tracks,
                    autoplay: true,
                })
            }
            ArtistTab::Albums => self
                .filtered_releases(false)
                .get(self.cursor)
                .map(|al| ViewEffect::OpenAlbum {
                    browse_id: al.browse_id.clone(),
                    title_hint: al.title.clone(),
                })
                .unwrap_or(ViewEffect::None),
            ArtistTab::Singles => self
                .filtered_releases(true)
                .get(self.cursor)
                .map(|al| ViewEffect::OpenAlbum {
                    browse_id: al.browse_id.clone(),
                    title_hint: al.title.clone(),
                })
                .unwrap_or(ViewEffect::None),
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
                .map(|pl| ViewEffect::OpenPlaylist {
                    browse_id: pl.browse_id.clone(),
                    title_hint: pl.title.clone(),
                })
                .unwrap_or(ViewEffect::None),
            ArtistTab::Similar => art
                .related
                .iter()
                .filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q))
                .nth(self.cursor)
                .map(|rel| ViewEffect::OpenArtist {
                    browse_id: rel.browse_id.clone(),
                    name_hint: rel.name.clone(),
                })
                .unwrap_or(ViewEffect::None),
        }
    }

    fn play_direct(&self) -> ViewEffect {
        let q = self.filter.trim().to_lowercase();
        match self.active_tab {
            ArtistTab::Albums => self
                .filtered_releases(false)
                .get(self.cursor)
                .map(|al| ViewEffect::PlayAlbumDirect {
                    browse_id: al.browse_id.clone(),
                })
                .unwrap_or(ViewEffect::None),
            ArtistTab::Singles => self
                .filtered_releases(true)
                .get(self.cursor)
                .map(|al| ViewEffect::PlayAlbumDirect {
                    browse_id: al.browse_id.clone(),
                })
                .unwrap_or(ViewEffect::None),
            ArtistTab::Playlists => self
                .artist
                .as_ref()
                .and_then(|a| {
                    a.playlists
                        .iter()
                        .filter(|p| {
                            q.is_empty()
                                || p.title.to_lowercase().contains(&q)
                                || p.author
                                    .as_deref()
                                    .is_some_and(|au| au.to_lowercase().contains(&q))
                        })
                        .nth(self.cursor)
                })
                .map(|pl| ViewEffect::PlayPlaylistDirect {
                    browse_id: pl.browse_id.clone(),
                })
                .unwrap_or(ViewEffect::None),
            ArtistTab::Similar => self
                .artist
                .as_ref()
                .and_then(|a| {
                    a.related
                        .iter()
                        .filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q))
                        .nth(self.cursor)
                })
                .map(|rel| ViewEffect::PlayArtistDirect {
                    browse_id: rel.browse_id.clone(),
                })
                .unwrap_or(ViewEffect::None),
            _ => self.activate_item(),
        }
    }
}
