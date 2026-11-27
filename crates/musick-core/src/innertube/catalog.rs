use std::collections::{HashMap, HashSet};

use serde_json::json;
use tracing::debug;

use crate::domain::{
    Album, DiscoverFeed, Lyrics, Playlist, SearchCategory, SearchItem, Track, search_item_key,
};
use crate::error::{CoreError, CoreResult};

use super::{ClientKind, InnerTubeClient, RadioBatch, parsers, search_filter_params};

impl InnerTubeClient {
    pub(super) async fn fetch_playlist_uncached(
        &self,
        playlist_id: &str,
        limit: usize,
    ) -> CoreResult<Playlist> {
        let clean_pid = playlist_id.strip_prefix("VL").unwrap_or(playlist_id);
        let browse_pid = if clean_pid.starts_with("MP") {
            clean_pid.to_string()
        } else {
            format!("VL{clean_pid}")
        };

        let raw = self
            .browse(&browse_pid, None, None, ClientKind::WebRemix, None)
            .await?;
        let (mut pl, mut cont) = parsers::parse_playlist(clean_pid, &raw);

        let mut pages = 0usize;
        while pl.tracks.len() < limit
            && let Some(token) = cont.take()
            && pages < 10
        {
            pages += 1;
            match self
                .browse(&browse_pid, None, Some(&token), ClientKind::WebRemix, None)
                .await
            {
                Ok(page_raw) => {
                    let (page_pl, next_cont) = parsers::parse_playlist(clean_pid, &page_raw);
                    if page_pl.tracks.is_empty() {
                        break;
                    }
                    pl.tracks.extend(page_pl.tracks);
                    cont = next_cont;
                }
                Err(err) => {
                    debug!("Playlist pagination stopped early for {clean_pid}: {err}");
                    break;
                }
            }
        }

        pl.tracks.truncate(limit);
        pl.total = Some(pl.tracks.len());
        Ok(pl)
    }

