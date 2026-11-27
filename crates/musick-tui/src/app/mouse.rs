use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use musick_core::AppCommand;

use crate::keymap::{Action, InputMode};
use crate::widgets::{PlayerBarControl, rect_contains};

use super::App;

impl App {
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.modal.is_some() {
            match mouse.kind {
                MouseEventKind::ScrollDown => self.handle_modal_action(Action::InputDown),
                MouseEventKind::ScrollUp => self.handle_modal_action(Action::InputUp),
                _ => {}
            }
            return;
        }

        match mouse.kind {
            MouseEventKind::Up(MouseButton::Left) => {
                self.dragging_seekbar = false;
                self.dragging_volume = false;
            }
            MouseEventKind::Down(MouseButton::Right) => {
                if self.input_mode() == InputMode::Normal {
                    self.dispatch_action(Action::OpenContextMenu);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                for &(tab, rect) in &self.topbar_tab_hitboxes {
                    if rect_contains(rect, mouse.column, mouse.row) {
                        self.player_bar_focused = false;
                        self.switch_root_tab(tab);
                        return;
                    }
                }

                let hit = self.player_bar_hitboxes;
                if rect_contains(hit.seekbar, mouse.column, mouse.row) {
                    self.player_bar_focused = true;
                    self.player_bar_control = PlayerBarControl::Seekbar;
                    self.dragging_seekbar = true;
                    self.seek_at_column(mouse.column);
                    return;
                }
                if rect_contains(hit.volume_bar, mouse.column, mouse.row) {
                    self.player_bar_focused = true;
                    self.player_bar_control = PlayerBarControl::Volume;
                    self.dragging_volume = true;
                    self.set_volume_at_column(mouse.column);
                    return;
                }
                if rect_contains(hit.volume_mute, mouse.column, mouse.row) {
                    self.player_bar_focused = true;
                    self.player_bar_control = PlayerBarControl::Volume;
                    self.services.engine.try_send(AppCommand::ToggleMute);
                    return;
                }
                if rect_contains(hit.prev, mouse.column, mouse.row) {
                    self.player_bar_focused = true;
                    self.player_bar_control = PlayerBarControl::Prev;
                    self.services.engine.try_send(AppCommand::Previous);
                    return;
                }
                if rect_contains(hit.play_pause, mouse.column, mouse.row) {
                    self.player_bar_focused = true;
                    self.player_bar_control = PlayerBarControl::PlayPause;
                    self.services.engine.try_send(AppCommand::TogglePause);
                    return;
                }
                if rect_contains(hit.next, mouse.column, mouse.row) {
                    self.player_bar_focused = true;
                    self.player_bar_control = PlayerBarControl::Next;
                    self.services.engine.try_send(AppCommand::Next);
                    return;
                }
                if rect_contains(hit.shuffle, mouse.column, mouse.row) {
                    self.player_bar_focused = true;
                    self.player_bar_control = PlayerBarControl::Shuffle;
                    self.services.engine.try_send(AppCommand::ToggleShuffle);
                    return;
                }
                if rect_contains(hit.repeat, mouse.column, mouse.row) {
                    self.player_bar_focused = true;
                    self.player_bar_control = PlayerBarControl::Repeat;
                    self.services.engine.try_send(AppCommand::CycleRepeat);
                    return;
                }
                if rect_contains(hit.bar_area, mouse.column, mouse.row) {
                    self.player_bar_focused = true;
                    return;
                }
                self.player_bar_focused = false;
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.dragging_seekbar {
                    self.seek_at_column(mouse.column);
                } else if self.dragging_volume {
                    self.set_volume_at_column(mouse.column);
                }
            }
            MouseEventKind::ScrollUp => {
                let hit = self.player_bar_hitboxes;
                if rect_contains(hit.seekbar, mouse.column, mouse.row) {
                    self.services
                        .engine
                        .try_send(AppCommand::SeekRelative { seconds: 5.0 });
                } else if rect_contains(hit.volume_bar, mouse.column, mouse.row)
                    || rect_contains(hit.volume_mute, mouse.column, mouse.row)
                {
                    self.services
                        .engine
                        .try_send(AppCommand::AdjustVolume { delta: 5 });
                } else if self.input_mode() == InputMode::Normal {
                    self.dispatch_action(Action::MoveUp);
                }
            }
            MouseEventKind::ScrollDown => {
                let hit = self.player_bar_hitboxes;
                if rect_contains(hit.seekbar, mouse.column, mouse.row) {
                    self.services
                        .engine
                        .try_send(AppCommand::SeekRelative { seconds: -5.0 });
                } else if rect_contains(hit.volume_bar, mouse.column, mouse.row)
                    || rect_contains(hit.volume_mute, mouse.column, mouse.row)
                {
                    self.services
                        .engine
                        .try_send(AppCommand::AdjustVolume { delta: -5 });
                } else if self.input_mode() == InputMode::Normal {
                    self.dispatch_action(Action::MoveDown);
                }
            }
            _ => {}
        }
    }

