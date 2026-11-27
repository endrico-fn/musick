use std::time::{Duration, Instant};

use musick_audio::MprisService;
use musick_core::{SavedCollectionKind, Track};

use crate::views::{DiscoverTab, LibraryTab, PlayerPanelMode};
use crate::widgets::RootTab;

use super::{App, BgMsg, NavPage};

impl App {
    pub(super) fn sync_mpris_state(&mut self) {
        if self.settings.player.mpris_enabled {
            if self.mpris.is_none()
                && let Some(ref flag) = self.mpris_quit_flag
            {
                self.mpris = Some(MprisService::spawn_with_quit_flag(
                    self.services.engine.clone(),
                    flag.clone(),
                ));
            }
        } else if let Some(m) = self.mpris.take() {
            m.shutdown();
        }
    }

    pub(super) fn re_request_active_covers(&mut self) {
        if !self.covers.is_enabled() {
            return;
        }
        let mut urls = Vec::new();
        if let Some(url) = self.snapshot.track.as_ref().and_then(Track::cover_url) {
            urls.push(url);
        }
        for item in self
            .player
            .search_results
            .iter()
            .take(30)
            .chain(self.player.feed.new_releases.iter().take(24))
            .chain(self.player.featured_items.iter().take(24))
        {
            if let Some(u) = item.cover_url() {
                urls.push(u);
            }
        }
        for tr in self
            .player
            .feed
            .mixes
            .iter()
            .take(24)
            .chain(self.library.liked_songs.iter().take(24))
            .chain(self.library.recent_tracks.iter().take(24))
        {
            if let Some(u) = tr.cover_url() {
                urls.push(u);
            }
        }
        if let Some(top) = self.nav_stack.last() {
            match top {
                NavPage::Artist(v) => {
                    if let Some(ref art) = v.artist {
                        if let Some(u) = art.summary.cover_url() {
                            urls.push(u);
                        }
                        for tr in art.songs.iter().take(24) {
                            if let Some(u) = tr.cover_url() {
                                urls.push(u);
                            }
                        }
                        for tr in art.videos.iter().take(12) {
                            if let Some(u) = tr.cover_url() {
                                urls.push(u);
                            }
                        }
                        for al in art.albums.iter().chain(art.singles.iter()).take(32) {
                            if let Some(u) = al.cover_url() {
                                urls.push(u);
                            }
                        }
                        for rel in art.related.iter().take(16) {
                            if let Some(u) = rel.cover_url() {
                                urls.push(u);
                            }
                        }
                    }
                }
                NavPage::Album(v) => {
                    if let Some(u) = v.album.as_ref().and_then(|a| a.summary.cover_url()) {
                        urls.push(u);
                    }
                }
                NavPage::PlaylistDetail(v) => {
                    if let Some(u) = v.playlist.as_ref().and_then(|p| p.summary.cover_url()) {
                        urls.push(u);
                    }
                }
            }
        }
        for col in self
            .library
            .saved_albums
            .iter()
            .chain(self.library.saved_artists.iter())
            .chain(self.library.saved_playlists.iter())
            .take(48)
        {
            if let Some(ref u) = col.thumb_url {
                urls.push(u.clone());
            }
        }
        for url in urls {
            self.request_cover_image(url);
        }
    }

    pub(super) fn request_cover_image(&mut self, url: String) {
        if !self.covers.is_enabled()
            || self.covers.has_image(&url)
            || !self.requested_images.insert(url.clone())
        {
            return;
        }
        let images = self.services.images.clone();
        let tx = self.bg_tx.clone();
        tokio::spawn(async move {
            if let Some(img) = images.fetch(&url).await {
                let _ = tx.send(BgMsg::ImageLoaded { url, image: img });
            } else {
                let _ = tx.send(BgMsg::ImageFailed { url });
            }
        });
    }

