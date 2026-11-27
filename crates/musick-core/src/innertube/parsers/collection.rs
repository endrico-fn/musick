use std::collections::HashSet;

use serde_json::Value;

use crate::domain::{
    Album, AlbumSummary, ArtistRef, ArtistSummary, Playlist, PlaylistSummary, SearchItem,
};
use crate::innertube::endpoints::PlaylistEditResult;
use crate::innertube::extract::{
    extract_navigation_browse_id, extract_thumbs, find_all_by_key, get_array, get_path, get_str,
    runs_text, runs_vec,
};

use super::items::{parse_any_item, parse_responsive_list_item, parse_two_row_item};
use super::{
    collect_append_continuation_items, extract_continuation_token, extract_response_continuation,
    is_separator_run, looks_like_year,
};

pub fn parse_album(browse_id: &str, raw: &Value) -> Album {
    let header = get_path(raw, &["header", "musicDetailHeaderRenderer"]).or_else(|| {
        let mut resp_headers = Vec::new();
        find_all_by_key(raw, "musicResponsiveHeaderRenderer", &mut resp_headers);
        resp_headers.into_iter().next()
    });

    let title = header
        .and_then(|h| h.get("title"))
        .and_then(runs_text)
        .unwrap_or_else(|| "Album".to_string());
    let thumbs = header.map(extract_thumbs).unwrap_or_default();
    let description = header
        .and_then(|h| {
            h.get("description").or_else(|| {
                get_path(
                    h,
                    &[
                        "description",
                        "musicDescriptionShelfRenderer",
                        "description",
                    ],
                )
            })
        })
        .and_then(runs_text);
    let duration_text = header
        .and_then(|h| h.get("secondSubtitle"))
        .and_then(runs_text);

    let mut artists = Vec::new();
    let mut year = None;
    let mut kind = "Album".to_string();
    if let Some(h) = header {
        for field_name in ["straplineTextOne", "subtitle"] {
            if let Some(v) = h.get(field_name) {
                for r in runs_vec(v) {
                    let txt = r.get("text").and_then(Value::as_str).unwrap_or("").trim();
                    if is_separator_run(txt) {
                        continue;
                    }
                    if looks_like_year(txt) {
                        year = Some(txt.to_string());
                    } else if matches!(txt, "Album" | "Single" | "EP") {
                        kind = txt.to_string();
                    } else if let Some(bid) = extract_navigation_browse_id(r) {
                        artists.push(ArtistRef {
                            name: txt.to_string(),
                            id: Some(bid),
                        });
                    } else if field_name == "straplineTextOne" && artists.is_empty() {
                        artists.push(ArtistRef {
                            name: txt.to_string(),
                            id: None,
                        });
                    }
                }
            }
        }
    }

    let mut tracks = Vec::new();
    let mut shelves = Vec::new();
    find_all_by_key(raw, "musicShelfRenderer", &mut shelves);
    for shelf in shelves {
        if let Some(contents) = get_array(shelf, &["contents"]) {
            for c in contents {
                if let Some(SearchItem::Track(mut tr)) = parse_any_item(c) {
                    if tr.artists.is_empty() {
                        tr.artists = artists.clone();
                    }
                    if tr.album.is_none() {
                        tr.album = Some(title.clone());
                    }
                    if tr.album_id.is_none() {
                        tr.album_id = Some(browse_id.to_string());
                    }
                    if tr.thumbs.is_empty() {
                        tr.thumbs = thumbs.clone();
                    }
                    tracks.push(tr);
                }
            }
        }
    }

    let mut playlist_ids = Vec::new();
    find_all_by_key(raw, "playlistId", &mut playlist_ids);
    let audio_playlist_id = playlist_ids
        .into_iter()
        .filter_map(Value::as_str)
        .find(|s| s.starts_with("OLAK"))
        .map(str::to_string);

    Album {
        summary: AlbumSummary {
            browse_id: browse_id.to_string(),
            title,
            year,
            kind,
            artists,
            thumbs,
        },
        tracks,
        description,
        duration_text,
        audio_playlist_id,
    }
}

