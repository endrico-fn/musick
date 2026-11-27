use std::time::Duration;

use musick_core::{
    AppCommand, AuthMode, OAuthPollStatus, SearchItem, Track, YoutubeTarget, extract_youtube_target,
};

use crate::widgets::ActiveModal;

use super::{App, BgMsg, NavPage};

impl App {
    pub fn handle_bg_msg(&mut self, msg: BgMsg) {
        match msg {
            BgMsg::DiscoverLoaded { gen_id, result } => {
                if gen_id != self.discover_gen {
                    return;
                }
                self.discover_task = None;
                match result {
                    Ok(feed) => {
                        self.player.set_discover_feed(feed);
                        let mut cover_urls: Vec<String> = self
                            .player
                            .feed
                            .mixes
                            .iter()
                            .take(24)
                            .filter_map(Track::cover_url)
                            .collect();
                        cover_urls.extend(
                            self.player
                                .feed
                                .new_releases
                                .iter()
                                .take(24)
                                .chain(self.player.featured_items.iter().take(24))
                                .filter_map(SearchItem::cover_url),
                        );
                        for url in cover_urls {
                            self.request_cover_image(url);
                        }
                        self.refresh_cache_stats();
                    }
                    Err(err) => {
                        self.player.loading_discover = false;
                        self.set_toast(format!("Discover load error: {err}"));
                    }
                }
            }
            BgMsg::SearchLoaded { gen_id, result } => {
                if gen_id != self.search_gen {
                    return;
                }
                self.search_task = None;
                match result {
                    Ok(items) => {
                        let cover_urls: Vec<String> = items
                            .iter()
                            .take(30)
                            .filter_map(SearchItem::cover_url)
                            .collect();
                        self.player.set_search_results(items);
                        for url in cover_urls {
                            self.request_cover_image(url);
                        }
                        self.refresh_cache_stats();
                    }
                    Err(err) => {
                        self.player.set_search_error(err.clone());
                        self.set_toast(format!("Search error: {err}"));
                    }
                }
            }
            BgMsg::SuggestionsLoaded {
                gen_id,
                suggestions,
            } => {
                if gen_id != self.autocomplete_gen {
                    return;
                }
                self.autocomplete_task = None;
                self.player.set_suggestions(suggestions);
            }
            BgMsg::ArtistLoaded { browse_id, result } => {
                let mut cover_urls = Vec::new();
                let mut err_toast = None;
                if let Some(NavPage::Artist(view)) = self.nav_stack.last_mut()
                    && view.browse_id == browse_id
                {
                    match result {
                        Ok(art) => {
                            if let Some(url) = art.summary.cover_url() {
                                cover_urls.push(url);
                            }
                            for tr in art.songs.iter().take(24) {
                                if let Some(u) = tr.cover_url() {
                                    cover_urls.push(u);
                                }
                            }
                            for tr in art.videos.iter().take(12) {
                                if let Some(u) = tr.cover_url() {
                                    cover_urls.push(u);
                                }
                            }
                            for al in art.albums.iter().chain(art.singles.iter()).take(32) {
                                if let Some(u) = al.cover_url() {
                                    cover_urls.push(u);
                                }
                            }
                            for rel in art.related.iter().take(16) {
                                if let Some(u) = rel.cover_url() {
                                    cover_urls.push(u);
                                }
                            }
                            for pl in art.playlists.iter().take(12) {
                                if let Some(u) = pl.cover_url() {
                                    cover_urls.push(u);
                                }
                            }
                            view.set_artist(*art);
                        }
                        Err(err) => {
                            view.loading = false;
                            err_toast = Some(format!("Artist load failed: {err}"));
                        }
                    }
                }
                for url in cover_urls {
                    self.request_cover_image(url);
                }
                if let Some(msg) = err_toast {
                    self.set_toast(msg);
                }
            }
            BgMsg::AlbumLoaded { browse_id, result } => {
                let mut cover_url = None;
                let mut err_toast = None;
                if let Some(NavPage::Album(view)) = self.nav_stack.last_mut()
                    && view.browse_id == browse_id
                {
                    match result {
                        Ok(alb) => {
                            cover_url = alb.summary.cover_url();
                            view.set_album(*alb);
                        }
                        Err(err) => {
                            view.loading = false;
                            err_toast = Some(format!("Album load failed: {err}"));
                        }
                    }
                }
                if let Some(url) = cover_url {
                    self.request_cover_image(url);
                }
                if let Some(msg) = err_toast {
                    self.set_toast(msg);
                }
            }
            BgMsg::PlaylistLoaded { browse_id, result } => {
                let mut cover_url = None;
                let mut err_toast = None;
                if let Some(NavPage::PlaylistDetail(view)) = self.nav_stack.last_mut()
                    && view.browse_id == browse_id
                {
                    match result {
                        Ok(pl) => {
                            cover_url = pl.summary.cover_url();
                            view.set_playlist(*pl);
                        }
                        Err(err) => {
                            view.loading = false;
                            err_toast = Some(format!("Playlist load failed: {err}"));
                        }
                    }
                }
                if let Some(url) = cover_url {
                    self.request_cover_image(url);
                }
                if let Some(msg) = err_toast {
                    self.set_toast(msg);
                }
            }
            BgMsg::ImageLoaded { url, image } => {
                self.covers.insert_image(url, image);
            }
            BgMsg::ImageFailed { url } => {
                self.requested_images.remove(&url);
            }
            BgMsg::SyncFinished(res) => {
                self.syncing = false;
                self.pending_sync = self.services.sync.pending_count();
                match res {
                    Ok(rep) if rep.skipped_anonymous => {
                        self.set_toast("Anonymous mode — sign in to sync with YouTube Music");
                    }
                    Ok(rep) if rep.auth_expired => {
                        self.reload_local_collections();
                        self.set_toast("Session expired (401/403) — re-authenticate in 5 Settings");
                    }
                    Ok(rep) => {
                        self.reload_local_collections();
                        self.set_toast(format!(
                            "Synced: {}↑ {} playlists · {} likes",
                            rep.pushed, rep.pulled_playlists, rep.pulled_likes
                        ));
                    }
                    Err(err) => self.set_toast(format!("Sync error: {err}")),
                }
            }
            BgMsg::PlaylistUploaded(res) => match res {
                Ok(title) => {
                    self.reload_local_collections();
                    self.set_toast(format!("Uploaded \"{title}\" to YouTube Music"));
                }
                Err(err) => self.set_toast(format!("Upload failed: {err}")),
            },
            BgMsg::PlaylistImported(res) => match res {
                Ok((id, title, count)) => {
                    self.reload_local_collections();
                    self.select_user_playlist(&id);
                    self.set_toast(format!("Imported \"{title}\" ({count} tracks)"));
                }
                Err(err) => self.set_toast(format!("Import failed: {err}")),
            },
            BgMsg::OAuthCodeStarted(res) => {
                if !matches!(self.modal, Some(ActiveModal::OAuthWizard { .. })) {
                    return;
                }
                match res {
                    Ok((device_code, user_code, verification_url, interval)) => {
                        if let Some(ActiveModal::OAuthWizard {
                            client_id,
                            client_secret,
                            page_id,
                            device_user_code,
                            verification_url: v_url,
                            polling,
                            error,
                            ..
                        }) = self.modal.as_mut()
                        {
                            *device_user_code = Some(user_code);
                            *v_url = Some(verification_url);
                            *polling = true;
                            *error = None;
                            let cid = client_id.clone();
                            let csec = client_secret.clone();
                            let pid = if page_id.trim().is_empty() {
                                None
                            } else {
                                Some(page_id.trim().to_string())
                            };
                            self.spawn_oauth_poller(cid, csec, device_code, pid, interval);
                        }
                    }
                    Err(err) => {
                        if let Some(ActiveModal::OAuthWizard { error, polling, .. }) =
                            self.modal.as_mut()
                        {
                            *polling = false;
                            *error = Some(err);
                        }
                    }
                }
            }
            BgMsg::OAuthPollCompleted(res) => {
                self.oauth_task = None;
                if !matches!(self.modal, Some(ActiveModal::OAuthWizard { .. })) {
                    return;
                }
                match res {
                    Ok(OAuthPollStatus::Authorized(_)) => {
                        self.modal = None;
                        self.settings.privacy.auth_mode = AuthMode::OAuth;
                        self.settings_view.secret_backend =
                            self.services.auth.secret_backend_label();
                        let _ = self.settings.save(&self.services.paths.settings_file());
                        self.services.cache.clear_visitor_data();
                        self.services.engine.try_send(AppCommand::ApplySettings {
                            settings: self.settings.clone(),
                        });
                        self.set_toast("Signed in via OAuth2 — syncing library…");
                        self.trigger_sync_now();
                        self.trigger_discover_refresh(true);
                    }
                    Ok(_) => {}
                    Err(err) => {
                        if let Some(ActiveModal::OAuthWizard { error, polling, .. }) =
                            self.modal.as_mut()
                        {
                            *polling = false;
                            *error = Some(err);
                        }
                    }
                }
            }
            BgMsg::Toast(msg) => self.set_toast(msg),
        }
    }