    pub fn reload_local_collections(&mut self) {
        let acct = self.services.auth.account_key();
        let pls = self
            .services
            .store
            .list_playlists(&acct)
            .unwrap_or_default();
        self.playlists.set_playlists(pls.clone());
        self.library.user_playlists = pls;
        self.library.liked_songs = self
            .services
            .store
            .list_liked_songs(&acct)
            .unwrap_or_default();
        self.library.saved_albums = self
            .services
            .store
            .list_saved_collections(&acct, SavedCollectionKind::Album)
            .unwrap_or_default();
        self.library.saved_artists = self
            .services
            .store
            .list_saved_collections(&acct, SavedCollectionKind::Artist)
            .unwrap_or_default();
        self.library.saved_playlists = self
            .services
            .store
            .list_saved_collections(&acct, SavedCollectionKind::Playlist)
            .unwrap_or_default();
        self.reload_recent_plays();

        let mut cover_urls: Vec<String> = self
            .library
            .saved_albums
            .iter()
            .chain(self.library.saved_artists.iter())
            .chain(self.library.saved_playlists.iter())
            .take(48)
            .filter_map(|c| c.thumb_url.clone())
            .collect();
        cover_urls.extend(
            self.library
                .liked_songs
                .iter()
                .take(24)
                .chain(self.library.recent_tracks.iter().take(24))
                .filter_map(Track::cover_url),
        );
        for url in cover_urls {
            self.request_cover_image(url);
        }

        for page in &mut self.nav_stack {
            let (kind, bid, saved_slot) = match page {
                NavPage::Artist(v) => (
                    SavedCollectionKind::Artist,
                    v.browse_id.as_str(),
                    &mut v.saved,
                ),
                NavPage::Album(v) => (
                    SavedCollectionKind::Album,
                    v.browse_id.as_str(),
                    &mut v.saved,
                ),
                NavPage::PlaylistDetail(v) => (
                    SavedCollectionKind::Playlist,
                    v.browse_id.as_str(),
                    &mut v.saved,
                ),
            };
            if let Ok(is_saved) = self.services.store.is_collection_saved(&acct, kind, bid) {
                *saved_slot = is_saved;
            }
        }

        self.pending_sync = self.services.sync.pending_count();
        self.sync_selected_playlist_tracks(true);
    }

    pub(super) fn sync_selected_playlist_tracks(&mut self, force: bool) {
        let changed = self.playlists.selected_playlist().map(|p| p.id.as_str())
            != self.last_selected_playlist_id.as_deref();
        if changed || force {
            let cur_pl_id = self.playlists.selected_playlist().map(|p| p.id.clone());
            self.last_selected_playlist_id = cur_pl_id.clone();
            if changed {
                self.playlists.on_playlist_selection_changed();
            }
            let tracks = cur_pl_id
                .as_deref()
                .and_then(|id| self.services.store.playlist_tracks(id).ok())
                .unwrap_or_default();
            self.playlists.set_selected_tracks(tracks);
        }
    }

    pub(super) fn select_user_playlist(&mut self, playlist_id: &str) {
        self.playlists.select_by_id(playlist_id);
        self.pending_sync = self.services.sync.pending_count();
        self.sync_selected_playlist_tracks(true);
    }

    pub(super) fn update_nav_saved_state(
        &mut self,
        kind: SavedCollectionKind,
        browse_id: &str,
        saved: bool,
    ) {
        for page in &mut self.nav_stack {
            match (page, kind) {
                (NavPage::Artist(v), SavedCollectionKind::Artist) if v.browse_id == browse_id => {
                    v.saved = saved;
                }
                (NavPage::Album(v), SavedCollectionKind::Album) if v.browse_id == browse_id => {
                    v.saved = saved;
                }
                (NavPage::PlaylistDetail(v), SavedCollectionKind::Playlist)
                    if v.browse_id == browse_id =>
                {
                    v.saved = saved;
                }
                _ => {}
            }
        }
    }