    async fn paginate_search_page(
        &self,
        mut items: Vec<SearchItem>,
        mut cont: Option<String>,
        limit: usize,
    ) -> Vec<SearchItem> {
        let mut seen: HashSet<(&'static str, String)> = items
            .iter()
            .map(|it| {
                let (k, id) = search_item_key(it);
                (k, id.to_string())
            })
            .collect();
        let mut pages = 0usize;
        while items.len() < limit.min(20)
            && let Some(token) = cont.take()
            && pages < 1
        {
            pages += 1;
            let mut map = serde_json::Map::new();
            map.insert("continuation".into(), json!(token));
            match self
                .post_json("search", ClientKind::WebRemix, None, map)
                .await
            {
                Ok(page_raw) => {
                    let (page_items, next_cont) =
                        parsers::parse_search_results_with_continuation(&page_raw);
                    let before = items.len();
                    for it in page_items {
                        let (k, id) = search_item_key(&it);
                        if seen.insert((k, id.to_string())) {
                            items.push(it);
                        }
                    }
                    if items.len() == before {
                        break;
                    }
                    cont = next_cont;
                }
                Err(err) => {
                    debug!("Search pagination stopped early: {err}");
                    break;
                }
            }
        }
        items
    }

    pub(super) async fn discover_feed_impl(
        &self,
        region: &str,
        force_refresh: bool,
    ) -> CoreResult<DiscoverFeed> {
        let cache_key = format!("discover:{region}:{}", self.auth.account_key());
        if !force_refresh
            && let Some(cached) = self.cache.get_catalog::<DiscoverFeed>(&cache_key)
        {
            return Ok(cached);
        }

        // fetch home first so visitorData is primed before follow-up browse requests
        let mut home_pages = Vec::new();
        if let Ok(first_home) = self
            .browse(
                "FEmusic_home",
                None,
                None,
                ClientKind::WebRemix,
                Some(region),
            )
            .await
        {
            let mut next_cont = parsers::extract_response_continuation(&first_home);
            home_pages.push(first_home);
            let mut pages = 0usize;
            while let Some(token) = next_cont.take()
                && pages < 2
            {
                pages += 1;
                match self
                    .browse(
                        "FEmusic_home",
                        None,
                        Some(&token),
                        ClientKind::WebRemix,
                        Some(region),
                    )
                    .await
                {
                    Ok(page_raw) => {
                        next_cont = parsers::extract_response_continuation(&page_raw);
                        home_pages.push(page_raw);
                    }
                    Err(err) => {
                        debug!("Home feed continuation stopped early: {err}");
                        break;
                    }
                }
            }
        }

        let (new_rel_res, explore_res, charts_res) = tokio::join!(
            self.browse(
                "FEmusic_new_releases",
                None,
                None,
                ClientKind::WebRemix,
                Some(region),
            ),
            self.browse(
                "FEmusic_explore",
                None,
                None,
                ClientKind::WebRemix,
                Some(region),
            ),
            self.browse(
                "FEmusic_charts",
                None,
                None,
                ClientKind::WebRemix,
                Some(region),
            ),
        );

        let new_rel_v = new_rel_res.ok();
        let explore_v = explore_res.ok();
        let charts_v = charts_res.ok();

        if home_pages.is_empty()
            && new_rel_v.is_none()
            && explore_v.is_none()
            && charts_v.is_none()
        {
            return Err(CoreError::Catalog(
                "Unable to load YouTube Music home/explore feeds".into(),
            ));
        }

        let home_refs: Vec<&serde_json::Value> = home_pages.iter().collect();
        let mut feed = parsers::parse_discover_feed_pages(
            &home_refs,
            new_rel_v.as_ref(),
            explore_v.as_ref(),
            charts_v.as_ref(),
        );

        if feed.mixes.is_empty() {
            let mix_playlist = feed
                .playlists
                .iter()
                .find(|pl| {
                    pl.browse_id.starts_with("RD")
                        || pl.title.to_ascii_lowercase().contains("mix")
                        || pl.title.to_ascii_lowercase().contains("supermix")
                })
                .or_else(|| feed.playlists.first())
                .cloned();

            if let Some(pl) = mix_playlist {
                if pl.browse_id.starts_with("RD") {
                    if let Ok(batch) = self.radio_impl(None, Some(&pl.browse_id), None, 25).await {
                        feed.mixes = batch.tracks;
                    }
                } else if let Ok(full_pl) = self.fetch_playlist_uncached(&pl.browse_id, 25).await {
                    feed.mixes = full_pl.tracks;
                }
            }

            if feed.mixes.is_empty()
                && self.auth.is_authenticated()
                && let Ok(liked) = self.library_liked_songs_impl(1).await
                && let Some(seed) = liked.first()
                && let Ok(batch) = self.radio_impl(Some(&seed.video_id), None, None, 25).await
            {
                feed.mixes = batch.tracks;
            }

            if feed.mixes.is_empty() {
                let mut seen = HashSet::new();
                for item in feed.new_releases.iter().chain(feed.featured.iter()) {
                    if let SearchItem::Track(tr) = item
                        && seen.insert(tr.video_id.clone())
                    {
                        feed.mixes.push(tr.clone());
                    }
                }
            }
        }

        let ttl = if feed.mixes.is_empty() { Some(60) } else { None };
        self.cache.put_catalog(&cache_key, &feed, ttl);
        Ok(feed)
    }

    pub(super) async fn search_impl(
        &self,
        category: SearchCategory,
        query: &str,
        limit: usize,
    ) -> CoreResult<Vec<SearchItem>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let cache_key = format!(
            "search:{}:{}:{}:{limit}",
            self.cache_scope(),
            category.as_str(),
            q.to_lowercase()
        );
        if let Some(cached) = self.cache.get_catalog::<Vec<SearchItem>>(&cache_key) {
            return Ok(cached);
        }

        let out = if category == SearchCategory::All {
            let mut map_all = serde_json::Map::new();
            map_all.insert("query".into(), json!(q));
            let raw_all = self
                .post_json("search", ClientKind::WebRemix, None, map_all)
                .await?;
            let (parsed_all, cont_all) = parsers::parse_search_results_with_continuation(&raw_all);

            // enrich with a songs query only when `all` returned few items or tracks missing album info
            let needs_song_enrichment = parsed_all.len() < limit.min(20)
                || parsed_all
                    .iter()
                    .any(|it| matches!(it, SearchItem::Track(t) if t.album.is_none()));
            let (parsed_songs, cont_songs) = if needs_song_enrichment {
                let mut map_songs = serde_json::Map::new();
                map_songs.insert("query".into(), json!(q));
                if let Some(p) = search_filter_params(SearchCategory::Songs) {
                    map_songs.insert("params".into(), json!(p));
                }
                self.post_json("search", ClientKind::WebRemix, None, map_songs)
                    .await
                    .as_ref()
                    .map(parsers::parse_search_results_with_continuation)
                    .unwrap_or_default()
            } else {
                (Vec::new(), None)
            };

            let song_by_vid: HashMap<String, Track> = parsed_songs
                .iter()
                .filter_map(|it| match it {
                    SearchItem::Track(t) => Some((t.video_id.clone(), t.clone())),
                    _ => None,
                })
                .collect();

            let mut merged = Vec::new();
            let mut seen: HashSet<(&'static str, String)> = HashSet::new();
            for item in parsed_all.into_iter().chain(parsed_songs) {
                let upgraded = match item {
                    SearchItem::Track(mut tr) => {
                        if let Some(song) = song_by_vid.get(&tr.video_id)
                            && ((tr.album.is_none() && song.album.is_some())
                                || song.video_type.as_deref() == Some("MUSIC_VIDEO_TYPE_ATV"))
                        {
                            if !song.title.is_empty() {
                                tr.title = song.title.clone();
                            }
                            if !song.artists.is_empty() {
                                tr.artists = song.artists.clone();
                            }
                            if song.album.is_some() {
                                tr.album = song.album.clone();
                            }
                            if song.album_id.is_some() {
                                tr.album_id = song.album_id.clone();
                            }
                            if song.duration.is_some() {
                                tr.duration = song.duration;
                            }
                            if !song.thumbs.is_empty() {
                                tr.thumbs = song.thumbs.clone();
                            }
                            if tr.plays.is_none() {
                                tr.plays = song.plays.clone();
                            }
                            if song.video_type.is_some() {
                                tr.video_type = song.video_type.clone();
                            }
                        }
                        SearchItem::Track(tr)
                    }
                    other => other,
                };
                let (k, id) = search_item_key(&upgraded);
                if seen.insert((k, id.to_string())) {
                    merged.push(upgraded);
                }
            }
            let cont = cont_songs.or(cont_all);
            if merged.len() < limit.min(20) && cont.is_some() {
                merged = self.paginate_search_page(merged, cont, limit).await;
            }
            if merged.is_empty()
                && let Some(corrected) = parsers::parse_spelling_hint(&raw_all)
                && !corrected.eq_ignore_ascii_case(q)
            {
                let mut map_corr = serde_json::Map::new();
                map_corr.insert("query".into(), json!(corrected));
                if let Ok(raw_corr) = self
                    .post_json("search", ClientKind::WebRemix, None, map_corr)
                    .await
                {
                    merged = parsers::parse_search_results(&raw_corr);
                }
            }
            merged
        } else {
            let mut map = serde_json::Map::new();
            map.insert("query".into(), json!(q));
            if let Some(p) = search_filter_params(category) {
                map.insert("params".into(), json!(p));
            }
            let raw = self
                .post_json("search", ClientKind::WebRemix, None, map)
                .await?;
            let (items, cont) = parsers::parse_search_results_with_continuation(&raw);
            let mut paged = self.paginate_search_page(items, cont, limit).await;
            if paged.is_empty()
                && let Some(corrected) = parsers::parse_spelling_hint(&raw)
                && !corrected.eq_ignore_ascii_case(q)
            {
                let mut map_corr = serde_json::Map::new();
                map_corr.insert("query".into(), json!(corrected));
                if let Some(p) = search_filter_params(category) {
                    map_corr.insert("params".into(), json!(p));
                }
                if let Ok(raw_corr) = self
                    .post_json("search", ClientKind::WebRemix, None, map_corr)
                    .await
                {
                    paged = parsers::parse_search_results(&raw_corr);
                }
            }
            paged
        };

        let capped: Vec<SearchItem> = out.into_iter().take(limit).collect();
        if !capped.is_empty() {
            self.cache.put_catalog(&cache_key, &capped, None);
        }
        Ok(capped)
    }

    pub(super) async fn suggestions_impl(&self, query: &str) -> CoreResult<Vec<String>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let cache_key = format!("sugg:{}:{}", self.region(), q.to_lowercase());
        if let Some(cached) = self.cache.get_catalog::<Vec<String>>(&cache_key) {
            return Ok(cached);
        }

        let mut map = serde_json::Map::new();
        map.insert("input".into(), json!(q));
        let raw = self
            .post_json(
                "music/get_search_suggestions",
                ClientKind::WebRemix,
                None,
                map,
            )
            .await?;
        let sug = parsers::parse_suggestions(&raw);
        self.cache.put_catalog(&cache_key, &sug, None);
        Ok(sug)
    }

