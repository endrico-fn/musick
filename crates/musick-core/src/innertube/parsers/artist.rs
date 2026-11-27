use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::domain::{AlbumSummary, Artist, ArtistSummary, SearchItem};
use crate::innertube::extract::{
    extract_thumbs, find_all_by_key, get_array, get_path, get_str, runs_text,
};

use super::extract_response_continuation;
use super::items::{parse_any_item, parse_responsive_list_item, parse_two_row_item};

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
                    "watchEndpoint",
                    "playlistId",
                ],
            )
            .or_else(|| {
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
                    "watchEndpoint",
                    "playlistId",
                ],
            )
            .or_else(|| {
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
        })
        .map(str::to_string);

    let views = header
        .and_then(|h| h.get("views").and_then(runs_text))
        .or_else(|| {
            let mut desc_shelves = Vec::new();
            find_all_by_key(raw, "musicDescriptionShelfRenderer", &mut desc_shelves);
            desc_shelves.into_iter().find_map(|s| {
                s.get("subheader")
                    .and_then(runs_text)
                    .filter(|t| t.to_ascii_lowercase().contains("view"))
            })
        });

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
        views,
        albums_browse_id: None,
        singles_browse_id: None,
        videos_playlist_id: None,
        shuffle_id,
    };

    let mut shelves = Vec::new();
    find_all_by_key(raw, "musicShelfRenderer", &mut shelves);
    for shelf in shelves {
        let heading = shelf
            .get("title")
            .and_then(runs_text)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let more_bid =
            get_str(shelf, &["bottomEndpoint", "browseEndpoint", "browseId"]).or_else(|| {
                get_str(
                    shelf,
                    &[
                        "title",
                        "runs",
                        "0",
                        "navigationEndpoint",
                        "browseEndpoint",
                        "browseId",
                    ],
                )
            });
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
                    &[
                        "moreContentButton",
                        "buttonRenderer",
                        "navigationEndpoint",
                        "browseEndpoint",
                    ],
                )
            })
            .or_else(|| {
                hdr.and_then(|h| {
                    get_path(
                        h,
                        &["title", "runs", "0", "navigationEndpoint", "browseEndpoint"],
                    )
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

        let is_featured_or_live = heading.contains("featured") || heading.contains("live");
        if heading.contains("album") && !is_featured_or_live {
            artist.albums_browse_id = more_bid;
            artist.albums_params = more_params;
        } else if heading.contains("single") && !is_featured_or_live {
            artist.singles_browse_id = more_bid;
            artist.singles_params = more_params;
        } else if heading.contains("video") {
            artist.videos_playlist_id =
                more_bid.map(|b| b.strip_prefix("VL").unwrap_or(&b).to_string());
        }

        if let Some(contents) = get_array(shelf, &["contents"]) {
            for c in contents {
                let Some(item) = parse_any_item(c) else {
                    continue;
                };
                match item {
                    SearchItem::Album(mut al) => {
                        if is_featured_or_live {
                            continue;
                        }
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
                        if heading.contains("video") || heading.contains("live") {
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
    parse_artist_releases_with_continuation(raw, default_kind).0
}

pub fn parse_artist_releases_with_continuation(
    raw: &Value,
    default_kind: &str,
) -> (Vec<AlbumSummary>, Option<String>) {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut two_rows = Vec::new();
    find_all_by_key(raw, "musicTwoRowItemRenderer", &mut two_rows);
    for node in two_rows {
        if let Some(SearchItem::Album(mut al)) = parse_two_row_item(node) {
            if al.kind.is_empty()
                || (default_kind.eq_ignore_ascii_case("single")
                    && al.kind.eq_ignore_ascii_case("album"))
            {
                al.kind = default_kind.to_string();
            }
            if seen.insert(al.browse_id.clone()) {
                out.push(al);
            }
        }
    }
    let continuation = extract_response_continuation(raw);
    (out, continuation)
}
