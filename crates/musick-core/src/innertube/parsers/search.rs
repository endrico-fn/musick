use std::collections::HashSet;

use serde_json::Value;

use crate::domain::{
    AlbumSummary, ArtistRef, ArtistSummary, PlaylistSummary, SearchItem, Track, search_item_key,
};
use crate::innertube::extract::{
    extract_navigation_browse_id, extract_navigation_page_type, extract_thumbs,
    extract_watch_video_id, extract_watch_video_type, find_all_by_key, get_array, get_str,
    parse_duration_str, runs_text, runs_vec,
};

use super::items::parse_any_item;
use super::{
    clean_plays_label, collect_append_continuation_items, extract_continuation_token,
    extract_response_continuation, flex_column_text_val, is_separator_run, looks_like_plays,
    looks_like_year,
};

pub fn parse_search_results(raw: &Value) -> Vec<SearchItem> {
    parse_search_results_with_continuation(raw).0
}

pub fn parse_search_results_with_continuation(raw: &Value) -> (Vec<SearchItem>, Option<String>) {
    let mut items = Vec::new();
    let mut seen: HashSet<(&'static str, String)> = HashSet::new();
    let mut continuation = None;

    let mut cards = Vec::new();
    find_all_by_key(raw, "musicCardShelfRenderer", &mut cards);
    for card in cards {
        if let Some(item) = parse_card_shelf_top(card) {
            let (k, id) = search_item_key(&item);
            if seen.insert((k, id.to_string())) {
                items.push(item);
            }
        }
        if let Some(contents) = get_array(card, &["contents"]) {
            for c in contents {
                if let Some(item) = parse_any_item(c) {
                    let (k, id) = search_item_key(&item);
                    if seen.insert((k, id.to_string())) {
                        items.push(item);
                    }
                }
            }
        }
    }

    let mut shelves = Vec::new();
    find_all_by_key(raw, "musicShelfRenderer", &mut shelves);
    find_all_by_key(raw, "musicShelfContinuation", &mut shelves);
    for shelf in shelves {
        if let Some(contents) = get_array(shelf, &["contents"]) {
            for c in contents {
                if let Some(item) = parse_any_item(c) {
                    let (k, id) = search_item_key(&item);
                    if seen.insert((k, id.to_string())) {
                        items.push(item);
                    }
                }
            }
        }
        if continuation.is_none() {
            continuation = extract_continuation_token(shelf);
        }
    }

    for c in collect_append_continuation_items(raw) {
        if let Some(item) = parse_any_item(c) {
            let (k, id) = search_item_key(&item);
            if seen.insert((k, id.to_string())) {
                items.push(item);
            }
        }
    }
    if continuation.is_none() {
        continuation = extract_response_continuation(raw);
    }

    (items, continuation)
}

pub fn parse_spelling_hint(raw: &Value) -> Option<String> {
    let mut nodes = Vec::new();
    find_all_by_key(raw, "didYouMeanRenderer", &mut nodes);
    find_all_by_key(raw, "showingResultsForRenderer", &mut nodes);
    for node in nodes {
        if let Some(q) = node
            .get("correctedQuery")
            .and_then(runs_text)
            .or_else(|| {
                get_str(node, &["correctedQueryEndpoint", "searchEndpoint", "query"])
                    .map(str::to_string)
            })
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            return Some(q);
        }
    }
    None
}

