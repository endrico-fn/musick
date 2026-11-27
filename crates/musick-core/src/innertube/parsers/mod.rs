mod artist;
mod collection;
mod feed;
mod items;
mod search;
mod watch;

use serde_json::Value;

use crate::innertube::extract::{find_all_by_key, get_array, get_path, get_str, runs_text};

pub use artist::{
    extract_plays_from_browse, parse_artist, parse_artist_releases,
    parse_artist_releases_with_continuation,
};
pub use collection::{
    parse_album, parse_library_albums, parse_library_albums_with_continuation,
    parse_library_artists, parse_library_artists_with_continuation, parse_library_playlists,
    parse_library_playlists_with_continuation, parse_playlist, parse_playlist_edit_results,
};
pub use feed::parse_discover_feed;
pub use items::{parse_responsive_list_item, parse_two_row_item};
pub use search::{
    parse_search_results, parse_search_results_with_continuation, parse_spelling_hint,
    parse_suggestions,
};
pub use watch::{parse_player_tracking, parse_watch_playlist, parse_yt_lyrics};

pub fn extract_visitor_data(raw: &Value) -> Option<String> {
    get_str(raw, &["responseContext", "visitorData"])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(super) fn is_separator_run(text: &str) -> bool {
    let t = text.trim();
    t.is_empty() || matches!(t, "•" | "·" | "," | "&" | "-" | "|")
}

pub(super) fn looks_like_plays(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    lower.contains("play") || lower.contains("view") || lower.contains("putar")
}

pub(super) fn clean_plays_label(text: &str) -> String {
    let trimmed = text.trim();
    trimmed
        .split_whitespace()
        .next()
        .unwrap_or(trimmed)
        .to_string()
}

pub(super) fn looks_like_year(text: &str) -> bool {
    let t = text.trim();
    t.len() == 4 && t.chars().all(|c| c.is_ascii_digit())
}

pub(super) fn extract_set_video_id(item: &Value) -> Option<String> {
    if let Some(sv) = get_str(item, &["playlistItemData", "playlistSetVideoId"]) {
        return Some(sv.to_string());
    }
    if let Some(sv) = get_str(item, &["playlistSetVideoId"]) {
        return Some(sv.to_string());
    }
    let mut actions = Vec::new();
    find_all_by_key(item, "setVideoId", &mut actions);
    actions
        .into_iter()
        .filter_map(Value::as_str)
        .find(|s| !s.is_empty())
        .map(str::to_string)
}

pub(super) fn flex_column_text_val(item: &Value, idx: usize) -> Option<&Value> {
    let cols = get_array(item, &["flexColumns"])?;
    let col = cols.get(idx)?;
    get_path(col, &["musicResponsiveListItemFlexColumnRenderer", "text"])
}

pub(super) fn fixed_column_text(item: &Value, idx: usize) -> Option<String> {
    let cols = get_array(item, &["fixedColumns"])?;
    let col = cols.get(idx)?;
    let txt_val = get_path(col, &["musicResponsiveListItemFixedColumnRenderer", "text"])?;
    runs_text(txt_val)
}

pub(super) fn collect_append_continuation_items(raw: &Value) -> Vec<&Value> {
    let mut out = Vec::new();
    let mut actions = Vec::new();
    find_all_by_key(raw, "appendContinuationItemsAction", &mut actions);
    for act in actions {
        if let Some(items) = get_array(act, &["continuationItems"]) {
            out.extend(items.iter());
        }
    }
    out
}

pub(super) fn extract_action_continuation_token(raw: &Value) -> Option<String> {
    let mut actions = Vec::new();
    find_all_by_key(raw, "appendContinuationItemsAction", &mut actions);
    for act in actions {
        if let Some(tok) = extract_continuation_token(act) {
            return Some(tok);
        }
    }
    None
}

pub(super) fn extract_response_continuation(raw: &Value) -> Option<String> {
    if let Some(tok) = extract_action_continuation_token(raw) {
        return Some(tok);
    }
    if let Some(cc) = raw.get("continuationContents")
        && let Some(tok) = extract_continuation_token(cc)
    {
        return Some(tok);
    }
    let mut containers = Vec::new();
    find_all_by_key(raw, "gridRenderer", &mut containers);
    find_all_by_key(raw, "musicShelfRenderer", &mut containers);
    find_all_by_key(raw, "sectionListRenderer", &mut containers);
    for node in containers {
        if let Some(tok) = extract_continuation_token(node) {
            return Some(tok);
        }
    }
    None
}

pub(super) fn extract_continuation_token(node: &Value) -> Option<String> {
    if let Some(conts) = get_array(node, &["continuations"]) {
        for c in conts {
            if let Some(tok) = get_str(c, &["nextContinuationData", "continuation"])
                .or_else(|| get_str(c, &["nextRadioContinuationData", "continuation"]))
            {
                return Some(tok.to_string());
            }
        }
    }
    let mut items = Vec::new();
    find_all_by_key(node, "continuationItemRenderer", &mut items);
    for item in items {
        if let Some(tok) = get_str(
            item,
            &["continuationEndpoint", "continuationCommand", "token"],
        ) {
            return Some(tok.to_string());
        }
    }
    None
}
