use musick_core::{AppCommand, AuthMode};

use crate::keymap::Action;
use crate::views::{LibraryTab, PlaylistsPane, ViewEffect};
use crate::widgets::{ActiveModal, ConfirmAction, ContextMenuAction, RootTab};

use super::{App, BgMsg};

impl App {
    pub(super) fn handle_modal_action(&mut self, action: Action) {
        let Some(mut modal) = self.modal.take() else {
            return;
        };
        let mut keep = true;

        match &mut modal {
            ActiveModal::CreatePlaylist { input } => match action {
                Action::InputCancel => keep = false,
                Action::InputSubmit => {
                    let name = input.trim().to_string();
                    keep = false;
                    if !name.is_empty() {
                        match self.services.sync.create_playlist(&name, "", true) {
                            Ok(pl) => {
                                self.reload_local_collections();
                                self.select_user_playlist(&pl.id);
                                self.set_toast(format!("Created playlist \"{}\"", pl.title));
                            }
                            Err(err) => self.set_toast(format!("Create failed: {err}")),
                        }
                    }
                }
                _ => {
                    let _ = input.handle_action(&action);
                }
            },
            ActiveModal::RenamePlaylist { playlist_id, input } => match action {
                Action::InputCancel => keep = false,
                Action::InputSubmit => {
                    let name = input.trim().to_string();
                    let pid = playlist_id.clone();
                    keep = false;
                    if !name.is_empty() {
                        match self.services.sync.rename_playlist(&pid, &name, None) {
                            Ok(_) => {
                                self.reload_local_collections();
                                self.set_toast(format!("Renamed to \"{name}\""));
                            }
                            Err(err) => self.set_toast(format!("Rename failed: {err}")),
                        }
                    }
                }
                _ => {
                    let _ = input.handle_action(&action);
                }
            },
            ActiveModal::ImportPlaylistUrl { input } => match action {
                Action::InputCancel => keep = false,
                Action::InputSubmit => {
                    let raw = input.trim().to_string();
                    keep = false;
                    if !raw.is_empty() {
                        self.import_playlist_input(raw);
                    }
                }
                _ => {
                    let _ = input.handle_action(&action);
                }
            },
            ActiveModal::AddToPlaylist {
                track,
                playlists,
                cursor,
            } => match action {
                Action::InputCancel => keep = false,
                Action::InputDown | Action::InputChar('j') => {
                    if !playlists.is_empty() {
                        *cursor = (*cursor + 1).min(playlists.len() - 1);
                    }
                }
                Action::InputUp | Action::InputChar('k') => {
                    *cursor = cursor.saturating_sub(1);
                }
                Action::InputSubmit => {
                    if let Some(pl) = playlists.get(*cursor) {
                        match self
                            .services
                            .sync
                            .add_tracks_to_playlist(&pl.id, std::slice::from_ref(track))
                        {
                            Ok(_) => {
                                let title = pl.title.clone();
                                self.reload_local_collections();
                                self.set_toast(format!("Added to \"{title}\""));
                            }
                            Err(err) => self.set_toast(format!("Add failed: {err}")),
                        }
                    }
                    keep = false;
                }
                _ => {}
            },
            ActiveModal::Confirm { action: conf, .. } => match action {
                Action::InputCancel | Action::InputChar('n') | Action::InputChar('N') => {
                    keep = false;
                }
                Action::InputSubmit | Action::InputChar('y') | Action::InputChar('Y') => {
                    let c = conf.clone();
                    keep = false;
                    match c {
                        ConfirmAction::DeletePlaylist { playlist_id } => {
                            if let Ok(true) = self.services.sync.delete_playlist(&playlist_id) {
                                self.reload_local_collections();
                                self.set_toast("Playlist deleted");
                            }
                        }
                        ConfirmAction::ClearCache => self.apply_effect(ViewEffect::ClearCache),
                        ConfirmAction::Logout => self.apply_effect(ViewEffect::Logout),
                    }
                }
                _ => {}
            },
            ActiveModal::OAuthWizard {
                client_id,
                client_secret,
                page_id,
                active_field,
                device_user_code,
                polling,
                error,
                ..
            } => match action {
                Action::InputCancel => {
                    client_secret.clear();
                    self.cancel_oauth_task();
                    keep = false;
                }
                Action::InputTab if device_user_code.is_none() => {
                    *active_field = (*active_field + 1) % 3;
                }
                Action::InputBackTab if device_user_code.is_none() => {
                    *active_field = (*active_field + 2) % 3;
                }
                Action::InputChar(c) if device_user_code.is_none() => match *active_field {
                    0 => client_id.push(c),
                    1 => client_secret.push(c),
                    _ => page_id.push(c),
                },
                Action::InputBackspace if device_user_code.is_none() => match *active_field {
                    0 => {
                        client_id.pop();
                    }
                    1 => {
                        client_secret.pop();
                    }
                    _ => {
                        page_id.pop();
                    }
                },
                Action::InputSubmit if device_user_code.is_none() => {
                    if client_id.trim().is_empty() || client_secret.trim().is_empty() {
                        *error = Some("Client ID and Client Secret are required".into());
                    } else {
                        self.cancel_oauth_task();
                        *polling = true;
                        *error = None;
                        let cid = client_id.trim().to_string();
                        let auth = self.services.auth.clone();
                        let tx = self.bg_tx.clone();
                        let jh = tokio::spawn(async move {
                            let res = auth
                                .start_oauth_device_flow(&cid)
                                .await
                                .map(|p| {
                                    (p.device_code, p.user_code, p.verification_url, p.interval)
                                })
                                .map_err(|e| e.to_string());
                            let _ = tx.send(BgMsg::OAuthCodeStarted(res));
                        });
                        self.oauth_task = Some(jh.abort_handle());
                    }
                }
                _ => {}
            },
            ActiveModal::BrowserAuthImport {
                raw_headers,
                page_id,
                active_field,
                error,
            } => match action {
                Action::InputCancel => {
                    raw_headers.clear();
                    keep = false;
                }
                Action::InputTab => *active_field = (*active_field + 1) % 2,
                Action::InputBackTab => *active_field = (*active_field + 1) % 2,
                Action::InputChar(c) => {
                    if *active_field == 0 {
                        raw_headers.push(c);
                    } else {
                        page_id.push(c);
                    }
                }
                Action::InputBackspace => {
                    if *active_field == 0 {
                        raw_headers.pop();
                    } else {
                        page_id.pop();
                    }
                }
                Action::InputSubmit => {
                    let pid = if page_id.trim().is_empty() {
                        None
                    } else {
                        Some(page_id.trim().to_string())
                    };
                    match self.services.auth.import_browser_headers(raw_headers, pid) {
                        Ok(creds) => {
                            raw_headers.clear();
                            keep = false;
                            self.settings.privacy.auth_mode = AuthMode::Browser;
                            self.settings_view.secret_backend =
                                self.services.auth.secret_backend_label();
                            let _ = self.settings.save(&self.services.paths.settings_file());
                            self.services.cache.clear_visitor_data();
                            if let Some(vd) = creds.visitor_id.as_deref() {
                                self.services.cache.set_visitor_data(vd);
                            }
                            self.services.engine.try_send(AppCommand::ApplySettings {
                                settings: self.settings.clone(),
                            });
                            self.set_toast("Browser Auth imported — syncing library…");
                            self.trigger_sync_now();
                            self.trigger_discover_refresh(true);
                        }
                        Err(err) => *error = Some(err.to_string()),
                    }
                }
                _ => {}
            },
            ActiveModal::ContextMenu { items, cursor, .. } => match action {
                Action::InputCancel
                | Action::InputChar('e')
                | Action::InputChar('q')
                | Action::InputChar('Q') => {
                    keep = false;
                }
                Action::InputDown | Action::InputChar('j') => {
                    if !items.is_empty() {
                        *cursor = (*cursor + 1).min(items.len() - 1);
                    }
                }
                Action::InputUp | Action::InputChar('k') => {
                    *cursor = cursor.saturating_sub(1);
                }
                Action::InputHome | Action::InputChar('g') => *cursor = 0,
                Action::InputEnd | Action::InputChar('G') => {
                    *cursor = items.len().saturating_sub(1);
                }
                Action::InputSubmit => {
                    let chosen = items.get(*cursor).map(|it| it.action.clone());
                    keep = false;
                    if let Some(act) = chosen {
                        self.execute_context_menu_action(act);
                    }
                }
                _ => {}
            },
            ActiveModal::TextViewer { scroll, .. } => match action {
                Action::InputCancel
                | Action::InputChar('i')
                | Action::InputChar('q')
                | Action::InputChar('Q') => {
                    keep = false;
                }
                Action::InputDown | Action::InputChar('j') => *scroll = scroll.saturating_add(1),
                Action::InputUp | Action::InputChar('k') => *scroll = scroll.saturating_sub(1),
                Action::InputHome | Action::InputChar('g') => *scroll = 0,
                _ => {}
            },
            ActiveModal::KeymapHelp { scroll } => match action {
                Action::InputCancel
                | Action::InputChar('?')
                | Action::InputChar('q')
                | Action::InputChar('Q') => {
                    keep = false;
                }
                Action::InputDown | Action::InputChar('j') => *scroll = scroll.saturating_add(1),
                Action::InputUp | Action::InputChar('k') => *scroll = scroll.saturating_sub(1),
                _ => {}
            },
        }

        if keep {
            self.modal = Some(modal);
        }
    }