pub fn parse_playlist(playlist_id: &str, raw: &Value) -> (Playlist, Option<String>) {
    let header = get_path(raw, &["header", "musicDetailHeaderRenderer"])
        .or_else(|| {
            get_path(
                raw,
                &[
                    "header",
                    "musicEditablePlaylistDetailHeaderRenderer",
                    "header",
                    "musicDetailHeaderRenderer",
                ],
            )
        })
        .or_else(|| {
            let mut resp_headers = Vec::new();
            find_all_by_key(raw, "musicResponsiveHeaderRenderer", &mut resp_headers);
            resp_headers.into_iter().next()
        });

    let title = header
        .and_then(|h| h.get("title"))
        .and_then(runs_text)
        .unwrap_or_else(|| "Playlist".to_string());
    let thumbs = header.map(extract_thumbs).unwrap_or_default();
    let author = header.and_then(|h| {
        h.get("straplineTextOne")
            .and_then(runs_text)
            .or_else(|| {
                get_str(h, &["facepile", "avatarStackViewModel", "text", "content"])
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
            .or_else(|| {
                get_str(
                    h,
                    &[
                        "facepile",
                        "avatarStackViewModel",
                        "rendererContext",
                        "accessibilityContext",
                        "label",
                    ],
                )
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
            })
            .or_else(|| {
                let sub = h.get("subtitle").and_then(runs_text)?;
                if sub.to_ascii_lowercase().starts_with("playlist") {
                    None
                } else {
                    Some(sub)
                }
            })
    });
    let description = header
        .and_then(|h| {
            h.get("description").or_else(|| {
                get_path(
                    h,
                    &[
                        "description",
                        "musicDescriptionShelfRenderer",
                        "description",
                    ],
                )
            })
        })
        .and_then(runs_text);
    let mut views = None;
    let mut count = None;
    let mut duration_text = None;
    let mut year = None;
    if let Some(sub) = header
        .and_then(|h| h.get("secondSubtitle"))
        .and_then(runs_text)
    {
        for raw_part in sub.split(['•', '·']) {
            let part = raw_part.trim();
            if part.is_empty() {
                continue;
            }
            let lower = part.to_ascii_lowercase();
            if lower.contains("view") {
                views = Some(part.to_string());
            } else if lower.contains("song") || lower.contains("track") {
                count = Some(part.to_string());
            } else if lower.contains("hour")
                || lower.contains("hr")
                || lower.contains("min")
                || lower.contains("sec")
                || part.contains(':')
            {
                duration_text = Some(part.to_string());
            } else if looks_like_year(part) {
                year = Some(part.to_string());
            } else if duration_text.is_none() {
                duration_text = Some(part.to_string());
            }
        }
    }

    let mut tracks = Vec::new();
    let mut shelves = Vec::new();
    find_all_by_key(raw, "musicPlaylistShelfRenderer", &mut shelves);
    find_all_by_key(raw, "musicShelfRenderer", &mut shelves);

    let mut continuation = None;
    for shelf in shelves {
        if let Some(contents) = get_array(shelf, &["contents"]) {
            for c in contents {
                if let Some(SearchItem::Track(tr)) = parse_any_item(c) {
                    tracks.push(tr);
                }
            }
        }
        if continuation.is_none() {
            continuation = extract_continuation_token(shelf);
        }
    }

    if let Some(cont_shelf) = get_path(
        raw,
        &["continuationContents", "musicPlaylistShelfContinuation"],
    ) {
        if let Some(contents) = get_array(cont_shelf, &["contents"]) {
            for c in contents {
                if let Some(SearchItem::Track(tr)) = parse_any_item(c) {
                    tracks.push(tr);
                }
            }
        }
        continuation = extract_continuation_token(cont_shelf);
    }

    for c in collect_append_continuation_items(raw) {
        if let Some(SearchItem::Track(tr)) = parse_any_item(c) {
            tracks.push(tr);
        }
    }
    if continuation.is_none() {
        continuation = extract_response_continuation(raw);
    }

    let total = Some(tracks.len());
    let playlist = Playlist {
        summary: PlaylistSummary {
            browse_id: playlist_id
                .strip_prefix("VL")
                .unwrap_or(playlist_id)
                .to_string(),
            title,
            author,
            count,
            thumbs,
        },
        tracks,
        description,
        views,
        duration_text,
        year,
        total,
    };
    (playlist, continuation)
}

pub fn parse_library_playlists(raw: &Value) -> Vec<PlaylistSummary> {
    parse_library_playlists_with_continuation(raw).0
}

pub fn parse_library_playlists_with_continuation(
    raw: &Value,
) -> (Vec<PlaylistSummary>, Option<String>) {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut two_rows = Vec::new();
    find_all_by_key(raw, "musicTwoRowItemRenderer", &mut two_rows);
    for node in two_rows {
        if let Some(SearchItem::Playlist(pl)) = parse_two_row_item(node) {
            if matches!(pl.browse_id.as_str(), "LM" | "SE") {
                continue;
            }
            if seen.insert(pl.browse_id.clone()) {
                out.push(pl);
            }
        }
    }
    (out, extract_response_continuation(raw))
}

pub fn parse_library_albums(raw: &Value) -> Vec<AlbumSummary> {
    parse_library_albums_with_continuation(raw).0
}

pub fn parse_library_albums_with_continuation(raw: &Value) -> (Vec<AlbumSummary>, Option<String>) {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut two_rows = Vec::new();
    find_all_by_key(raw, "musicTwoRowItemRenderer", &mut two_rows);
    for node in two_rows {
        if let Some(SearchItem::Album(al)) = parse_two_row_item(node)
            && seen.insert(al.browse_id.clone())
        {
            out.push(al);
        }
    }
    (out, extract_response_continuation(raw))
}

pub fn parse_library_artists(raw: &Value) -> Vec<ArtistSummary> {
    parse_library_artists_with_continuation(raw).0
}

pub fn parse_library_artists_with_continuation(
    raw: &Value,
) -> (Vec<ArtistSummary>, Option<String>) {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    find_all_by_key(raw, "musicResponsiveListItemRenderer", &mut rows);
    for node in rows {
        if let Some(SearchItem::Artist(ar)) = parse_responsive_list_item(node)
            && seen.insert(ar.browse_id.clone())
        {
            out.push(ar);
        }
    }
    let mut two_rows = Vec::new();
    find_all_by_key(raw, "musicTwoRowItemRenderer", &mut two_rows);
    for node in two_rows {
        if let Some(SearchItem::Artist(ar)) = parse_two_row_item(node)
            && seen.insert(ar.browse_id.clone())
        {
            out.push(ar);
        }
    }
    (out, extract_response_continuation(raw))
}

pub fn parse_playlist_edit_results(raw: &Value) -> PlaylistEditResult {
    let mut added_set_video_ids = Vec::new();
    if let Some(arr) = get_array(raw, &["playlistEditResults"]) {
        for item in arr {
            if let Some(added) = item.get("playlistEditVideoAddedResultData")
                && let (Some(vid), Some(svid)) = (
                    added.get("videoId").and_then(Value::as_str),
                    added.get("setVideoId").and_then(Value::as_str),
                )
            {
                added_set_video_ids.push((vid.to_string(), svid.to_string()));
            }
        }
    }
    PlaylistEditResult {
        added_set_video_ids,
    }
}
