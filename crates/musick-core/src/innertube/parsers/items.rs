use serde_json::Value;

use crate::domain::{AlbumSummary, ArtistRef, ArtistSummary, PlaylistSummary, SearchItem, Track};
use crate::innertube::extract::{
    extract_navigation_browse_id, extract_navigation_page_type, extract_thumbs,
    extract_watch_video_id, extract_watch_video_type, get_path, parse_duration_str, runs_text,
    runs_vec,
};

use super::{
    clean_plays_label, extract_set_video_id, fixed_column_text, flex_column_text_val,
    is_separator_run, looks_like_plays, looks_like_year,
};

pub fn parse_responsive_list_item(item: &Value) -> Option<SearchItem> {
    let title_val = flex_column_text_val(item, 0)?;
    let title = runs_text(title_val)?;
    let thumbs = extract_thumbs(item);

    let first_run = runs_vec(title_val).into_iter().next();
    let overlay_play_ep = get_path(
        item,
        &[
            "overlay",
            "musicItemThumbnailOverlayRenderer",
            "content",
            "musicPlayButtonRenderer",
            "playNavigationEndpoint",
        ],
    );

    let video_id = extract_watch_video_id(item)
        .or_else(|| first_run.and_then(extract_watch_video_id))
        .or_else(|| overlay_play_ep.and_then(extract_watch_video_id));

    let browse_id = extract_navigation_browse_id(item)
        .or_else(|| first_run.and_then(extract_navigation_browse_id));
    let page_type = extract_navigation_page_type(item)
        .or_else(|| first_run.and_then(extract_navigation_page_type));

    let subtitle_val = flex_column_text_val(item, 1);
    let col2_val = flex_column_text_val(item, 2);

    if video_id.is_none()
        && let Some(bid) = browse_id
    {
        let sub_text = subtitle_val.and_then(runs_text);
        let pt = page_type.as_deref().unwrap_or("");
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
            if let Some(sv) = subtitle_val {
                for r in runs_vec(sv) {
                    let txt = r.get("text").and_then(Value::as_str).unwrap_or("").trim();
                    if is_separator_run(txt) {
                        continue;
                    }
                    if let Some(art_bid) = extract_navigation_browse_id(r) {
                        artists.push(ArtistRef {
                            name: txt.to_string(),
                            id: Some(art_bid),
                        });
                    } else if looks_like_year(txt) {
                        year = Some(txt.to_string());
                    } else if matches!(txt, "Album" | "Single" | "EP") {
                        kind = txt.to_string();
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
        if pt == "MUSIC_PAGE_TYPE_PLAYLIST"
            || bid.starts_with("VL")
            || bid.starts_with("PL")
            || bid.starts_with("RDCLAK")
            || bid.starts_with("OLAK")
        {
            let mut author = None;
            let mut count = None;
            if let Some(sv) = subtitle_val {
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
            return Some(SearchItem::Playlist(PlaylistSummary {
                browse_id: bid.strip_prefix("VL").unwrap_or(&bid).to_string(),
                title,
                author,
                count,
                thumbs,
            }));
        }
    }

    let vid = video_id?;
    let video_type = extract_watch_video_type(item)
        .or_else(|| first_run.and_then(extract_watch_video_type))
        .or_else(|| overlay_play_ep.and_then(extract_watch_video_type));

    let mut artists = Vec::new();
    let mut album = None;
    let mut album_id = None;
    let mut duration = fixed_column_text(item, 0).and_then(|s| parse_duration_str(&s));
    let mut plays = None;

    for (col_idx, col_val) in [subtitle_val, col2_val].into_iter().enumerate() {
        let Some(col_val) = col_val else {
            continue;
        };
        let mut past_artists_segment = col_idx > 0;
        let mut last_sep_was_artist_join = false;
        for r in runs_vec(col_val) {
            let txt = r.get("text").and_then(Value::as_str).unwrap_or("").trim();
            if is_separator_run(txt) {
                if matches!(txt, "," | "&") {
                    last_sep_was_artist_join = true;
                } else if matches!(txt, "•" | "·") && !artists.is_empty() {
                    last_sep_was_artist_join = false;
                    past_artists_segment = true;
                }
                continue;
            }
            if txt.eq_ignore_ascii_case("song")
                || txt.eq_ignore_ascii_case("video")
                || txt.eq_ignore_ascii_case("episode")
            {
                continue;
            }
            if let Some(dur) = parse_duration_str(txt) {
                if duration.is_none() {
                    duration = Some(dur);
                }
                past_artists_segment = true;
                continue;
            }
            if looks_like_plays(txt) {
                plays = Some(clean_plays_label(txt));
                past_artists_segment = true;
                continue;
            }
            if let Some(bid) = extract_navigation_browse_id(r) {
                let pt = extract_navigation_page_type(r).unwrap_or_default();
                if pt == "MUSIC_PAGE_TYPE_ALBUM" || bid.starts_with("MPRE") {
                    album = Some(txt.to_string());
                    album_id = Some(bid);
                    past_artists_segment = true;
                } else if pt == "MUSIC_PAGE_TYPE_ARTIST"
                    || pt == "MUSIC_PAGE_TYPE_USER_CHANNEL"
                    || bid.starts_with("UC")
                    || bid.starts_with("MPLA")
                {
                    let clean = bid.strip_prefix("MPLA").unwrap_or(&bid).to_string();
                    artists.push(ArtistRef {
                        name: txt.to_string(),
                        id: Some(clean),
                    });
                }
            } else if !looks_like_year(txt)
                && (artists.is_empty() || (!past_artists_segment && last_sep_was_artist_join))
            {
                artists.push(ArtistRef {
                    name: txt.to_string(),
                    id: None,
                });
            }
        }
    }

    let is_available = item
        .get("musicItemRendererDisplayPolicy")
        .and_then(Value::as_str)
        != Some("MUSIC_ITEM_RENDERER_DISPLAY_POLICY_GREY_OUT");

    Some(SearchItem::Track(Track {
        video_id: vid,
        title,
        artists,
        album,
        album_id,
        duration,
        thumbs,
        plays,
        video_type,
        set_video_id: extract_set_video_id(item),
        is_available,
    }))
}

pub fn parse_two_row_item(item: &Value) -> Option<SearchItem> {
    let title = item.get("title").and_then(runs_text)?;
    let thumbs = extract_thumbs(item);
    let sub_val = item.get("subtitle");
    let sub_text = sub_val.and_then(runs_text);

    if let Some(vid) = extract_watch_video_id(item) {
        let mut artists = Vec::new();
        let mut plays = None;
        if let Some(sv) = sub_val {
            for r in runs_vec(sv) {
                let txt = r.get("text").and_then(Value::as_str).unwrap_or("").trim();
                if is_separator_run(txt)
                    || txt.eq_ignore_ascii_case("song")
                    || txt.eq_ignore_ascii_case("video")
                {
                    continue;
                }
                if looks_like_plays(txt) {
                    plays = Some(clean_plays_label(txt));
                    continue;
                }
                let bid = extract_navigation_browse_id(r)
                    .map(|b| b.strip_prefix("MPLA").unwrap_or(&b).to_string());
                artists.push(ArtistRef {
                    name: txt.to_string(),
                    id: bid,
                });
            }
        }
        return Some(SearchItem::Track(Track {
            video_id: vid,
            title,
            artists,
            album: None,
            album_id: None,
            duration: None,
            thumbs,
            plays,
            video_type: extract_watch_video_type(item),
            set_video_id: None,
            is_available: true,
        }));
    }

    let bid = extract_navigation_browse_id(item)?;
    let pt = extract_navigation_page_type(item).unwrap_or_default();

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
                } else {
                    artists.push(ArtistRef {
                        name: txt.to_string(),
                        id: extract_navigation_browse_id(r),
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

    Some(SearchItem::Playlist(PlaylistSummary {
        browse_id: bid.strip_prefix("VL").unwrap_or(&bid).to_string(),
        title,
        author: sub_text,
        count: None,
        thumbs,
    }))
}

pub(super) fn parse_any_item(node: &Value) -> Option<SearchItem> {
    if let Some(rli) = node.get("musicResponsiveListItemRenderer") {
        return parse_responsive_list_item(rli);
    }
    if let Some(tri) = node.get("musicTwoRowItemRenderer") {
        return parse_two_row_item(tri);
    }
    None
}
