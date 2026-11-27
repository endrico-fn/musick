use crossterm::event::KeyEvent;

use musick_core::{AppCommand, Track};

use crate::keymap::{Action, InputMode};
use crate::views::ViewEffect;
use crate::widgets::{ActiveModal, RootTab};

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

    fn switch_root_tab(&mut self, tab: RootTab) {
        self.active_tab = tab;
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

        match action {
            Action::Quit => {
                self.should_quit = true;
                return;
            }
            Action::ToggleHelp => {
                self.modal = Some(ActiveModal::KeymapHelp { scroll: 0 });
                return;
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
                self.trigger_discover_refresh();
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
                if let Some(track) = self.active_hovered_track() {
                    self.open_artist_for_track(&track);
                } else {
                    self.set_toast("No track selected to jump to Artist");
                }
                return;
            }
            Action::GoAlbum => {
                if let Some(track) = self.active_hovered_track() {
                    self.open_album_for_track(&track);
                } else {
                    self.set_toast("No track selected to jump to Album");
                }
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

    fn active_hovered_track(&self) -> Option<Track> {
        if let Some(top) = self.nav_stack.last() {
            match top {
                NavPage::Artist(v) => v.hovered_track(),
                NavPage::Album(v) => v.hovered_track(),
                NavPage::PlaylistDetail(v) => v.hovered_track(),
            }
        } else {
            match self.active_tab {
                RootTab::Player => self.player.hovered_track(),
                RootTab::Playlists => self.playlists.hovered_track(),
                RootTab::Library => self.library.hovered_track(),
                RootTab::Queue => self.queue.hovered_track(&self.queue_snap),
                RootTab::Settings => None,
            }
        }
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
        if self.nav_stack.len() >= MAX_NAV_DEPTH {
            self.nav_stack.remove(0);
        }
        self.nav_stack.push(page);
        self.sync_cava_state();
    }
}
