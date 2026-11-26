use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::domain::{
    Album, AlbumSummary, Artist, ArtistRef, ArtistSummary, HomeFeed, LyricLine, Lyrics, Playlist,
    PlaylistSummary, SearchItem, Track, search_item_key,
};
use crate::innertube::endpoints::{PlaybackTracking, PlaylistEditResult, RadioBatch};
use crate::innertube::extract::{
    extract_navigation_browse_id, extract_navigation_page_type, extract_thumbs,
    extract_watch_video_id, extract_watch_video_type, find_all_by_key, get_array, get_path,
    get_str, parse_duration_str, runs_text, runs_vec,
};

pub fn extract_visitor_data(raw: &Value) -> Option<String> {
    get_str(raw, &["responseContext", "visitorData"])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn is_separator_run(text: &str) -> bool {
    let t = text.trim();
    t.is_empty() || matches!(t, "•" | "·" | "," | "&" | "-" | "|")
}

fn looks_like_plays(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    lower.contains("play") || lower.contains("view") || lower.contains("putar")
}

fn clean_plays_label(text: &str) -> String {
    let trimmed = text.trim();
    trimmed
        .split_whitespace()
        .next()
        .unwrap_or(trimmed)
        .to_string()
}

fn looks_like_year(text: &str) -> bool {
    let t = text.trim();
    t.len() == 4 && t.chars().all(|c| c.is_ascii_digit())
}

fn extract_set_video_id(item: &Value) -> Option<String> {
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

fn flex_column_text_val(item: &Value, idx: usize) -> Option<&Value> {
    let cols = get_array(item, &["flexColumns"])?;
    let col = cols.get(idx)?;
    get_path(col, &["musicResponsiveListItemFlexColumnRenderer", "text"])
}

fn fixed_column_text(item: &Value, idx: usize) -> Option<String> {
    let cols = get_array(item, &["fixedColumns"])?;
    let col = cols.get(idx)?;
    let txt_val = get_path(col, &["musicResponsiveListItemFixedColumnRenderer", "text"])?;
    runs_text(txt_val)
}

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

    // If this row points to a browse entity without a videoId, classify as Artist / Album / Playlist.
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

    for col_val in [subtitle_val, col2_val].into_iter().flatten() {
        for r in runs_vec(col_val) {
            let txt = r.get("text").and_then(Value::as_str).unwrap_or("").trim();
            if is_separator_run(txt)
                || txt.eq_ignore_ascii_case("song")
                || txt.eq_ignore_ascii_case("video")
            {
                continue;
            }
            if let Some(dur) = parse_duration_str(txt) {
                if duration.is_none() {
                    duration = Some(dur);
                }
                continue;
            }
            if looks_like_plays(txt) {
                plays = Some(clean_plays_label(txt));
                continue;
            }
            if let Some(bid) = extract_navigation_browse_id(r) {
                let pt = extract_navigation_page_type(r).unwrap_or_default();
                if pt == "MUSIC_PAGE_TYPE_ALBUM" || bid.starts_with("MPRE") {
                    album = Some(txt.to_string());
                    album_id = Some(bid);
                } else if pt == "MUSIC_PAGE_TYPE_ARTIST"
                    || pt == "MUSIC_PAGE_TYPE_USER_CHANNEL"
                    || bid.starts_with("UC")
                {
                    artists.push(ArtistRef {
                        name: txt.to_string(),
                        id: Some(bid),
                    });
                }
            } else if artists.is_empty() && !looks_like_year(txt) {
                // Fallback unlinked artist name
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
        if let Some(sv) = sub_val {
            for r in runs_vec(sv) {
                let txt = r.get("text").and_then(Value::as_str).unwrap_or("").trim();
                if is_separator_run(txt)
                    || txt.eq_ignore_ascii_case("song")
                    || txt.eq_ignore_ascii_case("video")
                    || looks_like_plays(txt)
                {
                    continue;
                }
                let bid = extract_navigation_browse_id(r);
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
            plays: None,
            video_type: extract_watch_video_type(item),
            set_video_id: None,
            is_available: true,
        }));
    }

    let bid = extract_navigation_browse_id(item)?;
    let pt = extract_navigation_page_type(item).unwrap_or_default();

    if pt == "MUSIC_PAGE_TYPE_ARTIST" || bid.starts_with("UC") {
        return Some(SearchItem::Artist(ArtistSummary {
            browse_id: bid,
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

fn parse_any_item(node: &Value) -> Option<SearchItem> {
    if let Some(rli) = node.get("musicResponsiveListItemRenderer") {
        return parse_responsive_list_item(rli);
    }
    if let Some(tri) = node.get("musicTwoRowItemRenderer") {
        return parse_two_row_item(tri);
    }
    None
}

pub fn parse_search_results(raw: &Value) -> Vec<SearchItem> {
    let mut items = Vec::new();
    let mut seen: HashSet<(&'static str, String)> = HashSet::new();

    // 1. Top result card (`musicCardShelfRenderer`)
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

    // 2. Standard shelves (`musicShelfRenderer`)
    let mut shelves = Vec::new();
    find_all_by_key(raw, "musicShelfRenderer", &mut shelves);
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
    }

    items
}

fn parse_card_shelf_top(card: &Value) -> Option<SearchItem> {
    let title_val = card.get("title")?;
    let title = runs_text(title_val)?;
    let thumbs = extract_thumbs(card);
    let sub_text = card.get("subtitle").and_then(runs_text);
    let first_run = runs_vec(title_val).into_iter().next()?;

    if let Some(vid) = extract_watch_video_id(first_run) {
        let mut artists = Vec::new();
        if let Some(sv) = card.get("subtitle") {
            for r in runs_vec(sv) {
                let txt = r.get("text").and_then(Value::as_str).unwrap_or("").trim();
                if is_separator_run(txt)
                    || txt.eq_ignore_ascii_case("song")
                    || txt.eq_ignore_ascii_case("video")
                    || parse_duration_str(txt).is_some()
                {
                    continue;
                }
                if let Some(bid) = extract_navigation_browse_id(r)
                    && bid.starts_with("UC")
                {
                    artists.push(ArtistRef {
                        name: txt.to_string(),
                        id: Some(bid),
                    });
                }
            }
        }
        return Some(SearchItem::Track(Track {
            video_id: vid,
            title,
            artists,
            album: None,
            album_id: None,
            duration: card
                .get("subtitle")
                .and_then(|sv| {
                    runs_vec(sv)
                        .into_iter()
                        .filter_map(|r| r.get("text").and_then(Value::as_str))
                        .find_map(parse_duration_str)
                }),
            thumbs,
            plays: None,
            video_type: extract_watch_video_type(first_run),
            set_video_id: None,
            is_available: true,
        }));
    }

    let bid = extract_navigation_browse_id(first_run)?;
    let pt = extract_navigation_page_type(first_run).unwrap_or_default();
    if pt == "MUSIC_PAGE_TYPE_ARTIST" || bid.starts_with("UC") {
        return Some(SearchItem::Artist(ArtistSummary {
            browse_id: bid,
            name: title,
            subtitle: sub_text,
            thumbs,
        }));
    }
    if pt == "MUSIC_PAGE_TYPE_ALBUM" || bid.starts_with("MPRE") {
        return Some(SearchItem::Album(AlbumSummary {
            browse_id: bid,
            title,
            year: None,
            kind: "Album".into(),
            artists: Vec::new(),
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
    out
}

pub fn parse_home_feed(
    home_raw: Option<&Value>,
    new_releases_raw: Option<&Value>,
    explore_raw: Option<&Value>,
    charts_raw: Option<&Value>,
) -> HomeFeed {
    let mut feed = HomeFeed::default();
    let mut seen_mixes: HashSet<String> = HashSet::new();
    let mut seen_new_rel: HashSet<(&'static str, String)> = HashSet::new();
    let mut seen_featured: HashSet<(&'static str, String)> = HashSet::new();
    let mut seen_artists: HashSet<String> = HashSet::new();
    let mut seen_playlists: HashSet<String> = HashSet::new();

    // 1. FEmusic_home -> Mixes songs list (Quick Picks, Mixes, Supermix, Replay/Discover)
    if let Some(home) = home_raw {
        let mut carousels = Vec::new();
        find_all_by_key(home, "musicCarouselShelfRenderer", &mut carousels);
        for shelf in carousels {
            let heading = get_path(
                shelf,
                &[
                    "header",
                    "musicCarouselShelfBasicHeaderRenderer",
                    "title",
                ],
            )
            .and_then(runs_text)
            .unwrap_or_default()
            .to_ascii_lowercase();

            let is_new_rel_shelf = heading.contains("new release");
            if let Some(contents) = get_array(shelf, &["contents"]) {
                for c in contents {
                    let Some(item) = parse_any_item(c) else {
                        continue;
                    };
                    match &item {
                        SearchItem::Track(tr) => {
                            if is_new_rel_shelf {
                                let (k, id) = search_item_key(&item);
                                if seen_new_rel.insert((k, id.to_string())) {
                                    feed.new_releases.push(item);
                                }
                            } else if seen_mixes.insert(tr.video_id.clone()) {
                                feed.mixes.push(tr.clone());
                            }
                        }
                        SearchItem::Album(_) => {
                            if is_new_rel_shelf {
                                let (k, id) = search_item_key(&item);
                                if seen_new_rel.insert((k, id.to_string())) {
                                    feed.new_releases.push(item.clone());
                                }
                            }
                            let (k, id) = search_item_key(&item);
                            if seen_featured.insert((k, id.to_string())) {
                                feed.featured.push(item);
                            }
                        }
                        SearchItem::Playlist(pl) => {
                            if seen_playlists.insert(pl.browse_id.clone()) {
                                feed.playlists.push(pl.clone());
                            }
                            let (k, id) = search_item_key(&item);
                            if seen_featured.insert((k, id.to_string())) {
                                feed.featured.push(item);
                            }
                        }
                        SearchItem::Artist(ar) => {
                            if seen_artists.insert(ar.browse_id.clone()) {
                                feed.artists.push(ar.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. FEmusic_new_releases & FEmusic_explore -> New Releases tab
    for src in [new_releases_raw, explore_raw].into_iter().flatten() {
        let mut carousels = Vec::new();
        find_all_by_key(src, "musicCarouselShelfRenderer", &mut carousels);
        for shelf in carousels {
            let heading = get_path(
                shelf,
                &[
                    "header",
                    "musicCarouselShelfBasicHeaderRenderer",
                    "title",
                ],
            )
            .and_then(runs_text)
            .unwrap_or_default()
            .to_ascii_lowercase();
            let is_new_or_unlabeled = heading.is_empty()
                || heading.contains("new")
                || heading.contains("album")
                || heading.contains("single")
                || heading.contains("release");

            if let Some(contents) = get_array(shelf, &["contents"]) {
                for c in contents {
                    let Some(item) = parse_any_item(c) else {
                        continue;
                    };
                    if is_new_or_unlabeled {
                        let (k, id) = search_item_key(&item);
                        if seen_new_rel.insert((k, id.to_string())) {
                            feed.new_releases.push(item.clone());
                        }
                    }
                    if let SearchItem::Playlist(pl) = &item
                        && seen_playlists.insert(pl.browse_id.clone())
                    {
                        feed.playlists.push(pl.clone());
                    }
                }
            }
        }

        // Also support gridRenderer used by FEmusic_new_releases_albums
        let mut grids = Vec::new();
        find_all_by_key(src, "gridRenderer", &mut grids);
        for grid in grids {
            if let Some(items) = get_array(grid, &["items"]) {
                for c in items {
                    if let Some(item) = parse_any_item(c) {
                        let (k, id) = search_item_key(&item);
                        if seen_new_rel.insert((k, id.to_string())) {
                            feed.new_releases.push(item);
                        }
                    }
                }
            }
        }
    }

    // 3. FEmusic_charts -> top tracks fallback for mixes + trending artists
    if let Some(charts) = charts_raw {
        let mut carousels = Vec::new();
        find_all_by_key(charts, "musicCarouselShelfRenderer", &mut carousels);
        for shelf in carousels {
            if let Some(contents) = get_array(shelf, &["contents"]) {
                for c in contents {
                    let Some(item) = parse_any_item(c) else {
                        continue;
                    };
                    match item {
                        SearchItem::Track(tr) => {
                            if seen_mixes.insert(tr.video_id.clone()) {
                                feed.mixes.push(tr);
                            }
                        }
                        SearchItem::Artist(ar) => {
                            if seen_artists.insert(ar.browse_id.clone()) {
                                feed.artists.push(ar);
                            }
                        }
                        SearchItem::Playlist(pl) => {
                            if seen_playlists.insert(pl.browse_id.clone()) {
                                feed.playlists.push(pl);
                            }
                        }
                        SearchItem::Album(_) => {
                            let (k, id) = search_item_key(&item);
                            if seen_new_rel.insert((k, id.to_string())) {
                                feed.new_releases.push(item);
                            }
                        }
                    }
                }
            }
        }
    }

    feed
}

pub fn extract_plays_from_browse(raw: &Value) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut rows = Vec::new();
    find_all_by_key(raw, "musicResponsiveListItemRenderer", &mut rows);
    for row in rows {
        if let Some(SearchItem::Track(tr)) = parse_responsive_list_item(row)
            && let Some(p) = tr.plays
        {
            map.insert(tr.video_id, p);
        }
    }
    map
}

pub fn parse_artist(browse_id: &str, raw: &Value) -> Artist {
    let header = get_path(raw, &["header", "musicImmersiveHeaderRenderer"])
        .or_else(|| get_path(raw, &["header", "musicVisualHeaderRenderer"]));

    let name = header
        .and_then(|h| h.get("title"))
        .and_then(runs_text)
        .unwrap_or_else(|| "Artist".to_string());
    let description = header
        .and_then(|h| h.get("description"))
        .and_then(runs_text);
    let listeners = header
        .and_then(|h| h.get("monthlyListenerCount"))
        .and_then(runs_text);
    let subscribers = header
        .and_then(|h| {
            get_path(
                h,
                &[
                    "subscriptionButton",
                    "subscribeButtonRenderer",
                    "longSubscriberCountText",
                ],
            )
            .or_else(|| {
                get_path(
                    h,
                    &[
                        "subscriptionButton",
                        "subscribeButtonRenderer",
                        "subscriberCountText",
                    ],
                )
            })
        })
        .and_then(runs_text);
    let thumbs = header.map(extract_thumbs).unwrap_or_default();

    let radio_id = header
        .and_then(|h| {
            get_str(
                h,
                &[
                    "startRadioButton",
                    "buttonRenderer",
                    "navigationEndpoint",
                    "watchPlaylistEndpoint",
                    "playlistId",
                ],
            )
        })
        .map(str::to_string);
    let shuffle_id = header
        .and_then(|h| {
            get_str(
                h,
                &[
                    "playButton",
                    "buttonRenderer",
                    "navigationEndpoint",
                    "watchPlaylistEndpoint",
                    "playlistId",
                ],
            )
        })
        .map(str::to_string);

    let mut artist = Artist {
        summary: ArtistSummary {
            browse_id: browse_id.to_string(),
            name,
            subtitle: listeners.clone().or_else(|| subscribers.clone()),
            thumbs,
        },
        description,
        listeners,
        songs: Vec::new(),
        albums: Vec::new(),
        singles: Vec::new(),
        playlists: Vec::new(),
        related: Vec::new(),
        albums_params: None,
        singles_params: None,
        radio_id,
        songs_playlist_id: None,
        videos: Vec::new(),
        subscribers,
        views: None,
        albums_browse_id: None,
        singles_browse_id: None,
        videos_playlist_id: None,
        shuffle_id,
    };

    // Songs shelf (`musicShelfRenderer`)
    let mut shelves = Vec::new();
    find_all_by_key(raw, "musicShelfRenderer", &mut shelves);
    for shelf in shelves {
        let heading = shelf
            .get("title")
            .and_then(runs_text)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let more_bid = get_str(
            shelf,
            &["bottomEndpoint", "browseEndpoint", "browseId"],
        )
        .or_else(|| get_str(shelf, &["title", "runs", "0", "navigationEndpoint", "browseEndpoint", "browseId"]));
        if heading.contains("song")
            && let Some(bid) = more_bid
        {
            artist.songs_playlist_id = Some(bid.strip_prefix("VL").unwrap_or(bid).to_string());
        }
        if let Some(contents) = get_array(shelf, &["contents"]) {
            for c in contents {
                if let Some(SearchItem::Track(tr)) = parse_any_item(c) {
                    artist.songs.push(tr);
                }
            }
        }
    }

    // Carousel shelves (Albums, Singles, Videos, Featured on / Playlists, Fans might also like)
    let mut carousels = Vec::new();
    find_all_by_key(raw, "musicCarouselShelfRenderer", &mut carousels);
    for shelf in carousels {
        let hdr = get_path(shelf, &["header", "musicCarouselShelfBasicHeaderRenderer"]);
        let heading = hdr
            .and_then(|h| h.get("title"))
            .and_then(runs_text)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let more_ep = hdr
            .and_then(|h| {
                get_path(
                    h,
                    &["moreContentButton", "buttonRenderer", "navigationEndpoint", "browseEndpoint"],
                )
            })
            .or_else(|| {
                hdr.and_then(|h| {
                    get_path(h, &["title", "runs", "0", "navigationEndpoint", "browseEndpoint"])
                })
            });
        let more_bid = more_ep
            .and_then(|ep| ep.get("browseId"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let more_params = more_ep
            .and_then(|ep| ep.get("params"))
            .and_then(Value::as_str)
            .map(str::to_string);

        if heading.contains("album") {
            artist.albums_browse_id = more_bid;
            artist.albums_params = more_params;
        } else if heading.contains("single") {
            artist.singles_browse_id = more_bid;
            artist.singles_params = more_params;
        } else if heading.contains("video") {
            artist.videos_playlist_id = more_bid.map(|b| b.strip_prefix("VL").unwrap_or(&b).to_string());
        }

        if let Some(contents) = get_array(shelf, &["contents"]) {
            for c in contents {
                let Some(item) = parse_any_item(c) else {
                    continue;
                };
                match item {
                    SearchItem::Album(mut al) => {
                        if heading.contains("single") {
                            if al.kind == "Album" {
                                al.kind = "Single".into();
                            }
                            artist.singles.push(al);
                        } else {
                            artist.albums.push(al);
                        }
                    }
                    SearchItem::Track(tr) => {
                        if heading.contains("video") {
                            artist.videos.push(tr);
                        } else {
                            artist.songs.push(tr);
                        }
                    }
                    SearchItem::Playlist(pl) => {
                        artist.playlists.push(pl);
                    }
                    SearchItem::Artist(ar) => {
                        artist.related.push(ar);
                    }
                }
            }
        }
    }

    artist
}

pub fn parse_artist_releases(raw: &Value, default_kind: &str) -> Vec<AlbumSummary> {
    let mut out = Vec::new();
    let mut two_rows = Vec::new();
    find_all_by_key(raw, "musicTwoRowItemRenderer", &mut two_rows);
    for node in two_rows {
        if let Some(SearchItem::Album(mut al)) = parse_two_row_item(node) {
            if al.kind.is_empty() {
                al.kind = default_kind.to_string();
            }
            out.push(al);
        }
    }
    out
}

pub fn parse_album(browse_id: &str, raw: &Value) -> Album {
    let header = get_path(raw, &["header", "musicDetailHeaderRenderer"])
        .or_else(|| {
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
            h.get("description")
                .or_else(|| get_path(h, &["description", "musicDescriptionShelfRenderer", "description"]))
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
    }
}

pub fn parse_playlist(playlist_id: &str, raw: &Value) -> (Playlist, Option<String>) {
    let header = get_path(raw, &["header", "musicDetailHeaderRenderer"])
        .or_else(|| get_path(raw, &["header", "musicEditablePlaylistDetailHeaderRenderer", "header", "musicDetailHeaderRenderer"]))
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
    let author = header
        .and_then(|h| h.get("straplineTextOne").or_else(|| h.get("subtitle")))
        .and_then(runs_text);
    let description = header
        .and_then(|h| {
            h.get("description")
                .or_else(|| get_path(h, &["description", "musicDescriptionShelfRenderer", "description"]))
        })
        .and_then(runs_text);
    let second_sub = header
        .and_then(|h| h.get("secondSubtitle"))
        .and_then(runs_text);

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

    // Continuation responses return `continuationContents.musicPlaylistShelfContinuation`
    if let Some(cont_shelf) =
        get_path(raw, &["continuationContents", "musicPlaylistShelfContinuation"])
    {
        if let Some(contents) = get_array(cont_shelf, &["contents"]) {
            for c in contents {
                if let Some(SearchItem::Track(tr)) = parse_any_item(c) {
                    tracks.push(tr);
                }
            }
        }
        continuation = extract_continuation_token(cont_shelf);
    }

    let total = Some(tracks.len());
    let playlist = Playlist {
        summary: PlaylistSummary {
            browse_id: playlist_id.strip_prefix("VL").unwrap_or(playlist_id).to_string(),
            title,
            author,
            count: second_sub.clone(),
            thumbs,
        },
        tracks,
        description,
        views: None,
        duration_text: second_sub,
        year: None,
        total,
    };
    (playlist, continuation)
}

fn extract_continuation_token(node: &Value) -> Option<String> {
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
        if let Some(tok) = get_str(item, &["continuationEndpoint", "continuationCommand", "token"]) {
            return Some(tok.to_string());
        }
    }
    None
}

pub fn parse_watch_playlist(raw: &Value) -> (RadioBatch, Option<String>, Option<String>) {
    let mut tracks = Vec::new();
    let mut playlist_id = None;
    let mut continuation = None;

    let panel = get_path(
        raw,
        &[
            "contents",
            "singleColumnMusicWatchNextResultsRenderer",
            "tabbedRenderer",
            "watchNextTabbedResultsRenderer",
            "tabs",
            "0",
            "tabRenderer",
            "content",
            "musicQueueRenderer",
            "content",
            "playlistPanelRenderer",
        ],
    )
    .or_else(|| {
        get_path(
            raw,
            &["continuationContents", "playlistPanelContinuation"],
        )
    });

    if let Some(p) = panel {
        playlist_id = p
            .get("playlistId")
            .and_then(Value::as_str)
            .map(str::to_string);
        continuation = extract_continuation_token(p);
        if let Some(contents) = get_array(p, &["contents"]) {
            for c in contents {
                if let Some(r) = c
                    .get("playlistPanelVideoRenderer")
                    .or_else(|| get_path(c, &["playlistPanelVideoWrapperRenderer", "primaryRenderer", "playlistPanelVideoRenderer"]))
                    && let Some(tr) = parse_playlist_panel_video(r)
                {
                    tracks.push(tr);
                }
            }
        }
    }

    // Extract lyrics and related browse IDs from watch tabs
    let mut lyrics_id = None;
    let mut related_id = None;
    if let Some(tabs) = get_array(
        raw,
        &[
            "contents",
            "singleColumnMusicWatchNextResultsRenderer",
            "tabbedRenderer",
            "watchNextTabbedResultsRenderer",
            "tabs",
        ],
    ) {
        for tab in tabs {
            let Some(tr) = tab.get("tabRenderer") else {
                continue;
            };
            let tab_title = tr
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            let bid = get_str(tr, &["endpoint", "browseEndpoint", "browseId"]);
            if let Some(b) = bid {
                if tab_title.contains("lyric") || b.starts_with("MPLYt") {
                    lyrics_id = Some(b.to_string());
                } else if tab_title.contains("related") || b.starts_with("MPTRt") {
                    related_id = Some(b.to_string());
                }
            }
        }
    }

    (
        RadioBatch {
            tracks,
            continuation,
            playlist_id,
        },
        lyrics_id,
        related_id,
    )
}

fn parse_playlist_panel_video(r: &Value) -> Option<Track> {
    let video_id = r.get("videoId")?.as_str()?.to_string();
    let title = r.get("title").and_then(runs_text)?;
    let thumbs = extract_thumbs(r);
    let duration = r
        .get("lengthText")
        .and_then(runs_text)
        .and_then(|s| parse_duration_str(&s));
    let video_type = extract_watch_video_type(r);
    let set_video_id = r
        .get("playlistSetVideoId")
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut artists = Vec::new();
    let mut album = None;
    let mut album_id = None;
    if let Some(byline) = r.get("longBylineText").or_else(|| r.get("shortBylineText")) {
        for run in runs_vec(byline) {
            let txt = run.get("text").and_then(Value::as_str).unwrap_or("").trim();
            if is_separator_run(txt) || looks_like_year(txt) {
                continue;
            }
            if let Some(bid) = extract_navigation_browse_id(run) {
                let pt = extract_navigation_page_type(run).unwrap_or_default();
                if pt == "MUSIC_PAGE_TYPE_ALBUM" || bid.starts_with("MPRE") {
                    album = Some(txt.to_string());
                    album_id = Some(bid);
                } else {
                    artists.push(ArtistRef {
                        name: txt.to_string(),
                        id: Some(bid),
                    });
                }
            } else if artists.is_empty() {
                artists.push(ArtistRef {
                    name: txt.to_string(),
                    id: None,
                });
            }
        }
    }

    Some(Track {
        video_id,
        title,
        artists,
        album,
        album_id,
        duration,
        thumbs,
        plays: None,
        video_type,
        set_video_id,
        is_available: true,
    })
}

pub fn parse_yt_lyrics(raw: &Value) -> Option<Lyrics> {
    // 1. Timed lyrics (`ANDROID_MUSIC` -> `timedLyricsData`)
    let mut timed_lists = Vec::new();
    find_all_by_key(raw, "timedLyricsData", &mut timed_lists);
    for list_val in timed_lists {
        let Some(arr) = list_val.as_array() else {
            continue;
        };
        let mut lines = Vec::new();
        for entry in arr {
            let Some(text) = entry.get("lyricLine").and_then(Value::as_str) else {
                continue;
            };
            let start_ms = get_path(entry, &["cueRange", "startTimeMilliseconds"])
                .and_then(|v| {
                    v.as_u64()
                        .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
                })
                .unwrap_or(0);
            lines.push(LyricLine::new(start_ms as f64 / 1000.0, text.trim()));
        }
        if !lines.is_empty() {
            lines.sort_by(|a, b| a.time.total_cmp(&b.time));
            return Some(Lyrics {
                lines,
                synced: true,
                source: "YouTube Music".to_string(),
                translation_lang: String::new(),
            });
        }
    }

    // 2. Plain lyrics (`WEB_REMIX` -> `musicDescriptionShelfRenderer`)
    let mut shelves = Vec::new();
    find_all_by_key(raw, "musicDescriptionShelfRenderer", &mut shelves);
    for shelf in shelves {
        if let Some(text) = shelf.get("description").and_then(runs_text) {
            let lines: Vec<LyricLine> = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(|l| LyricLine::new(0.0, l))
                .collect();
            if !lines.is_empty() {
                return Some(Lyrics {
                    lines,
                    synced: false,
                    source: "YouTube Music".to_string(),
                    translation_lang: String::new(),
                });
            }
        }
    }

    None
}

pub fn parse_library_playlists(raw: &Value) -> Vec<PlaylistSummary> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut two_rows = Vec::new();
    find_all_by_key(raw, "musicTwoRowItemRenderer", &mut two_rows);
    for node in two_rows {
        if let Some(SearchItem::Playlist(pl)) = parse_two_row_item(node) {
            // Skip Liked Music pseudo-playlist and Episodes for Later
            if matches!(pl.browse_id.as_str(), "LM" | "SE") {
                continue;
            }
            if seen.insert(pl.browse_id.clone()) {
                out.push(pl);
            }
        }
    }
    out
}

pub fn parse_library_albums(raw: &Value) -> Vec<AlbumSummary> {
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
    out
}

pub fn parse_library_artists(raw: &Value) -> Vec<ArtistSummary> {
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
    out
}

pub fn parse_player_tracking(video_id: &str, raw: &Value) -> Option<PlaybackTracking> {
    let tracking = raw.get("playbackTracking")?;
    let playback_url = get_str(tracking, &["videostatsPlaybackUrl", "baseUrl"]).map(str::to_string);
    let watchtime_url =
        get_str(tracking, &["videostatsWatchtimeUrl", "baseUrl"]).map(str::to_string);
    if playback_url.is_none() && watchtime_url.is_none() {
        return None;
    }
    let length_secs = get_str(raw, &["videoDetails", "lengthSeconds"])
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    Some(PlaybackTracking {
        video_id: video_id.to_string(),
        playback_url,
        watchtime_url,
        length_secs,
    })
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
