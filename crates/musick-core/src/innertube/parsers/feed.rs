use std::collections::HashSet;

use serde_json::Value;

use crate::domain::{DiscoverFeed, SearchItem, search_item_key};
use crate::innertube::extract::{find_all_by_key, get_array, get_path, runs_text};

use super::items::parse_any_item;

pub fn parse_discover_feed(
    home_raw: Option<&Value>,
    new_releases_raw: Option<&Value>,
    explore_raw: Option<&Value>,
    charts_raw: Option<&Value>,
) -> DiscoverFeed {
    let mut feed = DiscoverFeed::default();
    let mut seen_mixes: HashSet<String> = HashSet::new();
    let mut seen_new_rel: HashSet<(&'static str, String)> = HashSet::new();
    let mut seen_featured: HashSet<(&'static str, String)> = HashSet::new();
    let mut seen_artists: HashSet<String> = HashSet::new();
    let mut seen_playlists: HashSet<String> = HashSet::new();

    if let Some(home) = home_raw {
        let mut carousels = Vec::new();
        find_all_by_key(home, "musicCarouselShelfRenderer", &mut carousels);
        for shelf in carousels {
            let heading = get_path(
                shelf,
                &["header", "musicCarouselShelfBasicHeaderRenderer", "title"],
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

    for src in [new_releases_raw, explore_raw].into_iter().flatten() {
        let mut carousels = Vec::new();
        find_all_by_key(src, "musicCarouselShelfRenderer", &mut carousels);
        for shelf in carousels {
            let heading = get_path(
                shelf,
                &["header", "musicCarouselShelfBasicHeaderRenderer", "title"],
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