fn parse_card_shelf_top(card: &Value) -> Option<SearchItem> {
    let title_val = card.get("title")?;
    let title = runs_text(title_val)?;
    let thumbs = extract_thumbs(card);
    let sub_val = card.get("subtitle");
    let sub_text = sub_val.and_then(runs_text);
    let first_run = runs_vec(title_val).into_iter().next()?;

    if let Some(vid) = extract_watch_video_id(first_run) {
        let mut artists = Vec::new();
        let mut album = None;
        let mut album_id = None;
        let mut duration = None;
        let mut plays = None;
        let mut past_artists = false;
        let mut last_sep_artist = false;
        if let Some(sv) = sub_val {
            for r in runs_vec(sv) {
                let txt = r.get("text").and_then(Value::as_str).unwrap_or("").trim();
                if is_separator_run(txt) {
                    if matches!(txt, "," | "&") {
                        last_sep_artist = true;
                    } else if matches!(txt, "•" | "·") && !artists.is_empty() {
                        last_sep_artist = false;
                        past_artists = true;
                    }
                    continue;
                }
                if txt.eq_ignore_ascii_case("song") || txt.eq_ignore_ascii_case("video") {
                    continue;
                }
                if let Some(dur) = parse_duration_str(txt) {
                    duration = Some(dur);
                    past_artists = true;
                    continue;
                }
                if looks_like_plays(txt) {
                    plays = Some(clean_plays_label(txt));
                    past_artists = true;
                    continue;
                }
                if let Some(bid) = extract_navigation_browse_id(r) {
                    let pt = extract_navigation_page_type(r).unwrap_or_default();
                    if pt == "MUSIC_PAGE_TYPE_ALBUM" || bid.starts_with("MPRE") {
                        album = Some(txt.to_string());
                        album_id = Some(bid);
                        past_artists = true;
                    } else if pt == "MUSIC_PAGE_TYPE_ARTIST"
                        || pt == "MUSIC_PAGE_TYPE_USER_CHANNEL"
                        || bid.starts_with("UC")
                        || bid.starts_with("MPLA")
                    {
                        artists.push(ArtistRef {
                            name: txt.to_string(),
                            id: Some(bid.strip_prefix("MPLA").unwrap_or(&bid).to_string()),
                        });
                    }
                } else if !looks_like_year(txt)
                    && (artists.is_empty() || (!past_artists && last_sep_artist))
                {
                    artists.push(ArtistRef {
                        name: txt.to_string(),
                        id: None,
                    });
                }
            }
        }
        return Some(SearchItem::Track(Track {
            video_id: vid,
            title,
            artists,
            album,
            album_id,
            duration,
            thumbs,
            plays,
            video_type: extract_watch_video_type(first_run),
            set_video_id: None,
            is_available: true,
        }));
    }

    let bid = extract_navigation_browse_id(first_run)?;
    let pt = extract_navigation_page_type(first_run).unwrap_or_default();
    if pt == "MUSIC_PAGE_TYPE_ARTIST" || bid.starts_with("UC") || bid.starts_with("MPLA") {
        return Some(SearchItem::Artist(ArtistSummary {
            browse_id: bid.strip_prefix("MPLA").unwrap_or(&bid).to_string(),
            name: title,
            subtitle: sub_text,
            thumbs,
        }));
    }
    if pt == "MUSIC_PAGE_TYPE_ALBUM" || bid.starts_with("MPRE") {
        let mut artists = Vec::new();
        let mut year = None;
        let mut kind = "Album".to_string();
        if let Some(sv) = sub_val {
            for r in runs_vec(sv) {
                let txt = r.get("text").and_then(Value::as_str).unwrap_or("").trim();
                if is_separator_run(txt) {
                    continue;
                }
                if looks_like_year(txt) {
                    year = Some(txt.to_string());
                } else if matches!(txt, "Album" | "Single" | "EP") {
                    kind = txt.to_string();
                } else if let Some(art_bid) = extract_navigation_browse_id(r) {
                    artists.push(ArtistRef {
                        name: txt.to_string(),
                        id: Some(art_bid.strip_prefix("MPLA").unwrap_or(&art_bid).to_string()),
                    });
                } else if !txt.eq_ignore_ascii_case("album") {
                    artists.push(ArtistRef {
                        name: txt.to_string(),
                        id: None,
                    });
                }
            }
        }
        return Some(SearchItem::Album(AlbumSummary {
            browse_id: bid,
            title,
            year,
            kind,
            artists,
            thumbs,
        }));
    }
    let mut author = None;
    let mut count = None;
    if let Some(sv) = sub_val {
        for r in runs_vec(sv) {
            let txt = r.get("text").and_then(Value::as_str).unwrap_or("").trim();
            if is_separator_run(txt) || txt.eq_ignore_ascii_case("playlist") {
                continue;
            }
            let lower = txt.to_ascii_lowercase();
            if lower.contains("song") || lower.contains("track") || lower.contains("view") {
                count = Some(txt.to_string());
            } else if author.is_none() {
                author = Some(txt.to_string());
            }
        }
    }
    Some(SearchItem::Playlist(PlaylistSummary {
        browse_id: bid.strip_prefix("VL").unwrap_or(&bid).to_string(),
        title,
        author: author.or(sub_text),
        count,
        thumbs,
    }))
}

pub fn parse_suggestions(raw: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut renderers = Vec::new();
    find_all_by_key(raw, "searchSuggestionRenderer", &mut renderers);
    for r in renderers {
        if let Some(q) = get_str(r, &["navigationEndpoint", "searchEndpoint", "query"])
            .map(str::to_string)
            .or_else(|| r.get("suggestion").and_then(runs_text))
            && !q.trim().is_empty()
            && !out.contains(&q)
        {
            out.push(q);
        }
    }
    let mut entity_rows = Vec::new();
    find_all_by_key(raw, "musicResponsiveListItemRenderer", &mut entity_rows);
    for row in entity_rows {
        if let Some(title) = flex_column_text_val(row, 0).and_then(runs_text) {
            let clean = title.trim().to_string();
            if !clean.is_empty() && !out.iter().any(|s| s.eq_ignore_ascii_case(&clean)) {
                out.push(clean);
            }
        }
    }
    out
}