    pub(super) fn ensure_visible_covers(&mut self) {
        if !self.covers.is_enabled() {
            return;
        }
        let mut urls = Vec::new();
        let window = |cursor: usize, len: usize| -> (usize, usize) {
            let start = cursor.saturating_sub(4);
            let end = (cursor + 12).min(len);
            (start, end)
        };

        if let Some(top) = self.nav_stack.last() {
            if let NavPage::Artist(v) = top
                && let Some(ref art) = v.artist
            {
                let (s, e) = window(v.cursor, art.songs.len());
                for tr in &art.songs[s..e] {
                    if let Some(u) = tr.cover_url() {
                        urls.push(u);
                    }
                }
            }
        } else {
            match self.active_tab {
                RootTab::Player => match self.player.mode {
                    PlayerPanelMode::Discover => match self.player.discover_tab {
                        DiscoverTab::Mixes => {
                            let (s, e) = window(self.player.mixes_cursor, self.player.feed.mixes.len());
                            for tr in &self.player.feed.mixes[s..e] {
                                if let Some(u) = tr.cover_url() {
                                    urls.push(u);
                                }
                            }
                        }
                        DiscoverTab::NewReleases => {
                            let (s, e) = window(
                                self.player.new_rel_cursor,
                                self.player.feed.new_releases.len(),
                            );
                            for it in &self.player.feed.new_releases[s..e] {
                                if let Some(u) = it.cover_url() {
                                    urls.push(u);
                                }
                            }
                        }
                        DiscoverTab::Featured => {
                            let (s, e) = window(
                                self.player.featured_cursor,
                                self.player.featured_items.len(),
                            );
                            for it in &self.player.featured_items[s..e] {
                                if let Some(u) = it.cover_url() {
                                    urls.push(u);
                                }
                            }
                        }
                    },
                    PlayerPanelMode::SearchResults => {
                        let (s, e) =
                            window(self.player.results_cursor, self.player.search_results.len());
                        for it in &self.player.search_results[s..e] {
                            if let Some(u) = it.cover_url() {
                                urls.push(u);
                            }
                        }
                    }
                    _ => {}
                },
                RootTab::Library => match self.library.active_tab {
                    LibraryTab::LikedSongs => {
                        let (s, e) = window(self.library.cursor, self.library.liked_songs.len());
                        for tr in &self.library.liked_songs[s..e] {
                            if let Some(u) = tr.cover_url() {
                                urls.push(u);
                            }
                        }
                    }
                    LibraryTab::Recent => {
                        let (s, e) = window(self.library.cursor, self.library.recent_tracks.len());
                        for tr in &self.library.recent_tracks[s..e] {
                            if let Some(u) = tr.cover_url() {
                                urls.push(u);
                            }
                        }
                    }
                    LibraryTab::SavedAlbums => {
                        let (s, e) = window(self.library.cursor, self.library.saved_albums.len());
                        for c in &self.library.saved_albums[s..e] {
                            if let Some(ref u) = c.thumb_url {
                                urls.push(u.clone());
                            }
                        }
                    }
                    LibraryTab::SavedArtists => {
                        let (s, e) = window(self.library.cursor, self.library.saved_artists.len());
                        for c in &self.library.saved_artists[s..e] {
                            if let Some(ref u) = c.thumb_url {
                                urls.push(u.clone());
                            }
                        }
                    }
                    LibraryTab::SavedPlaylists => {
                        let (s, e) =
                            window(self.library.cursor, self.library.saved_playlists.len());
                        for c in &self.library.saved_playlists[s..e] {
                            if let Some(ref u) = c.thumb_url {
                                urls.push(u.clone());
                            }
                        }
                    }
                    LibraryTab::UserPlaylists => {}
                },
                _ => {}
            }
        }

        for url in urls {
            self.request_cover_image(url);
        }
    }

    pub(super) fn reload_recent_plays(&mut self) {
        self.library.recent_tracks = self.services.store.recent_plays(50).unwrap_or_default();
    }

    pub(super) fn refresh_cache_stats(&mut self) {
        if self.last_cache_stats_at.elapsed() >= Duration::from_secs(5) {
            self.refresh_cache_stats_now();
        }
    }

    pub(super) fn refresh_cache_stats_now(&mut self) {
        self.cache_stats = self.services.cache.stats().unwrap_or_default();
        self.last_cache_stats_at = Instant::now();
    }
}