    fn execute_context_menu_action(&mut self, chosen: ContextMenuAction) {
        let on_user_pl = self.nav_stack.is_empty()
            && ((self.active_tab == RootTab::Playlists
                && self.playlists.pane == PlaylistsPane::List)
                || (self.active_tab == RootTab::Library
                    && self.library.active_tab == LibraryTab::UserPlaylists));

        if on_user_pl || self.active_hovered_item().is_some() {
            let mapped = match chosen {
                ContextMenuAction::PlayNow => Action::PlayDirect,
                ContextMenuAction::ShufflePlay => Action::ArtistShuffle,
                ContextMenuAction::PlayNext => Action::PlayNext,
                ContextMenuAction::AddToQueue => Action::AppendQueue,
                ContextMenuAction::StartRadio => Action::StartRadio,
                ContextMenuAction::GoArtist => Action::GoArtist,
                ContextMenuAction::GoAlbum => Action::GoAlbum,
                ContextMenuAction::AddToPlaylist | ContextMenuAction::SaveCopyToMyPlaylists => {
                    Action::AddToPlaylist
                }
                ContextMenuAction::ToggleLike => Action::ToggleLike,
                ContextMenuAction::ToggleSave => Action::ToggleSave,
                ContextMenuAction::ToggleArtistFollow => Action::ToggleArtistFollow,
                ContextMenuAction::ShowArtistBio => Action::ShowArtistBio,
                ContextMenuAction::CopyUrl => Action::CopyUrl,
                ContextMenuAction::RemoveItem => Action::DeleteItem,
            };
            self.dispatch_action(mapped);
            return;
        }

        let Some(playing) = self.snapshot.track.clone() else {
            return;
        };
        match chosen {
            ContextMenuAction::PlayNow => {
                self.services.engine.try_send(AppCommand::TogglePause);
            }
            ContextMenuAction::PlayNext => {
                self.services
                    .engine
                    .try_send(AppCommand::PlayNext { tracks: vec![playing] });
            }
            ContextMenuAction::AddToQueue => {
                self.services.engine.try_send(AppCommand::AppendTracks {
                    tracks: vec![playing],
                });
            }
            ContextMenuAction::StartRadio | ContextMenuAction::ShufflePlay => {
                self.services.engine.try_send(AppCommand::StartRadio {
                    video_id: Some(playing.video_id.clone()),
                    playlist_id: None,
                    seed_track: Some(playing),
                });
            }
            ContextMenuAction::GoArtist => self.dispatch_action(Action::GoPlayingArtist),
            ContextMenuAction::GoAlbum => self.dispatch_action(Action::GoPlayingAlbum),
            ContextMenuAction::AddToPlaylist => {
                self.apply_effect(ViewEffect::AddToPlaylistPicker(playing));
            }
            ContextMenuAction::ToggleLike => {
                self.apply_effect(ViewEffect::ToggleLike(playing));
            }
            ContextMenuAction::CopyUrl => {
                self.apply_effect(ViewEffect::CopyUrl(playing.url()));
            }
            _ => {}
        }
    }
}
