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
    let home_pages: Vec<&Value> = home_raw.into_iter().collect();
    parse_discover_feed_pages(&home_pages, new_releases_raw, explore_raw, charts_raw)
}

#[allow(clippy::too_many_arguments)]
fn push_home_item(
    item: SearchItem,
    is_new_rel_shelf: bool,
    feed: &mut DiscoverFeed,
    seen_mixes: &mut HashSet<String>,
    seen_new_rel: &mut HashSet<(&'static str, String)>,
    seen_featured: &mut HashSet<(&'static str, String)>,
    seen_artists: &mut HashSet<String>,
    seen_playlists: &mut HashSet<String>,
) {
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

pub fn parse_discover_feed_pages(
    home_pages: &[&Value],
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

    for home in home_pages {
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
                    if let Some(item) = parse_any_item(c) {
                        push_home_item(
                            item,
                            is_new_rel_shelf,
                            &mut feed,
                            &mut seen_mixes,
                            &mut seen_new_rel,
                            &mut seen_featured,
                            &mut seen_artists,
                            &mut seen_playlists,
                        );
                    }
                }
            }
        }

        let mut shelves = Vec::new();
        find_all_by_key(home, "musicShelfRenderer", &mut shelves);
        for shelf in shelves {
            let heading = shelf
                .get("title")
                .and_then(runs_text)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let is_new_rel_shelf = heading.contains("new release");
            if let Some(contents) = get_array(shelf, &["contents"]) {
                for c in contents {
                    if let Some(item) = parse_any_item(c) {
                        push_home_item(
                            item,
                            is_new_rel_shelf,
                            &mut feed,
                            &mut seen_mixes,
                            &mut seen_new_rel,
                            &mut seen_featured,
                            &mut seen_artists,
                            &mut seen_playlists,
                        );
                    }
                }
            }
        }

        let mut grids = Vec::new();
        find_all_by_key(home, "gridRenderer", &mut grids);
        for grid in grids {
            if let Some(items) = get_array(grid, &["items"]) {
                for c in items {
                    if let Some(item) = parse_any_item(c) {
                        push_home_item(
                            item,
                            false,
                            &mut feed,
                            &mut seen_mixes,
                            &mut seen_new_rel,
                            &mut seen_featured,
                            &mut seen_artists,
                            &mut seen_playlists,
                        );
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
                    } else if let SearchItem::Track(tr) = &item
                        && seen_mixes.insert(tr.video_id.clone())
                    {
                        feed.mixes.push(tr.clone());
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
        let mut shelves = Vec::new();
        find_all_by_key(charts, "musicCarouselShelfRenderer", &mut shelves);
        find_all_by_key(charts, "musicShelfRenderer", &mut shelves);
        for shelf in shelves {
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