    fn seek_at_column(&self, col: u16) {
        let bar = self.player_bar_hitboxes.seekbar;
        if bar.width < 2 {
            return;
        }
        let dur = if self.position.duration_secs > 0.0 {
            self.position.duration_secs
        } else {
            self.snapshot
                .track
                .as_ref()
                .and_then(|t| t.duration)
                .map(f64::from)
                .unwrap_or(0.0)
        };
        if dur <= 0.0 {
            return;
        }
        let offset = col.saturating_sub(bar.x).min(bar.width - 1);
        let ratio = f64::from(offset) / f64::from(bar.width - 1);
        self.services.engine.try_send(AppCommand::SeekAbsolute {
            seconds: (ratio * dur).clamp(0.0, dur),
        });
    }

    fn set_volume_at_column(&self, col: u16) {
        let bar = self.player_bar_hitboxes.volume_bar;
        if bar.width < 2 {
            return;
        }
        let offset = col.saturating_sub(bar.x).min(bar.width - 1);
        let ratio = f64::from(offset) / f64::from(bar.width - 1);
        let vol = (ratio * 100.0).round().clamp(0.0, 100.0) as u8;
        self.services
            .engine
            .try_send(AppCommand::SetVolume { volume: vol });
    }

    /// handle keyboard cursor navigation inside the now playing bar; returns true if consumed
    pub(super) fn handle_player_bar_action(&mut self, action: &Action) -> bool {
        if !self.player_bar_focused {
            return false;
        }
        match action {
            Action::Escape | Action::FocusPaneUp => {
                self.player_bar_focused = false;
                true
            }
            Action::NextSubTab => {
                self.player_bar_control = self.player_bar_control.next();
                true
            }
            Action::PrevSubTab => {
                self.player_bar_control = self.player_bar_control.prev();
                true
            }
            Action::MoveDown => {
                self.player_bar_control = PlayerBarControl::Seekbar;
                true
            }
            Action::MoveUp => {
                if self.player_bar_control == PlayerBarControl::Seekbar {
                    self.player_bar_control = PlayerBarControl::PlayPause;
                } else {
                    self.player_bar_focused = false;
                }
                true
            }
            Action::MoveLeft => {
                match self.player_bar_control {
                    PlayerBarControl::Seekbar => {
                        self.services
                            .engine
                            .try_send(AppCommand::SeekRelative { seconds: -5.0 });
                    }
                    PlayerBarControl::Volume => {
                        self.services
                            .engine
                            .try_send(AppCommand::AdjustVolume { delta: -5 });
                    }
                    ctrl => self.player_bar_control = ctrl.prev(),
                }
                true
            }
            Action::MoveRight => {
                match self.player_bar_control {
                    PlayerBarControl::Seekbar => {
                        self.services
                            .engine
                            .try_send(AppCommand::SeekRelative { seconds: 5.0 });
                    }
                    PlayerBarControl::Volume => {
                        self.services
                            .engine
                            .try_send(AppCommand::AdjustVolume { delta: 5 });
                    }
                    ctrl => self.player_bar_control = ctrl.next(),
                }
                true
            }
            Action::JumpTop | Action::CenterLyrics
                if self.player_bar_control == PlayerBarControl::Seekbar =>
            {
                self.services
                    .engine
                    .try_send(AppCommand::SeekAbsolute { seconds: 0.0 });
                true
            }
            Action::HalfPageUp if self.player_bar_control == PlayerBarControl::Seekbar => {
                self.services
                    .engine
                    .try_send(AppCommand::SeekRelative { seconds: -30.0 });
                true
            }
            Action::HalfPageDown if self.player_bar_control == PlayerBarControl::Seekbar => {
                self.services
                    .engine
                    .try_send(AppCommand::SeekRelative { seconds: 30.0 });
                true
            }
            Action::Activate => {
                match self.player_bar_control {
                    PlayerBarControl::Prev => {
                        self.services.engine.try_send(AppCommand::Previous);
                    }
                    PlayerBarControl::PlayPause | PlayerBarControl::Seekbar => {
                        self.services.engine.try_send(AppCommand::TogglePause);
                    }
                    PlayerBarControl::Next => {
                        self.services.engine.try_send(AppCommand::Next);
                    }
                    PlayerBarControl::Shuffle => {
                        self.services.engine.try_send(AppCommand::ToggleShuffle);
                    }
                    PlayerBarControl::Repeat => {
                        self.services.engine.try_send(AppCommand::CycleRepeat);
                    }
                    PlayerBarControl::Volume => {
                        self.services.engine.try_send(AppCommand::ToggleMute);
                    }
                }
                true
            }
            _ => false,
        }
    }
}
