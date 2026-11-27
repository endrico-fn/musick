use crossterm::event::KeyEvent;

use musick_core::{AppCommand, SearchItem, Track};

use crate::keymap::{Action, InputMode};
use crate::views::{
    ArtistTab, LibraryTab, PlayerFocusPane, PlayerPanelMode, PlaylistsPane, ViewEffect,
};
use crate::widgets::{
    ActiveModal, ContextMenuAction, ContextMenuItem, PlayerBarControl, RootTab,
};

use super::{App, MAX_NAV_DEPTH, NavPage};

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) {
        let mode = self.input_mode();
        let Some(action) = self.keymap.handle_key(mode, key) else {
            return;
        };
        self.dispatch_action(action);
    }

    pub fn handle_paste(&mut self, pasted: &str) {
        if self.input_mode() == InputMode::Normal {
            return;
        }
        let allow_newlines = matches!(
            self.modal,
            Some(ActiveModal::BrowserAuthImport {
                active_field: 0,
                ..
            })
        );
        for ch in pasted.chars() {
            if ch == '\r' || (ch == '\n' && !allow_newlines) {
                continue;
            }
            self.dispatch_action(Action::InputChar(ch));
        }
    }

    pub(super) fn switch_root_tab(&mut self, tab: RootTab) {
        self.active_tab = tab;
        self.player_bar_focused = false;
        self.nav_stack.clear();
        if matches!(tab, RootTab::Playlists | RootTab::Library) {
            self.reload_local_collections();
        }
        if tab == RootTab::Settings {
            self.refresh_cache_stats_now();
        }
        self.sync_cava_state();
    }

    pub fn dispatch_action(&mut self, action: Action) {
        if self.modal.is_some() {
            self.handle_modal_action(action);
            return;
        }
        if self.handle_player_bar_action(&action) {
            return;
        }

        match action {
            Action::Quit => {
                self.should_quit = true;
                return;
            }
            Action::ToggleHelp => {
                self.modal = Some(ActiveModal::KeymapHelp { scroll: 0 });
                return;
            }
            Action::OpenContextMenu => {
                self.open_context_menu();
                return;
            }
            Action::FocusPaneDown => {
                let enters_lyrics = self.nav_stack.is_empty()
                    && self.active_tab == RootTab::Player
                    && self.player.mode != PlayerPanelMode::Lyrics
                    && self.player.focus_pane == PlayerFocusPane::Main;
                if !enters_lyrics {
                    self.player_bar_focused = true;
                    self.player_bar_control = PlayerBarControl::Seekbar;
                    return;
                }
            }
            Action::SwitchTab(idx) => {
                if let Some(tab) = RootTab::from_index(idx) {
                    self.switch_root_tab(tab);
                }
                return;
            }
            Action::NextTab => {
                self.switch_root_tab(self.active_tab.next());
                return;
            }
            Action::PrevTab => {
                self.switch_root_tab(self.active_tab.prev());
                return;
            }
            Action::ToggleLyrics => {
                self.active_tab = RootTab::Player;
                self.nav_stack.clear();
                self.player.toggle_expanded_lyrics();
                self.sync_cava_state();
                return;
            }
            Action::LyricsOffsetEarlier => {
                self.services
                    .engine
                    .try_send(AppCommand::AdjustLyricsOffset { delta_secs: -0.25 });
                return;
            }
            Action::LyricsOffsetLater => {
                self.services
                    .engine
                    .try_send(AppCommand::AdjustLyricsOffset { delta_secs: 0.25 });
                return;
            }
            Action::SyncNow => {
                self.trigger_sync_now();
                return;
            }
            Action::RefreshDiscover => {
                self.trigger_discover_refresh(true);
                self.set_toast("Refreshing Discover…");
                return;
            }
            Action::ToggleTranslation => {
                self.services
                    .engine
                    .try_send(AppCommand::ToggleLyricsTranslation);
                return;
            }
            Action::TogglePause => {
                self.services.engine.try_send(AppCommand::TogglePause);
                return;
            }
            Action::ToggleMute => {
                self.services.engine.try_send(AppCommand::ToggleMute);
                return;
            }
            Action::NextTrack => {
                self.services.engine.try_send(AppCommand::Next);
                return;
            }
            Action::PrevTrack => {
                self.services.engine.try_send(AppCommand::Previous);
                return;
            }
            Action::SeekForward => {
                self.services
                    .engine
                    .try_send(AppCommand::SeekRelative { seconds: 5.0 });
                return;
            }
            Action::SeekBackward => {
                self.services
                    .engine
                    .try_send(AppCommand::SeekRelative { seconds: -5.0 });
                return;
            }
            Action::SeekForwardFast => {
                self.services
                    .engine
                    .try_send(AppCommand::SeekRelative { seconds: 30.0 });
                return;
            }
            Action::SeekBackwardFast => {
                self.services
                    .engine
                    .try_send(AppCommand::SeekRelative { seconds: -30.0 });
                return;
            }
            Action::VolumeUp => {
                self.services
                    .engine
                    .try_send(AppCommand::AdjustVolume { delta: 5 });
                return;
            }
            Action::VolumeDown => {
                self.services
                    .engine
                    .try_send(AppCommand::AdjustVolume { delta: -5 });
                return;
            }
            Action::ToggleShuffle => {
                self.services.engine.try_send(AppCommand::ToggleShuffle);
                return;
            }
            Action::CycleRepeat => {
                self.services.engine.try_send(AppCommand::CycleRepeat);
                return;
            }
            Action::GlobalSearch => {
                self.active_tab = RootTab::Player;
                self.nav_stack.clear();
                self.sync_cava_state();
                let eff = self.player.handle_action(Action::StartSearch);
                self.apply_effect(eff);
                return;
            }
            Action::GoArtist => {
                self.handle_quick_go_artist();
                return;
            }
            Action::GoAlbum => {
                self.handle_quick_go_album();
                return;
            }
            Action::GoPlayingArtist => {
                if let Some(track) = self.snapshot.track.clone() {
                    self.open_artist_for_track(&track);
                } else {
                    self.set_toast("Nothing playing");
                }
                return;
            }
            Action::GoPlayingAlbum => {
                if let Some(track) = self.snapshot.track.clone() {
                    self.open_album_for_track(&track);
                } else {
                    self.set_toast("Nothing playing");
                }
                return;
            }
            Action::GoBack => {
                if !self.nav_stack.is_empty() {
                    self.nav_stack.pop();
                    self.sync_cava_state();
                }
                return;
            }
            Action::Escape if !self.nav_stack.is_empty() => {
                let clear_filter = match self.nav_stack.last_mut() {
                    Some(NavPage::Artist(v)) if v.has_filter() => {
                        let eff = v.handle_action(Action::Escape);
                        self.apply_effect(eff);
                        true
                    }
                    Some(NavPage::Album(v)) if v.has_filter() => {
                        let eff = v.handle_action(Action::Escape);
                        self.apply_effect(eff);
                        true
                    }
                    Some(NavPage::PlaylistDetail(v)) if v.has_filter() => {
                        let eff = v.handle_action(Action::Escape);
                        self.apply_effect(eff);
                        true
                    }
                    _ => false,
                };
                if clear_filter {
                    return;
                }
                self.nav_stack.pop();
                self.sync_cava_state();
                return;
            }
            Action::StartSearch
                if self.nav_stack.is_empty() && self.active_tab == RootTab::Queue =>
            {
                self.active_tab = RootTab::Player;
                self.sync_cava_state();
                let eff = self.player.handle_action(Action::StartSearch);
                self.apply_effect(eff);
                return;
            }
            _ => {}
        }

        let effect = if let Some(top) = self.nav_stack.last_mut() {
            match top {
                NavPage::Artist(v) => v.handle_action(action),
                NavPage::Album(v) => v.handle_action(action),
                NavPage::PlaylistDetail(v) => v.handle_action(action),
            }
        } else {
            match self.active_tab {
                RootTab::Player => self.player.handle_action(action),
                RootTab::Playlists => {
                    self.sync_selected_playlist_tracks(false);
                    let eff = self.playlists.handle_action(action);
                    self.sync_selected_playlist_tracks(false);
                    eff
                }
                RootTab::Library => self.library.handle_action(action),
                RootTab::Queue => self.queue.handle_action(action, &self.queue_snap),
                RootTab::Settings => self.settings_view.handle_action(
                    action,
                    &self.settings,
                    self.services.auth.mode(),
                ),
            }
        };

        self.apply_effect(effect);
    }

    pub(super) fn active_hovered_item(&self) -> Option<SearchItem> {
        if self.player_bar_focused {
            return None;
        }
        if let Some(top) = self.nav_stack.last() {
            match top {
                NavPage::Artist(v) => v.hovered_item(),
                NavPage::Album(v) => v.hovered_track().map(SearchItem::Track),
                NavPage::PlaylistDetail(v) => v.hovered_track().map(SearchItem::Track),
            }
        } else {
            match self.active_tab {
                RootTab::Player => self.player.hovered_item(),
                RootTab::Playlists => self.playlists.hovered_track().map(SearchItem::Track),
                RootTab::Library => self.library.hovered_item(),
                RootTab::Queue => self.queue.hovered_track(&self.queue_snap).map(SearchItem::Track),
                RootTab::Settings => None,
            }
        }
    }

    fn handle_quick_go_artist(&mut self) {
        match self.active_hovered_item() {
            Some(SearchItem::Artist(ar)) => {
                self.apply_effect(ViewEffect::OpenArtist {
                    browse_id: ar.browse_id,
                    name_hint: ar.name,
                });
                return;
            }
            Some(SearchItem::Album(al)) => {
                if let Some(art) = al.artists.iter().find(|a| a.id.is_some())
                    && let Some(ref bid) = art.id
                {
                    self.apply_effect(ViewEffect::OpenArtist {
                        browse_id: bid.clone(),
                        name_hint: art.name.clone(),
                    });
                    return;
                }
            }
            Some(SearchItem::Track(tr)) => {
                if let Some(art) = tr.artists.iter().find(|a| a.id.is_some())
                    && let Some(ref bid) = art.id
                {
                    self.apply_effect(ViewEffect::OpenArtist {
                        browse_id: bid.clone(),
                        name_hint: art.name.clone(),
                    });
                    return;
                }
                if let Some(NavPage::Album(v)) = self.nav_stack.last()
                    && let Some(ref alb) = v.album
                    && let Some(art) = alb.summary.artists.iter().find(|a| a.id.is_some())
                    && let Some(ref bid) = art.id
                {
                    self.apply_effect(ViewEffect::OpenArtist {
                        browse_id: bid.clone(),
                        name_hint: art.name.clone(),
                    });
                    return;
                }
                self.set_toast("No artist link available for this track");
                return;
            }
            Some(SearchItem::Playlist(_)) | None => {}
        }

        if let Some(NavPage::Album(v)) = self.nav_stack.last()
            && let Some(ref alb) = v.album
            && let Some(art) = alb.summary.artists.iter().find(|a| a.id.is_some())
            && let Some(ref bid) = art.id
        {
            self.apply_effect(ViewEffect::OpenArtist {
                browse_id: bid.clone(),
                name_hint: art.name.clone(),
            });
            return;
        }

        if let Some(playing) = self.snapshot.track.clone() {
            self.open_artist_for_track(&playing);
        } else {
            self.set_toast("No artist available — select an item or play a song");
        }
    }

    fn handle_quick_go_album(&mut self) {
        match self.active_hovered_item() {
            Some(SearchItem::Artist(ar)) => {
                self.apply_effect(ViewEffect::OpenArtistTab {
                    browse_id: ar.browse_id,
                    name_hint: ar.name,
                    tab: ArtistTab::Albums,
                });
                return;
            }
            Some(SearchItem::Album(al)) => {
                self.apply_effect(ViewEffect::OpenAlbum {
                    browse_id: al.browse_id,
                    title_hint: al.title,
                });
                return;
            }
            Some(SearchItem::Track(tr)) => {
                self.open_album_for_track(&tr);
                return;
            }
            Some(SearchItem::Playlist(_)) | None => {}
        }

        if let Some(playing) = self.snapshot.track.clone() {
            self.open_album_for_track(&playing);
        } else {
            self.set_toast("No album available — select an item or play a song");
        }
    }

    fn open_context_menu(&mut self) {
        let on_user_pl = self.nav_stack.is_empty()
            && ((self.active_tab == RootTab::Playlists
                && self.playlists.pane == PlaylistsPane::List)
                || (self.active_tab == RootTab::Library
                    && self.library.active_tab == LibraryTab::UserPlaylists));
        if on_user_pl {
            let pl = if self.active_tab == RootTab::Playlists {
                self.playlists.selected_playlist().cloned()
            } else {
                self.library.selected_user_playlist().cloned()
            };
            if let Some(pl) = pl {
                let mut items = vec![
                    ContextMenuItem {
                        key: "p",
                        label: "Play playlist".into(),
                        action: ContextMenuAction::PlayNow,
                    },
                    ContextMenuItem {
                        key: "x",
                        label: "Shuffle play".into(),
                        action: ContextMenuAction::ShufflePlay,
                    },
                    ContextMenuItem {
                        key: "N",
                        label: "Play next".into(),
                        action: ContextMenuAction::PlayNext,
                    },
                    ContextMenuItem {
                        key: "q",
                        label: "Add to queue".into(),
                        action: ContextMenuAction::AddToQueue,
                    },
                ];
                if pl.remote_id.is_some() {
                    items.push(ContextMenuItem {
                        key: "yy",
                        label: "Copy playlist URL".into(),
                        action: ContextMenuAction::CopyUrl,
                    });
                }
                items.push(ContextMenuItem {
                    key: "dd",
                    label: "Delete playlist".into(),
                    action: ContextMenuAction::RemoveItem,
                });
                self.modal = Some(ActiveModal::ContextMenu {
                    title: pl.title,
                    subtitle: format!("{} tracks", pl.track_count),
                    items,
                    cursor: 0,
                });
                return;
            }
        }

        let item = self
            .active_hovered_item()
            .or_else(|| self.snapshot.track.clone().map(SearchItem::Track));
        let Some(item) = item else {
            self.set_toast("No item selected for menu");
            return;
        };

        let in_queue = self.nav_stack.is_empty() && self.active_tab == RootTab::Queue;
        let in_user_tracks = self.nav_stack.is_empty()
            && self.active_tab == RootTab::Playlists
            && self.playlists.pane == PlaylistsPane::Tracks;
        let on_artist_page = matches!(self.nav_stack.last(), Some(NavPage::Artist(_)));

        let (title, subtitle, items) = match item {
            SearchItem::Track(tr) => {
                let mut items = vec![
                    ContextMenuItem {
                        key: "Enter",
                        label: "Play now".into(),
                        action: ContextMenuAction::PlayNow,
                    },
                    ContextMenuItem {
                        key: "N",
                        label: "Play next".into(),
                        action: ContextMenuAction::PlayNext,
                    },
                    ContextMenuItem {
                        key: "q",
                        label: "Add to queue".into(),
                        action: ContextMenuAction::AddToQueue,
                    },
                    ContextMenuItem {
                        key: "r",
                        label: "Start song mix / radio".into(),
                        action: ContextMenuAction::StartRadio,
                    },
                ];
                if tr.artists.iter().any(|a| a.id.is_some()) {
                    items.push(ContextMenuItem {
                        key: "A",
                        label: "Go to artist".into(),
                        action: ContextMenuAction::GoArtist,
                    });
                }
                if tr.album_id.is_some() {
                    items.push(ContextMenuItem {
                        key: "a",
                        label: "Go to album".into(),
                        action: ContextMenuAction::GoAlbum,
                    });
                }
                items.push(ContextMenuItem {
                    key: "P",
                    label: "Add to playlist".into(),
                    action: ContextMenuAction::AddToPlaylist,
                });
                items.push(ContextMenuItem {
                    key: "f",
                    label: "Like / unlike song".into(),
                    action: ContextMenuAction::ToggleLike,
                });
                items.push(ContextMenuItem {
                    key: "yy",
                    label: "Copy song URL".into(),
                    action: ContextMenuAction::CopyUrl,
                });
                if in_queue || in_user_tracks {
                    items.push(ContextMenuItem {
                        key: "dd",
                        label: if in_queue {
                            "Remove from queue".into()
                        } else {
                            "Remove from playlist".into()
                        },
                        action: ContextMenuAction::RemoveItem,
                    });
                }
                (tr.title.clone(), tr.artist_names(), items)
            }
            SearchItem::Artist(ar) => {
                let mut items = vec![
                    ContextMenuItem {
                        key: "x",
                        label: "Shuffle play".into(),
                        action: ContextMenuAction::ShufflePlay,
                    },
                    ContextMenuItem {
                        key: "r",
                        label: "Start artist mix".into(),
                        action: ContextMenuAction::StartRadio,
                    },
                    ContextMenuItem {
                        key: "Enter",
                        label: "Open artist profile".into(),
                        action: ContextMenuAction::GoArtist,
                    },
                    ContextMenuItem {
                        key: "a",
                        label: "Open artist albums".into(),
                        action: ContextMenuAction::GoAlbum,
                    },
                    ContextMenuItem {
                        key: "W",
                        label: "Follow / unfollow artist".into(),
                        action: ContextMenuAction::ToggleArtistFollow,
                    },
                ];
                if on_artist_page {
                    items.push(ContextMenuItem {
                        key: "i",
                        label: "Read biography".into(),
                        action: ContextMenuAction::ShowArtistBio,
                    });
                }
                items.push(ContextMenuItem {
                    key: "yy",
                    label: "Copy artist URL".into(),
                    action: ContextMenuAction::CopyUrl,
                });
                (
                    ar.name,
                    ar.subtitle.unwrap_or_else(|| "Artist".to_string()),
                    items,
                )
            }
            SearchItem::Album(al) => {
                let mut items = vec![
                    ContextMenuItem {
                        key: "Enter",
                        label: "Open album".into(),
                        action: ContextMenuAction::GoAlbum,
                    },
                    ContextMenuItem {
                        key: "p",
                        label: "Play album".into(),
                        action: ContextMenuAction::PlayNow,
                    },
                    ContextMenuItem {
                        key: "x",
                        label: "Shuffle play".into(),
                        action: ContextMenuAction::ShufflePlay,
                    },
                    ContextMenuItem {
                        key: "r",
                        label: "Start album mix".into(),
                        action: ContextMenuAction::StartRadio,
                    },
                    ContextMenuItem {
                        key: "N",
                        label: "Play next".into(),
                        action: ContextMenuAction::PlayNext,
                    },
                    ContextMenuItem {
                        key: "q",
                        label: "Add to queue".into(),
                        action: ContextMenuAction::AddToQueue,
                    },
                ];
                if al.artists.iter().any(|a| a.id.is_some()) {
                    items.push(ContextMenuItem {
                        key: "A",
                        label: "Go to artist".into(),
                        action: ContextMenuAction::GoArtist,
                    });
                }
                items.push(ContextMenuItem {
                    key: "w",
                    label: "Save / remove album in Library".into(),
                    action: ContextMenuAction::ToggleSave,
                });
                items.push(ContextMenuItem {
                    key: "yy",
                    label: "Copy album URL".into(),
                    action: ContextMenuAction::CopyUrl,
                });
                let sub = al.subtitle();
                (al.title, sub, items)
            }
            SearchItem::Playlist(pl) => {
                let items = vec![
                    ContextMenuItem {
                        key: "x",
                        label: "Shuffle play".into(),
                        action: ContextMenuAction::ShufflePlay,
                    },
                    ContextMenuItem {
                        key: "r",
                        label: "Start mix".into(),
                        action: ContextMenuAction::StartRadio,
                    },
                    ContextMenuItem {
                        key: "N",
                        label: "Play next".into(),
                        action: ContextMenuAction::PlayNext,
                    },
                    ContextMenuItem {
                        key: "q",
                        label: "Add to queue".into(),
                        action: ContextMenuAction::AddToQueue,
                    },
                    ContextMenuItem {
                        key: "w",
                        label: "Save playlist to Library".into(),
                        action: ContextMenuAction::ToggleSave,
                    },
                    ContextMenuItem {
                        key: "P",
                        label: "Save copy to My Playlists".into(),
                        action: ContextMenuAction::SaveCopyToMyPlaylists,
                    },
                    ContextMenuItem {
                        key: "yy",
                        label: "Copy playlist URL".into(),
                        action: ContextMenuAction::CopyUrl,
                    },
                ];
                (
                    pl.title,
                    pl.author.unwrap_or_else(|| "Community playlist".into()),
                    items,
                )
            }
        };

        self.modal = Some(ActiveModal::ContextMenu {
            title,
            subtitle,
            items,
            cursor: 0,
        });
    }

    fn open_artist_for_track(&mut self, track: &Track) {
        if let Some(art) = track.artists.iter().find(|a| a.id.is_some())
            && let Some(ref bid) = art.id
        {
            self.apply_effect(ViewEffect::OpenArtist {
                browse_id: bid.clone(),
                name_hint: art.name.clone(),
            });
        } else {
            self.set_toast("No artist link available for this track");
        }
    }

    fn open_album_for_track(&mut self, track: &Track) {
        if let Some(ref bid) = track.album_id {
            self.apply_effect(ViewEffect::OpenAlbum {
                browse_id: bid.clone(),
                title_hint: track.album.clone().unwrap_or_else(|| "Album".to_string()),
            });
        } else {
            self.set_toast("No album link available for this track");
        }
    }

    pub(super) fn push_nav(&mut self, page: NavPage) {
        self.player_bar_focused = false;
        if self.nav_stack.len() >= MAX_NAV_DEPTH {
            self.nav_stack.remove(0);
        }
        self.nav_stack.push(page);
        self.sync_cava_state();
    }
}
