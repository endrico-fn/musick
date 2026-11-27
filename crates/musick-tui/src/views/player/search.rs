use musick_core::{AppCommand, SearchCategory, SearchItem};

use crate::keymap::Action;
use crate::views::ViewEffect;

use super::discover::{activate_search_item, item_url, play_direct_item, save_effect_for_item};
use super::{PlayerPanelMode, PlayerView};

impl PlayerView {
    pub(super) fn handle_search_input_action(&mut self, action: Action) -> ViewEffect {
        match action {
            Action::InputDown => {
                if !self.suggestions.is_empty() {
                    self.suggest_cursor = Some(match self.suggest_cursor {
                        None => 0,
                        Some(i) => (i + 1).min(self.suggestions.len() - 1),
                    });
                }
                return ViewEffect::None;
            }
            Action::InputUp => {
                if let Some(i) = self.suggest_cursor {
                    self.suggest_cursor = if i == 0 { None } else { Some(i - 1) };
                }
                return ViewEffect::None;
            }
            Action::InputTab => {
                if let Some(idx) = self.suggest_cursor
                    && let Some(sug) = self.suggestions.get(idx)
                {
                    self.search_input.set_text(sug.text.clone());
                    self.suggest_cursor = None;
                    return ViewEffect::TriggerAutocomplete {
                        query: self.search_input.as_str().to_string(),
                    };
                }
                return ViewEffect::None;
            }
            Action::InputSubmit => {
                if let Some(idx) = self.suggest_cursor
                    && let Some(sug) = self.suggestions.get(idx)
                {
                    self.search_input.set_text(sug.text.clone());
                }
                let q = self.search_input.trim().to_string();
                if q.is_empty() {
                    self.mode = PlayerPanelMode::Discover;
                    return ViewEffect::None;
                }
                // sync typed search prefixes (`a:`, `al:`, `pl:`, `c:`, `v:`, `s:`) into category tab ui
                let (parsed_cat, stripped_q) =
                    musick_core::parse_query_prefix(&q, self.search_category);
                let final_query = if stripped_q != q && !stripped_q.is_empty() {
                    self.search_category = parsed_cat;
                    self.search_input.set_text(stripped_q.clone());
                    stripped_q
                } else {
                    q
                };
                self.mode = PlayerPanelMode::SearchResults;
                self.loading_search = true;
                self.search_error = None;
                self.search_results.clear();
                self.results_cursor = 0;
                self.results_scroll.reset();
                return ViewEffect::TriggerSearch {
                    category: self.search_category,
                    query: final_query,
                };
            }
            Action::InputCancel => {
                self.mode = if self.search_results.is_empty() {
                    PlayerPanelMode::Discover
                } else {
                    PlayerPanelMode::SearchResults
                };
                return ViewEffect::None;
            }
            _ => {}
        }

        let (handled, changed) = self.search_input.handle_action(&action);
        if handled && changed {
            self.suggest_cursor = None;
            return ViewEffect::TriggerAutocomplete {
                query: self.search_input.as_str().to_string(),
            };
        }
        ViewEffect::None
    }

    pub(super) fn handle_results_action(&mut self, action: Action) -> ViewEffect {
        match action {
            Action::NextSubTab => {
                let cats = SearchCategory::ALL;
                let idx = cats
                    .iter()
                    .position(|c| *c == self.search_category)
                    .unwrap_or(0);
                self.search_category = cats[(idx + 1) % cats.len()];
                self.loading_search = true;
                self.search_error = None;
                self.search_results.clear();
                self.results_cursor = 0;
                self.results_scroll.reset();
                ViewEffect::TriggerSearch {
                    category: self.search_category,
                    query: self.search_input.as_str().to_string(),
                }
            }
            Action::PrevSubTab => {
                let cats = SearchCategory::ALL;
                let idx = cats
                    .iter()
                    .position(|c| *c == self.search_category)
                    .unwrap_or(0);
                self.search_category = cats[(idx + cats.len() - 1) % cats.len()];
                self.loading_search = true;
                self.search_error = None;
                self.search_results.clear();
                self.results_cursor = 0;
                self.results_scroll.reset();
                ViewEffect::TriggerSearch {
                    category: self.search_category,
                    query: self.search_input.as_str().to_string(),
                }
            }
            Action::MoveDown => {
                if !self.search_results.is_empty() {
                    self.results_cursor =
                        (self.results_cursor + 1).min(self.search_results.len() - 1);
                }
                ViewEffect::None
            }
            Action::MoveUp => {
                self.results_cursor = self.results_cursor.saturating_sub(1);
                ViewEffect::None
            }
            Action::HalfPageDown => {
                if !self.search_results.is_empty() {
                    self.results_cursor =
                        (self.results_cursor + 8).min(self.search_results.len() - 1);
                }
                ViewEffect::None
            }
            Action::HalfPageUp => {
                self.results_cursor = self.results_cursor.saturating_sub(8);
                ViewEffect::None
            }
            Action::JumpTop => {
                self.results_cursor = 0;
                ViewEffect::None
            }
            Action::JumpBottom => {
                self.results_cursor = self.search_results.len().saturating_sub(1);
                ViewEffect::None
            }
            Action::Activate => {
                if let Some(item) = self.search_results.get(self.results_cursor) {
                    activate_search_item(item, &self.search_results, self.results_cursor)
                } else {
                    ViewEffect::None
                }
            }
            Action::PlayDirect => {
                if let Some(item) = self.search_results.get(self.results_cursor) {
                    play_direct_item(item)
                } else {
                    ViewEffect::None
                }
            }
            Action::PlayNext => {
                if let Some(SearchItem::Track(tr)) = self.search_results.get(self.results_cursor) {
                    ViewEffect::Command(AppCommand::PlayNext {
                        tracks: vec![tr.clone()],
                    })
                } else {
                    ViewEffect::None
                }
            }
            Action::AppendQueue => {
                if let Some(SearchItem::Track(tr)) = self.search_results.get(self.results_cursor) {
                    ViewEffect::Command(AppCommand::AppendTracks {
                        tracks: vec![tr.clone()],
                    })
                } else {
                    ViewEffect::None
                }
            }
            Action::AddToPlaylist => {
                if let Some(SearchItem::Track(tr)) = self.search_results.get(self.results_cursor) {
                    ViewEffect::AddToPlaylistPicker(tr.clone())
                } else {
                    ViewEffect::None
                }
            }
            Action::ToggleLike => {
                if let Some(SearchItem::Track(tr)) = self.search_results.get(self.results_cursor) {
                    ViewEffect::ToggleLike(tr.clone())
                } else {
                    ViewEffect::None
                }
            }
            Action::ToggleSave => {
                if let Some(item) = self.search_results.get(self.results_cursor) {
                    save_effect_for_item(item)
                } else {
                    ViewEffect::None
                }
            }
            Action::StartRadio => {
                if let Some(SearchItem::Track(tr)) = self.search_results.get(self.results_cursor) {
                    ViewEffect::Command(AppCommand::StartRadio {
                        video_id: Some(tr.video_id.clone()),
                        playlist_id: None,
                        seed_track: Some(tr.clone()),
                    })
                } else {
                    ViewEffect::None
                }
            }
            Action::CopyUrl => self
                .search_results
                .get(self.results_cursor)
                .map(|it| ViewEffect::CopyUrl(item_url(it)))
                .unwrap_or(ViewEffect::None),
            _ => ViewEffect::None,
        }
    }
}