    pub(super) fn trigger_discover_refresh(&mut self, force_refresh: bool) {
        if let Some(h) = self.discover_task.take() {
            h.abort();
        }
        self.discover_gen = self.discover_gen.wrapping_add(1);
        let gen_id = self.discover_gen;
        self.player.loading_discover = true;
        let cat = self.services.catalog.clone();
        let region = self.settings.display.region.clone();
        let tx = self.bg_tx.clone();
        let jh = tokio::spawn(async move {
            let res = if force_refresh {
                cat.discover_feed_fresh(&region).await
            } else {
                cat.discover_feed(&region).await
            }
            .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::DiscoverLoaded {
                gen_id,
                result: res,
            });
        });
        self.discover_task = Some(jh.abort_handle());
    }

    pub(super) fn trigger_sync_now(&mut self) {
        if self.syncing {
            return;
        }
        if !self.services.auth.is_authenticated() {
            self.set_toast("Anonymous mode — sign in via 5 Settings to enable 2-way sync");
            return;
        }
        self.syncing = true;
        let sync = self.services.sync.clone();
        let tx = self.bg_tx.clone();
        tokio::spawn(async move {
            let res = sync.sync_now().await.map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::SyncFinished(res));
        });
    }

    pub(super) fn import_playlist_input(&mut self, raw: String) {
        let path = std::path::Path::new(&raw);
        if path.is_file()
            && let Ok(content) = std::fs::read_to_string(path)
        {
            let acct = "local";
            let res = if raw.ends_with(".json") {
                self.services.store.import_playlist_json(acct, &content)
            } else {
                let fallback = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Imported");
                self.services
                    .store
                    .import_playlist_m3u8(acct, fallback, &content)
            };
            match res {
                Ok(pl) => {
                    self.reload_local_collections();
                    self.select_user_playlist(&pl.id);
                    self.set_toast(format!(
                        "Imported \"{}\" ({} tracks)",
                        pl.title, pl.track_count
                    ));
                }
                Err(err) => self.set_toast(format!("Import file error: {err}")),
            }
            return;
        }

        let pid = match extract_youtube_target(&raw) {
            Some(YoutubeTarget::Playlist(id)) => id,
            _ => raw,
        };
        let cat = self.services.catalog.clone();
        let sync = self.services.sync.clone();
        let tx = self.bg_tx.clone();
        tokio::spawn(async move {
            let res = match cat.playlist(&pid, 200).await {
                Ok(remote) => match sync.create_playlist(
                    &remote.summary.title,
                    remote.description.as_deref().unwrap_or(""),
                    false,
                ) {
                    Ok(local_pl) => {
                        let _ = sync.add_tracks_to_playlist(&local_pl.id, &remote.tracks);
                        Ok((local_pl.id, remote.summary.title, remote.tracks.len()))
                    }
                    Err(err) => Err(format!("Save import failed: {err}")),
                },
                Err(err) => Err(format!("Import URL failed: {err}")),
            };
            let _ = tx.send(BgMsg::PlaylistImported(res));
        });
    }

    fn spawn_oauth_poller(
        &mut self,
        client_id: String,
        client_secret: String,
        device_code: String,
        page_id: Option<String>,
        interval_secs: u64,
    ) {
        self.cancel_oauth_task();
        let auth = self.services.auth.clone();
        let tx = self.bg_tx.clone();
        let jh = tokio::spawn(async move {
            let mut wait = Duration::from_secs(interval_secs.max(2));
            for _ in 0..120 {
                tokio::time::sleep(wait).await;
                match auth
                    .poll_oauth_device_token(
                        &client_id,
                        &client_secret,
                        &device_code,
                        page_id.clone(),
                    )
                    .await
                {
                    Ok(OAuthPollStatus::Pending) => {}
                    Ok(OAuthPollStatus::SlowDown) => wait += Duration::from_secs(3),
                    Ok(OAuthPollStatus::Authorized(c)) => {
                        let _ = tx.send(BgMsg::OAuthPollCompleted(Ok(
                            OAuthPollStatus::Authorized(c),
                        )));
                        return;
                    }
                    Err(err) => {
                        let _ = tx.send(BgMsg::OAuthPollCompleted(Err(err.to_string())));
                        return;
                    }
                }
            }
            let _ = tx.send(BgMsg::OAuthPollCompleted(Err(
                "OAuth device code timed out".into(),
            )));
        });
        self.oauth_task = Some(jh.abort_handle());
    }
}