    pub(super) async fn album_impl(&self, browse_id: &str) -> CoreResult<Album> {
        let cache_key = format!("album:{}:{browse_id}", self.region());
        if let Some(cached) = self.cache.get_catalog::<Album>(&cache_key) {
            return Ok(cached);
        }
        let raw = self
            .browse(browse_id, None, None, ClientKind::WebRemix, None)
            .await?;
        let album = parsers::parse_album(browse_id, &raw);
        self.cache.put_catalog(&cache_key, &album, None);
        Ok(album)
    }

    pub(super) async fn playlist_impl(
        &self,
        playlist_id: &str,
        limit: usize,
    ) -> CoreResult<Playlist> {
        let clean_pid = playlist_id.strip_prefix("VL").unwrap_or(playlist_id);
        let cacheable = clean_pid != "LM";
        let cache_key = format!("playlist:{}:{clean_pid}:{limit}", self.region());
        if cacheable && let Some(cached) = self.cache.get_catalog::<Playlist>(&cache_key) {
            return Ok(cached);
        }
        let pl = self.fetch_playlist_uncached(playlist_id, limit).await?;
        if cacheable {
            self.cache.put_catalog(&cache_key, &pl, None);
        }
        Ok(pl)
    }

    pub(super) async fn radio_impl(
        &self,
        video_id: Option<&str>,
        playlist_id: Option<&str>,
        continuation: Option<&str>,
        limit: usize,
    ) -> CoreResult<RadioBatch> {
        let mut map = serde_json::Map::new();
        map.insert("enablePersistentPlaylistPanel".into(), json!(true));
        map.insert("isAudioOnly".into(), json!(true));

        if let Some(cont) = continuation {
            map.insert("continuation".into(), json!(cont));
        } else {
            if let Some(vid) = video_id {
                map.insert("videoId".into(), json!(vid));
            }
            let pid = playlist_id
                .map(str::to_string)
                .or_else(|| video_id.map(|v| format!("RDAMVM{v}")));
            if let Some(p) = pid {
                map.insert("playlistId".into(), json!(p));
            }
            map.insert("params".into(), json!("wAEB"));
        }

        let raw = self
            .post_json("next", ClientKind::WebRemix, None, map)
            .await?;
        let (mut batch, _, _) = parsers::parse_watch_playlist(&raw);
        batch.tracks.truncate(limit);
        Ok(batch)
    }

    pub(super) async fn yt_lyrics_impl(&self, video_id: &str) -> CoreResult<Option<Lyrics>> {
        let mut map = serde_json::Map::new();
        map.insert("videoId".into(), json!(video_id));
        map.insert("isAudioOnly".into(), json!(true));
        let watch_raw = self
            .post_json("next", ClientKind::WebRemix, None, map)
            .await?;
        let (_, lyrics_id, _) = parsers::parse_watch_playlist(&watch_raw);
        let Some(lid) = lyrics_id else {
            return Ok(None);
        };

        // try ANDROID_MUSIC timed lyrics first, then fall back to WEB_REMIX plain lyrics
        if let Ok(timed_raw) = self
            .browse(&lid, None, None, ClientKind::AndroidMusic, None)
            .await
            && let Some(lyr) = parsers::parse_yt_lyrics(&timed_raw)
        {
            return Ok(Some(lyr));
        }

        let plain_raw = self
            .browse(&lid, None, None, ClientKind::WebRemix, None)
            .await?;
        Ok(parsers::parse_yt_lyrics(&plain_raw))
    }
}
