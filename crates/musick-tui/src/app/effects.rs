use std::io::Write;
use std::time::Duration;

use base64::Engine as _;

use musick_core::{AppCommand, AuthMode, SavedCollectionKind};

use crate::text::format_bytes;
use crate::theme::set_active_accent;
use crate::views::artist::shuffle_tracks_in_place;
use crate::views::{AlbumView, ArtistTab, ArtistView, PlaylistDetailView, ViewEffect};
use crate::widgets::{ActiveModal, RootTab};

use super::{App, BgMsg, NavPage, SESSION_KV_KEY};

impl App {
    pub(super) fn apply_effect(&mut self, effect: ViewEffect) {
        match effect {
            ViewEffect::None => {}
            ViewEffect::Command(cmd) => {
                self.services.engine.try_send(cmd);
            }
            ViewEffect::OpenArtist {
                browse_id,
                name_hint,
            } => {
                self.open_artist_page(browse_id, name_hint, ArtistTab::Overview);
            }
            ViewEffect::OpenArtistTab {
                browse_id,
                name_hint,
                tab,
            } => {
                self.open_artist_page(browse_id, name_hint, tab);
            }
            ViewEffect::OpenAlbum {
                browse_id,
                title_hint,
            } => {
                let saved = self
                    .services
                    .store
                    .is_collection_saved(
                        &self.services.auth.account_key(),
                        SavedCollectionKind::Album,
                        &browse_id,
                    )
                    .unwrap_or(false);
                self.push_nav(NavPage::Album(AlbumView::new(
                    browse_id.clone(),
                    title_hint,
                    saved,
                )));
                let cat = self.services.catalog.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    let res = cat
                        .album(&browse_id)
                        .await
                        .map(Box::new)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(BgMsg::AlbumLoaded {
                        browse_id,
                        result: res,
                    });
                });
            }
            ViewEffect::OpenPlaylist {
                browse_id,
                title_hint,
            } => {
                let saved = self
                    .services
                    .store
                    .is_collection_saved(
                        &self.services.auth.account_key(),
                        SavedCollectionKind::Playlist,
                        &browse_id,
                    )
                    .unwrap_or(false);
                self.push_nav(NavPage::PlaylistDetail(PlaylistDetailView::new(
                    browse_id.clone(),
                    title_hint,
                    saved,
                )));
                let cat = self.services.catalog.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    let res = cat
                        .playlist(&browse_id, 200)
                        .await
                        .map(Box::new)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(BgMsg::PlaylistLoaded {
                        browse_id,
                        result: res,
                    });
                });
            }
            ViewEffect::OpenUserPlaylist { playlist_id } => {
                self.active_tab = RootTab::Playlists;
                self.nav_stack.clear();
                self.reload_local_collections();
                self.select_user_playlist(&playlist_id);
            }
            ViewEffect::PlayUserPlaylistDirect { playlist_id } => {
                match self.services.store.playlist_tracks(&playlist_id) {
                    Ok(tracks) if !tracks.is_empty() => {
                        self.services.engine.try_send(AppCommand::PlayTracks {
                            tracks,
                            start_index: 0,
                            autoplay: true,
                        });
                    }
                    Ok(_) => self.set_toast("Playlist is empty"),
                    Err(err) => self.set_toast(format!("Playlist load error: {err}")),
                }
            }
            ViewEffect::PlayAlbumDirect { browse_id } => {
                let cat = self.services.catalog.clone();
                let eng = self.services.engine.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    match cat.album(&browse_id).await {
                        Ok(alb) if !alb.tracks.is_empty() => {
                            eng.send(AppCommand::PlayTracks {
                                tracks: alb.tracks,
                                start_index: 0,
                                autoplay: true,
                            })
                            .await;
                        }
                        Ok(_) => {
                            let _ = tx.send(BgMsg::Toast("Album has no playable tracks".into()));
                        }
                        Err(err) => {
                            let _ = tx.send(BgMsg::Toast(format!("Album play error: {err}")));
                        }
                    }
                });
            }
            ViewEffect::PlayPlaylistDirect { browse_id } => {
                let cat = self.services.catalog.clone();
                let eng = self.services.engine.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    match cat.playlist(&browse_id, 100).await {
                        Ok(pl) if !pl.tracks.is_empty() => {
                            eng.send(AppCommand::PlayTracks {
                                tracks: pl.tracks,
                                start_index: 0,
                                autoplay: true,
                            })
                            .await;
                        }
                        Ok(_) => {
                            let _ = tx.send(BgMsg::Toast("Playlist is empty".into()));
                        }
                        Err(err) => {
                            let _ = tx.send(BgMsg::Toast(format!("Playlist play error: {err}")));
                        }
                    }
                });
            }
            ViewEffect::PlayArtistDirect { browse_id } => {
                let cat = self.services.catalog.clone();
                let eng = self.services.engine.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    match cat.artist(&browse_id).await {
                        Ok(art) if !art.songs.is_empty() => {
                            eng.send(AppCommand::PlayTracks {
                                tracks: art.songs,
                                start_index: 0,
                                autoplay: true,
                            })
                            .await;
                        }
                        Ok(_) => {
                            let _ = tx.send(BgMsg::Toast("Artist has no top songs".into()));
                        }
                        Err(err) => {
                            let _ = tx.send(BgMsg::Toast(format!("Artist play error: {err}")));
                        }
                    }
                });
            }
            ViewEffect::ShuffleAlbumDirect { browse_id } => {
                let cat = self.services.catalog.clone();
                let eng = self.services.engine.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    match cat.album(&browse_id).await {
                        Ok(mut alb) if !alb.tracks.is_empty() => {
                            shuffle_tracks_in_place(&mut alb.tracks);
                            eng.send(AppCommand::PlayTracks {
                                tracks: alb.tracks,
                                start_index: 0,
                                autoplay: true,
                            })
                            .await;
                        }
                        Ok(_) => {
                            let _ = tx.send(BgMsg::Toast("Album has no playable tracks".into()));
                        }
                        Err(err) => {
                            let _ = tx.send(BgMsg::Toast(format!("Album shuffle error: {err}")));
                        }
                    }
                });
            }
            ViewEffect::ShufflePlaylistDirect { browse_id } => {
                let cat = self.services.catalog.clone();
                let eng = self.services.engine.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    match cat.playlist(&browse_id, 100).await {
                        Ok(mut pl) if !pl.tracks.is_empty() => {
                            shuffle_tracks_in_place(&mut pl.tracks);
                            eng.send(AppCommand::PlayTracks {
                                tracks: pl.tracks,
                                start_index: 0,
                                autoplay: true,
                            })
                            .await;
                        }
                        Ok(_) => {
                            let _ = tx.send(BgMsg::Toast("Playlist is empty".into()));
                        }
                        Err(err) => {
                            let _ = tx.send(BgMsg::Toast(format!("Playlist shuffle error: {err}")));
                        }
                    }
                });
            }
            ViewEffect::ShuffleArtistDirect { browse_id } => {
                let cat = self.services.catalog.clone();
                let eng = self.services.engine.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    match cat.artist(&browse_id).await {
                        Ok(mut art) => {
                            if let Some(sid) = art.shuffle_id {
                                eng.send(AppCommand::StartRadio {
                                    video_id: None,
                                    playlist_id: Some(sid),
                                    seed_track: None,
                                })
                                .await;
                            } else if !art.songs.is_empty() {
                                shuffle_tracks_in_place(&mut art.songs);
                                eng.send(AppCommand::PlayTracks {
                                    tracks: art.songs,
                                    start_index: 0,
                                    autoplay: true,
                                })
                                .await;
                            } else {
                                let _ =
                                    tx.send(BgMsg::Toast("Artist has no playable songs".into()));
                            }
                        }
                        Err(err) => {
                            let _ = tx.send(BgMsg::Toast(format!("Artist shuffle error: {err}")));
                        }
                    }
                });
            }
            ViewEffect::ShuffleUserPlaylistDirect { playlist_id } => {
                match self.services.store.playlist_tracks(&playlist_id) {
                    Ok(mut tracks) if !tracks.is_empty() => {
                        shuffle_tracks_in_place(&mut tracks);
                        self.services.engine.try_send(AppCommand::PlayTracks {
                            tracks,
                            start_index: 0,
                            autoplay: true,
                        });
                    }
                    Ok(_) => self.set_toast("Playlist is empty"),
                    Err(err) => self.set_toast(format!("Playlist shuffle error: {err}")),
                }
            }
            ViewEffect::StartArtistRadio { browse_id } => {
                let cat = self.services.catalog.clone();
                let eng = self.services.engine.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    match cat.artist(&browse_id).await {
                        Ok(art) => {
                            if let Some(rid) = art.radio_id {
                                eng.send(AppCommand::StartRadio {
                                    video_id: None,
                                    playlist_id: Some(rid),
                                    seed_track: None,
                                })
                                .await;
                            } else if let Some(first) = art.songs.into_iter().next() {
                                eng.send(AppCommand::StartRadio {
                                    video_id: Some(first.video_id.clone()),
                                    playlist_id: None,
                                    seed_track: Some(first),
                                })
                                .await;
                            } else {
                                let _ = tx.send(BgMsg::Toast("Artist has no songs for mix".into()));
                            }
                        }
                        Err(err) => {
                            let _ = tx.send(BgMsg::Toast(format!("Artist mix error: {err}")));
                        }
                    }
                });
            }
            ViewEffect::StartAlbumRadio { browse_id } => {
                let cat = self.services.catalog.clone();
                let eng = self.services.engine.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    match cat.album(&browse_id).await {
                        Ok(alb) => {
                            if let Some(first) = alb.tracks.into_iter().next() {
                                eng.send(AppCommand::StartRadio {
                                    video_id: Some(first.video_id.clone()),
                                    playlist_id: None,
                                    seed_track: Some(first),
                                })
                                .await;
                            } else {
                                let _ = tx.send(BgMsg::Toast("Album has no tracks for mix".into()));
                            }
                        }
                        Err(err) => {
                            let _ = tx.send(BgMsg::Toast(format!("Album mix error: {err}")));
                        }
                    }
                });
            }
            ViewEffect::StartPlaylistRadio { browse_id } => {
                let cat = self.services.catalog.clone();
                let eng = self.services.engine.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    if browse_id.starts_with("RD") {
                        eng.send(AppCommand::StartRadio {
                            video_id: None,
                            playlist_id: Some(browse_id),
                            seed_track: None,
                        })
                        .await;
                        return;
                    }
                    match cat.playlist(&browse_id, 50).await {
                        Ok(pl) => {
                            if let Some(first) = pl.tracks.into_iter().next() {
                                eng.send(AppCommand::StartRadio {
                                    video_id: Some(first.video_id.clone()),
                                    playlist_id: None,
                                    seed_track: Some(first),
                                })
                                .await;
                            } else {
                                let _ = tx.send(BgMsg::Toast("Playlist is empty".into()));
                            }
                        }
                        Err(err) => {
                            let _ = tx.send(BgMsg::Toast(format!("Playlist mix error: {err}")));
                        }
                    }
                });
            }
            ViewEffect::QueueAlbum {
                browse_id,
                play_next,
            } => {
                let cat = self.services.catalog.clone();
                let eng = self.services.engine.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    match cat.album(&browse_id).await {
                        Ok(alb) if !alb.tracks.is_empty() => {
                            let cmd = if play_next {
                                AppCommand::PlayNext { tracks: alb.tracks }
                            } else {
                                AppCommand::AppendTracks { tracks: alb.tracks }
                            };
                            eng.send(cmd).await;
                        }
                        Ok(_) => {
                            let _ = tx.send(BgMsg::Toast("Album has no playable tracks".into()));
                        }
                        Err(err) => {
                            let _ = tx.send(BgMsg::Toast(format!("Queue album error: {err}")));
                        }
                    }
                });
            }
            ViewEffect::QueuePlaylist {
                browse_id,
                play_next,
            } => {
                let cat = self.services.catalog.clone();
                let eng = self.services.engine.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    match cat.playlist(&browse_id, 100).await {
                        Ok(pl) if !pl.tracks.is_empty() => {
                            let cmd = if play_next {
                                AppCommand::PlayNext { tracks: pl.tracks }
                            } else {
                                AppCommand::AppendTracks { tracks: pl.tracks }
                            };
                            eng.send(cmd).await;
                        }
                        Ok(_) => {
                            let _ = tx.send(BgMsg::Toast("Playlist is empty".into()));
                        }
                        Err(err) => {
                            let _ = tx.send(BgMsg::Toast(format!("Queue playlist error: {err}")));
                        }
                    }
                });
            }
            ViewEffect::QueueUserPlaylist {
                playlist_id,
                play_next,
            } => match self.services.store.playlist_tracks(&playlist_id) {
                Ok(tracks) if !tracks.is_empty() => {
                    let cmd = if play_next {
                        AppCommand::PlayNext { tracks }
                    } else {
                        AppCommand::AppendTracks { tracks }
                    };
                    self.services.engine.try_send(cmd);
                }
                Ok(_) => self.set_toast("Playlist is empty"),
                Err(err) => self.set_toast(format!("Queue playlist error: {err}")),
            },
            ViewEffect::ImportRemotePlaylist { browse_id } => {
                self.set_toast("Saving playlist copy to My Playlists…");
                self.import_playlist_input(browse_id);
            }
            ViewEffect::TriggerSearch { category, query } => {
                if let Some(h) = self.search_task.take() {
                    h.abort();
                }
                self.search_gen = self.search_gen.wrapping_add(1);
                let gen_id = self.search_gen;
                let search = self.services.search.clone();
                let tx = self.bg_tx.clone();
                let jh = tokio::spawn(async move {
                    let res = search
                        .search(category, &query, 40)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(BgMsg::SearchLoaded {
                        gen_id,
                        result: res,
                    });
                });
                self.search_task = Some(jh.abort_handle());
            }
            ViewEffect::TriggerAutocomplete { query } => {
                if let Some(h) = self.autocomplete_task.take() {
                    h.abort();
                }
                self.autocomplete_gen = self.autocomplete_gen.wrapping_add(1);
                let gen_id = self.autocomplete_gen;
                let search = self.services.search.clone();
                let tx = self.bg_tx.clone();
                let jh = tokio::spawn(async move {
                    if !query.trim().is_empty() {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                    }
                    let sugs = search.autocomplete(&query, 7).await;
                    let _ = tx.send(BgMsg::SuggestionsLoaded {
                        gen_id,
                        suggestions: sugs,
                    });
                });
                self.autocomplete_task = Some(jh.abort_handle());
            }
            ViewEffect::RefreshDiscover => self.trigger_discover_refresh(true),
            ViewEffect::OpenModal(m) => self.modal = Some(m),
            ViewEffect::SyncNow => self.trigger_sync_now(),
            ViewEffect::ToggleLike(track) => match self.services.sync.toggle_like(&track) {
                Ok(true) => {
                    self.reload_local_collections();
                    self.set_toast(format!("Liked \"{}\"", track.title));
                }
                Ok(false) => {
                    self.reload_local_collections();
                    self.set_toast(format!("Removed \"{}\" from Liked Songs", track.title));
                }
                Err(err) => self.set_toast(format!("Like failed: {err}")),
            },
            ViewEffect::ToggleSave {
                kind,
                browse_id,
                title,
                subtitle,
                thumb_url,
            } => match self
                .services
                .sync
                .toggle_save_collection(kind, &browse_id, &title, subtitle, thumb_url)
            {
                Ok(now_saved) => {
                    self.update_nav_saved_state(kind, &browse_id, now_saved);
                    self.reload_local_collections();
                    if now_saved {
                        self.set_toast(format!("Saved \"{title}\" to Library"));
                    } else {
                        self.set_toast(format!("Removed \"{title}\" from Library"));
                    }
                }
                Err(err) => self.set_toast(format!("Save failed: {err}")),
            },
            ViewEffect::AddToPlaylistPicker(track) => {
                let playlists = self
                    .services
                    .store
                    .list_playlists(&self.services.auth.account_key())
                    .unwrap_or_default();
                self.modal = Some(ActiveModal::AddToPlaylist {
                    track,
                    playlists,
                    cursor: 0,
                });
            }
            ViewEffect::RemovePlaylistTrack {
                playlist_id,
                position,
            } => match self.services.sync.remove_track_at(&playlist_id, position) {
                Ok(Some(_)) => {
                    self.reload_local_collections();
                    self.set_toast("Removed track from playlist");
                }
                Ok(None) => {
                    self.reload_local_collections();
                }
                Err(err) => self.set_toast(format!("Remove failed: {err}")),
            },
            ViewEffect::MovePlaylistTrack {
                playlist_id,
                from,
                to,
            } => match self.services.sync.move_track(&playlist_id, from, to) {
                Ok(true) => {
                    self.reload_local_collections();
                }
                Ok(false) => {
                    self.playlists.track_cursor = from;
                    self.reload_local_collections();
                }
                Err(err) => {
                    self.playlists.track_cursor = from;
                    self.set_toast(format!("Reorder failed: {err}"));
                }
            },
            ViewEffect::ExportPlaylist { playlist_id } => {
                let dir = self.services.paths.data_dir.join("exports");
                let _ = std::fs::create_dir_all(&dir);
                if let Ok(Some(pl)) = self.services.store.get_playlist(&playlist_id) {
                    let safe: String = pl
                        .title
                        .chars()
                        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                        .collect();
                    let json_p = dir.join(format!("{safe}.json"));
                    let m3u_p = dir.join(format!("{safe}.m3u8"));
                    let ok1 = self
                        .services
                        .store
                        .export_playlist_json(&playlist_id)
                        .and_then(|s| musick_core::atomic_write_text(&json_p, &s));
                    let ok2 = self
                        .services
                        .store
                        .export_playlist_m3u8(&playlist_id)
                        .and_then(|s| musick_core::atomic_write_text(&m3u_p, &s));
                    if ok1.is_ok() && ok2.is_ok() {
                        self.set_toast(format!("Exported to {}", dir.display()));
                    } else {
                        self.set_toast("Playlist export failed");
                    }
                }
            }
            ViewEffect::UploadPlaylist { playlist_id } => {
                let sync = self.services.sync.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    let res = sync
                        .promote_local_to_synced(&playlist_id)
                        .await
                        .map(|pl| pl.title)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(BgMsg::PlaylistUploaded(res));
                });
            }
            ViewEffect::CopyUrl(url) => {
                write_osc52_clipboard(&url);
                self.set_toast(format!("Copied URL: {url}"));
            }
            ViewEffect::UpdateSettings(next) => {
                let mut s = *next;
                s.normalize();
                let region_changed = s.display.region != self.settings.display.region;
                let mpris_changed = s.player.mpris_enabled != self.settings.player.mpris_enabled;
                let sync_interval_changed =
                    s.sync.auto_sync_interval_secs != self.settings.sync.auto_sync_interval_secs;
                let images_cap_shrunk = s.cache.images_max_mib < self.settings.cache.images_max_mib;
                let history_disabled =
                    self.settings.privacy.save_history && !s.privacy.save_history;
                self.settings = s.clone();
                set_active_accent(s.display.accent);
                if history_disabled {
                    self.session_dirty_at = None;
                    let _ = self.services.store.delete_kv(SESSION_KV_KEY);
                }
                self.spinner.reduced_motion = s.display.reduced_motion;
                let proto_changed = self.covers.set_protocol(s.display.image_protocol);
                self.services.catalog.set_region(&s.display.region);
                self.services.cache.update_settings(s.cache.clone());
                self.services.search.update_settings(s.clone());
                self.services.engine.try_send(AppCommand::ApplySettings {
                    settings: s.clone(),
                });
                let _ = s.save(&self.services.paths.settings_file());
                self.sync_cava_state();
                if mpris_changed {
                    self.sync_mpris_state();
                }
                if sync_interval_changed && let Some(ref tx) = self.sync_interval_tx {
                    let _ = tx.send(s.sync.auto_sync_interval_secs.max(60));
                }
                if images_cap_shrunk {
                    let c = self.services.cache.clone();
                    tokio::task::spawn_blocking(move || {
                        let _ = c.prune_all();
                    });
                }
                if proto_changed {
                    self.requested_images.clear();
                    self.re_request_active_covers();
                }
                if region_changed {
                    self.trigger_discover_refresh(true);
                }
            }
            ViewEffect::ClearCache => {
                self.services.images.clear_memory_cache();
                self.covers.clear_cache();
                self.requested_images.clear();
                match self.services.cache.clear_cache() {
                    Ok(()) => {
                        self.refresh_cache_stats();
                        self.re_request_active_covers();
                        self.set_toast("Cache cleared & visitorData rotated (Playlists safe)");
                    }
                    Err(err) => self.set_toast(format!("Clear cache failed: {err}")),
                }
            }
            ViewEffect::PruneCache => match self.services.cache.prune_all() {
                Ok(rep) => {
                    self.refresh_cache_stats();
                    self.set_toast(format!("Pruned {}", format_bytes(rep.bytes_freed)));
                }
                Err(err) => self.set_toast(format!("Prune failed: {err}")),
            },
            ViewEffect::ClearHistory => {
                let _ = self.services.store.clear_play_history();
                let _ = self.services.store.clear_search_history();
                let _ = self.services.store.delete_kv(SESSION_KV_KEY);
                self.session_dirty_at = None;
                self.reload_recent_plays();
                self.set_toast("Local history cleared");
            }
            ViewEffect::Logout => {
                self.cancel_oauth_task();
                let prev_acct = self.services.auth.account_key();
                if let Err(err) = self.services.auth.logout() {
                    self.set_toast(format!("Logout failed: {err}"));
                    return;
                }
                let _ = self.services.store.purge_account(&prev_acct);
                self.services.cache.clear_visitor_data();
                self.settings.privacy.auth_mode = AuthMode::Anonymous;
                self.settings.privacy.report_playback_history = false;
                let _ = self.settings.save(&self.services.paths.settings_file());
                self.services.engine.try_send(AppCommand::ApplySettings {
                    settings: self.settings.clone(),
                });
                self.reload_local_collections();
                self.settings_view.secret_backend = self.services.auth.secret_backend_label();
                self.trigger_discover_refresh(true);
                self.set_toast("Logged out -> no cookies ● (local playlists kept)");
            }
            ViewEffect::StatusToast(msg) => self.set_toast(msg),
        }
    }

    fn open_artist_page(&mut self, browse_id: String, name_hint: String, tab: ArtistTab) {
        let saved = self
            .services
            .store
            .is_collection_saved(
                &self.services.auth.account_key(),
                SavedCollectionKind::Artist,
                &browse_id,
            )
            .unwrap_or(false);
        self.push_nav(NavPage::Artist(Box::new(
            ArtistView::new(browse_id.clone(), name_hint, saved).with_tab(tab),
        )));
        let cat = self.services.catalog.clone();
        let tx = self.bg_tx.clone();
        tokio::spawn(async move {
            let res = cat
                .artist(&browse_id)
                .await
                .map(Box::new)
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::ArtistLoaded {
                browse_id,
                result: res,
            });
        });
    }
}

fn write_osc52_clipboard(text: &str) {
    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let mut stdout = std::io::stdout();
    let _ = write!(stdout, "\x1b]52;c;{b64}\x07");
    let _ = stdout.flush();
}
