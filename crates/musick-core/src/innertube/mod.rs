pub mod endpoints;
pub mod extract;
pub mod parsers;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::debug;

use crate::auth::AuthManager;
use crate::domain::{
    Album, AlbumSummary, Artist, ArtistRef, ArtistSummary, HomeFeed, Lyrics, Playlist,
    PlaylistSummary, SearchCategory, SearchItem, Track, search_item_key,
};
use crate::error::{CoreError, CoreResult};
use crate::storage::CacheManager;

pub use endpoints::{
    ClientKind, INNERTUBE_BASE_URL, PlaybackTelemetryEvent, PlaybackTracking, PlaylistEditAction,
    PlaylistEditResult, RadioBatch, SongRating, build_context, search_filter_params,
};

#[async_trait]
pub trait CatalogApi: Send + Sync {
    async fn home_feed(&self, region: &str) -> CoreResult<HomeFeed>;
    async fn search(
        &self,
        category: SearchCategory,
        query: &str,
        limit: usize,
    ) -> CoreResult<Vec<SearchItem>>;
    async fn suggestions(&self, query: &str) -> CoreResult<Vec<String>>;
    async fn artist(&self, browse_id: &str) -> CoreResult<Artist>;
    async fn artist_releases(
        &self,
        browse_id: &str,
        params: &str,
        kind: &str,
    ) -> CoreResult<Vec<AlbumSummary>>;
    async fn album(&self, browse_id: &str) -> CoreResult<Album>;
    async fn playlist(&self, playlist_id: &str, limit: usize) -> CoreResult<Playlist>;
    async fn radio(
        &self,
        video_id: Option<&str>,
        playlist_id: Option<&str>,
        continuation: Option<&str>,
        limit: usize,
    ) -> CoreResult<RadioBatch>;
    async fn song_related(&self, video_id: &str) -> CoreResult<Vec<SearchItem>>;
    async fn yt_lyrics(&self, video_id: &str) -> CoreResult<Option<Lyrics>>;

    // ── Authenticated Library Read Endpoints ────────────────────────────────
    async fn library_playlists(&self) -> CoreResult<Vec<PlaylistSummary>>;
    async fn library_liked_songs(&self, limit: usize) -> CoreResult<Vec<Track>>;
    async fn library_albums(&self) -> CoreResult<Vec<AlbumSummary>>;
    async fn library_artists(&self) -> CoreResult<Vec<ArtistSummary>>;

    // ── Authenticated 2-Way Sync Write Endpoints ────────────────────────────
    async fn create_playlist(
        &self,
        title: &str,
        description: &str,
        video_ids: &[String],
    ) -> CoreResult<String>;
    async fn edit_playlist(
        &self,
        remote_playlist_id: &str,
        actions: &[PlaylistEditAction],
    ) -> CoreResult<PlaylistEditResult>;
    async fn delete_playlist(&self, remote_playlist_id: &str) -> CoreResult<()>;
    async fn rate_song(&self, video_id: &str, rating: SongRating) -> CoreResult<()>;
    async fn set_artist_subscription(&self, channel_id: &str, subscribe: bool) -> CoreResult<()>;
    async fn set_album_library_status(&self, browse_id: &str, save: bool) -> CoreResult<()>;
    async fn set_playlist_library_status(&self, playlist_id: &str, save: bool) -> CoreResult<()>;

    // ── Strictly Opt-In Algorithm Playback Telemetry ────────────────────────
    async fn fetch_playback_tracking(
        &self,
        video_id: &str,
        report_enabled: bool,
    ) -> CoreResult<Option<PlaybackTracking>>;
    async fn report_playback(
        &self,
        tracking: &PlaybackTracking,
        cpn: &str,
        event: PlaybackTelemetryEvent,
        report_enabled: bool,
    ) -> CoreResult<()>;
}

pub struct InnerTubeClient {
    http: reqwest::Client,
    auth: Arc<AuthManager>,
    cache: Arc<CacheManager>,
    region: RwLock<String>,
    base_url: String,
}

impl InnerTubeClient {
    pub fn new(
        http: reqwest::Client,
        auth: Arc<AuthManager>,
        cache: Arc<CacheManager>,
        region: impl Into<String>,
    ) -> Self {
        Self {
            http,
            auth,
            cache,
            region: RwLock::new(region.into()),
            base_url: INNERTUBE_BASE_URL.to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn set_region(&self, region: &str) {
        if let Ok(mut guard) = self.region.write() {
            *guard = region.to_string();
        }
    }

    pub fn region(&self) -> String {
        self.region
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|_| "ID".to_string())
    }

    fn cache_scope(&self) -> String {
        format!("{}:{}", self.region(), self.auth.account_key())
    }

    pub fn build_request_body(
        &self,
        client_kind: ClientKind,
        region: &str,
        mut payload: serde_json::Map<String, Value>,
    ) -> Value {
        let vd = self.cache.get_visitor_data();
        let on_behalf = self.auth.on_behalf_of_user();
        let ctx = build_context(client_kind, region, vd.as_deref(), on_behalf.as_deref());
        payload.insert("context".to_string(), ctx);
        Value::Object(payload)
    }

    async fn post_json(
        &self,
        endpoint: &str,
        client_kind: ClientKind,
        region_override: Option<&str>,
        payload: serde_json::Map<String, Value>,
    ) -> CoreResult<Value> {
        let region = region_override
            .map(str::to_string)
            .unwrap_or_else(|| self.region());
        let body = self.build_request_body(client_kind, &region, payload);
        let url = format!("{}/{endpoint}?prettyPrint=false", self.base_url);

        let ua = match client_kind {
            ClientKind::WebRemix => endpoints::WEB_USER_AGENT,
            ClientKind::AndroidMusic => endpoints::ANDROID_MUSIC_USER_AGENT,
        };

        let mut req = self
            .http
            .post(&url)
            .header("User-Agent", ua)
            .header("Content-Type", "application/json")
            .header("Accept-Language", "en-US,en;q=0.9");

        if let Some(vd) = self.cache.get_visitor_data() {
            req = req.header("X-Goog-Visitor-Id", vd);
        }

        // Strict privacy boundary: AuthManager::apply_headers is a no-op in Anonymous mode.
        req = self.auth.apply_headers(req).await?;
        let resp = req.json(&body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(CoreError::Catalog(format!(
                "InnerTube {endpoint} HTTP {status}: {}",
                &txt[..txt.len().min(200)]
            )));
        }

        let json_val: Value = resp.json().await?;
        if let Some(new_vd) = parsers::extract_visitor_data(&json_val) {
            self.cache.set_visitor_data(&new_vd);
        }
        Ok(json_val)
    }

    async fn browse(
        &self,
        browse_id: &str,
        params: Option<&str>,
        continuation: Option<&str>,
        client_kind: ClientKind,
        region_override: Option<&str>,
    ) -> CoreResult<Value> {
        let mut map = serde_json::Map::new();
        if let Some(cont) = continuation {
            map.insert("continuation".into(), json!(cont));
        } else {
            map.insert("browseId".into(), json!(browse_id));
            if let Some(p) = params {
                map.insert("params".into(), json!(p));
            }
        }
        self.post_json("browse", client_kind, region_override, map)
            .await
    }

    fn require_auth(&self, action: &str) -> CoreResult<()> {
        if !self.auth.is_authenticated() {
            return Err(CoreError::Auth(format!(
                "{action} requires OAuth or Browser authentication"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl CatalogApi for InnerTubeClient {
    async fn home_feed(&self, region: &str) -> CoreResult<HomeFeed> {
        let cache_key = format!("home:{region}:{}", self.auth.account_key());
        if let Some(cached) = self.cache.get_catalog::<HomeFeed>(&cache_key) {
            return Ok(cached);
        }

        let (home_res, new_rel_res, explore_res, charts_res) = tokio::join!(
            self.browse(
                "FEmusic_home",
                None,
                None,
                ClientKind::WebRemix,
                Some(region)
            ),
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

        let home_v = home_res.ok();
        let new_rel_v = new_rel_res.ok();
        let explore_v = explore_res.ok();
        let charts_v = charts_res.ok();

        if home_v.is_none() && new_rel_v.is_none() && explore_v.is_none() && charts_v.is_none() {
            return Err(CoreError::Catalog(
                "Unable to load YouTube Music home/explore feeds".into(),
            ));
        }

        let feed = parsers::parse_home_feed(
            home_v.as_ref(),
            new_rel_v.as_ref(),
            explore_v.as_ref(),
            charts_v.as_ref(),
        );
        self.cache.put_catalog(&cache_key, &feed, Some(600));
        Ok(feed)
    }

    async fn search(
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
            let mut map_songs = serde_json::Map::new();
            map_songs.insert("query".into(), json!(q));
            if let Some(p) = search_filter_params(SearchCategory::Songs) {
                map_songs.insert("params".into(), json!(p));
            }

            let (res_all, res_songs) = tokio::join!(
                self.post_json("search", ClientKind::WebRemix, None, map_all),
                self.post_json("search", ClientKind::WebRemix, None, map_songs),
            );

            if res_songs.is_err()
                && let Err(e) = res_all
            {
                return Err(e);
            }

            let parsed_all = res_all
                .map(|v| parsers::parse_search_results(&v))
                .unwrap_or_default();
            let parsed_songs = res_songs
                .map(|v| parsers::parse_search_results(&v))
                .unwrap_or_default();

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
            parsers::parse_search_results(&raw)
        };

        let capped: Vec<SearchItem> = out.into_iter().take(limit).collect();
        self.cache.put_catalog(&cache_key, &capped, Some(600));
        Ok(capped)
    }

    async fn suggestions(&self, query: &str) -> CoreResult<Vec<String>> {
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
        self.cache.put_catalog(&cache_key, &sug, Some(300));
        Ok(sug)
    }

    async fn artist(&self, browse_id: &str) -> CoreResult<Artist> {
        let clean_bid = browse_id.strip_prefix("MPLA").unwrap_or(browse_id);
        let cache_key = format!("artist:{}:{clean_bid}", self.region());
        if let Some(cached) = self.cache.get_catalog::<Artist>(&cache_key) {
            return Ok(cached);
        }

        let raw = self
            .browse(clean_bid, None, None, ClientKind::WebRemix, None)
            .await?;
        let mut art = parsers::parse_artist(clean_bid, &raw);
        let mut plays_map = parsers::extract_plays_from_browse(&raw);

        let songs_pl_id = art.songs_playlist_id.clone();
        let videos_pl_id = art.videos_playlist_id.clone();
        let albums_bid = art
            .albums_browse_id
            .clone()
            .unwrap_or_else(|| clean_bid.to_string());
        let albums_params = art.albums_params.clone();
        let singles_bid = art
            .singles_browse_id
            .clone()
            .unwrap_or_else(|| clean_bid.to_string());
        let singles_params = art.singles_params.clone();

        let (songs_pl, videos_pl, full_albums, full_singles) = tokio::join!(
            async {
                if let Some(pid) = songs_pl_id.as_deref() {
                    self.playlist(pid, 150).await.ok()
                } else {
                    None
                }
            },
            async {
                if let Some(pid) = videos_pl_id.as_deref() {
                    self.playlist(pid, 100).await.ok()
                } else {
                    None
                }
            },
            async {
                if let Some(params) = albums_params.as_deref() {
                    self.artist_releases(&albums_bid, params, "Album")
                        .await
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            },
            async {
                if let Some(params) = singles_params.as_deref() {
                    self.artist_releases(&singles_bid, params, "Single")
                        .await
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            },
        );

        let fallback_artists = vec![ArtistRef {
            name: art.summary.name.clone(),
            id: Some(clean_bid.to_string()),
        }];

        if let Some(pl) = songs_pl
            && !pl.tracks.is_empty()
        {
            for t in &pl.tracks {
                if let Some(p) = &t.plays {
                    plays_map
                        .entry(t.video_id.clone())
                        .or_insert_with(|| p.clone());
                }
            }
            let orig_map: HashMap<String, Track> = art
                .songs
                .iter()
                .map(|t| (t.video_id.clone(), t.clone()))
                .collect();
            let mut seen = HashSet::new();
            let mut enriched = Vec::new();
            for mut t in pl.tracks {
                seen.insert(t.video_id.clone());
                if let Some(orig) = orig_map.get(&t.video_id) {
                    if t.artists.is_empty() {
                        t.artists = orig.artists.clone();
                    }
                    if t.album.is_none() {
                        t.album = orig.album.clone();
                    }
                    if t.album_id.is_none() {
                        t.album_id = orig.album_id.clone();
                    }
                    if t.duration.is_none() {
                        t.duration = orig.duration;
                    }
                    if t.thumbs.is_empty() {
                        t.thumbs = orig.thumbs.clone();
                    }
                    if t.plays.is_none() {
                        t.plays = orig.plays.clone();
                    }
                    if t.video_type.is_none() {
                        t.video_type = orig.video_type.clone();
                    }
                }
                if t.artists.is_empty() {
                    t.artists = fallback_artists.clone();
                }
                if let Some(p) = plays_map.get(&t.video_id) {
                    t.plays = Some(p.clone());
                }
                enriched.push(t);
            }
            for mut orig in art.songs {
                if seen.insert(orig.video_id.clone()) {
                    if orig.artists.is_empty() {
                        orig.artists = fallback_artists.clone();
                    }
                    if let Some(p) = plays_map.get(&orig.video_id) {
                        orig.plays = Some(p.clone());
                    }
                    enriched.push(orig);
                }
            }
            art.songs = enriched;
        } else {
            for t in &mut art.songs {
                if t.artists.is_empty() {
                    t.artists = fallback_artists.clone();
                }
                if let Some(p) = plays_map.get(&t.video_id) {
                    t.plays = Some(p.clone());
                }
            }
        }

        if let Some(vpl) = videos_pl
            && !vpl.tracks.is_empty()
        {
            let mut seen = HashSet::new();
            let mut vids = Vec::new();
            for mut t in vpl.tracks {
                if seen.insert(t.video_id.clone()) {
                    if t.artists.is_empty() {
                        t.artists = fallback_artists.clone();
                    }
                    if t.video_type.is_none() {
                        t.video_type = Some("MUSIC_VIDEO_TYPE_OMV".into());
                    }
                    vids.push(t);
                }
            }
            for orig in art.videos {
                if seen.insert(orig.video_id.clone()) {
                    vids.push(orig);
                }
            }
            art.videos = vids;
        }

        if !full_albums.is_empty() {
            art.albums = full_albums;
        }
        for al in &mut art.albums {
            if al.artists.is_empty() {
                al.artists = fallback_artists.clone();
            }
        }

        if !full_singles.is_empty() {
            art.singles = full_singles;
        }
        for s in &mut art.singles {
            if s.artists.is_empty() {
                s.artists = fallback_artists.clone();
            }
        }

        self.cache.put_catalog(&cache_key, &art, Some(1800));
        Ok(art)
    }

    async fn artist_releases(
        &self,
        browse_id: &str,
        params: &str,
        kind: &str,
    ) -> CoreResult<Vec<AlbumSummary>> {
        let cache_key = format!("releases:{}:{browse_id}:{params}", self.region());
        if let Some(cached) = self.cache.get_catalog::<Vec<AlbumSummary>>(&cache_key) {
            return Ok(cached);
        }
        let raw = self
            .browse(browse_id, Some(params), None, ClientKind::WebRemix, None)
            .await?;
        let items = parsers::parse_artist_releases(&raw, kind);
        self.cache.put_catalog(&cache_key, &items, Some(1800));
        Ok(items)
    }

    async fn album(&self, browse_id: &str) -> CoreResult<Album> {
        let cache_key = format!("album:{}:{browse_id}", self.region());
        if let Some(cached) = self.cache.get_catalog::<Album>(&cache_key) {
            return Ok(cached);
        }
        let raw = self
            .browse(browse_id, None, None, ClientKind::WebRemix, None)
            .await?;
        let album = parsers::parse_album(browse_id, &raw);
        self.cache.put_catalog(&cache_key, &album, Some(1800));
        Ok(album)
    }

    async fn playlist(&self, playlist_id: &str, limit: usize) -> CoreResult<Playlist> {
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

    async fn radio(
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

    async fn song_related(&self, video_id: &str) -> CoreResult<Vec<SearchItem>> {
        let mut map = serde_json::Map::new();
        map.insert("videoId".into(), json!(video_id));
        map.insert("isAudioOnly".into(), json!(true));
        let raw = self
            .post_json("next", ClientKind::WebRemix, None, map)
            .await?;
        let (_, _, related_id) = parsers::parse_watch_playlist(&raw);
        let Some(rel_bid) = related_id else {
            return Ok(Vec::new());
        };
        let rel_raw = self
            .browse(&rel_bid, None, None, ClientKind::WebRemix, None)
            .await?;
        let mut out = Vec::new();
        let mut carousels = Vec::new();
        extract::find_all_by_key(&rel_raw, "musicCarouselShelfRenderer", &mut carousels);
        for shelf in carousels {
            if let Some(contents) = extract::get_array(shelf, &["contents"]) {
                for c in contents {
                    if let Some(rli) = c.get("musicResponsiveListItemRenderer")
                        && let Some(item) = parsers::parse_responsive_list_item(rli)
                    {
                        out.push(item);
                    } else if let Some(tri) = c.get("musicTwoRowItemRenderer")
                        && let Some(item) = parsers::parse_two_row_item(tri)
                    {
                        out.push(item);
                    }
                }
            }
        }
        Ok(out)
    }

    async fn yt_lyrics(&self, video_id: &str) -> CoreResult<Option<Lyrics>> {
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

        // Try ANDROID_MUSIC timed lyrics first, then fall back to WEB_REMIX plain lyrics.
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

    async fn library_playlists(&self) -> CoreResult<Vec<PlaylistSummary>> {
        self.require_auth("library_playlists")?;
        let raw = self
            .browse(
                "FEmusic_liked_playlists",
                None,
                None,
                ClientKind::WebRemix,
                None,
            )
            .await?;
        Ok(parsers::parse_library_playlists(&raw))
    }

    async fn library_liked_songs(&self, limit: usize) -> CoreResult<Vec<Track>> {
        self.require_auth("library_liked_songs")?;
        let pl = self.playlist("LM", limit).await?;
        Ok(pl.tracks)
    }

    async fn library_albums(&self) -> CoreResult<Vec<AlbumSummary>> {
        self.require_auth("library_albums")?;
        let raw = self
            .browse(
                "FEmusic_liked_albums",
                None,
                None,
                ClientKind::WebRemix,
                None,
            )
            .await?;
        Ok(parsers::parse_library_albums(&raw))
    }

    async fn library_artists(&self) -> CoreResult<Vec<ArtistSummary>> {
        self.require_auth("library_artists")?;
        let raw = self
            .browse(
                "FEmusic_library_corpus_artists",
                None,
                None,
                ClientKind::WebRemix,
                None,
            )
            .await?;
        Ok(parsers::parse_library_artists(&raw))
    }

    async fn create_playlist(
        &self,
        title: &str,
        description: &str,
        video_ids: &[String],
    ) -> CoreResult<String> {
        self.require_auth("create_playlist")?;
        let mut map = serde_json::Map::new();
        map.insert("title".into(), json!(title));
        map.insert("description".into(), json!(description));
        map.insert("privacyStatus".into(), json!("PRIVATE"));
        if !video_ids.is_empty() {
            map.insert("videoIds".into(), json!(video_ids));
        }
        let raw = self
            .post_json("playlist/create", ClientKind::WebRemix, None, map)
            .await?;
        raw.get("playlistId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| CoreError::Catalog("playlist/create response missing playlistId".into()))
    }

    async fn edit_playlist(
        &self,
        remote_playlist_id: &str,
        actions: &[PlaylistEditAction],
    ) -> CoreResult<PlaylistEditResult> {
        self.require_auth("edit_playlist")?;
        if actions.is_empty() {
            return Ok(PlaylistEditResult::default());
        }
        let clean_pid = remote_playlist_id
            .strip_prefix("VL")
            .unwrap_or(remote_playlist_id);
        let mut map = serde_json::Map::new();
        map.insert("playlistId".into(), json!(clean_pid));
        map.insert(
            "actions".into(),
            Value::Array(
                actions
                    .iter()
                    .map(PlaylistEditAction::to_innertube_json)
                    .collect(),
            ),
        );
        let raw = self
            .post_json("browse/edit_playlist", ClientKind::WebRemix, None, map)
            .await?;
        Ok(parsers::parse_playlist_edit_results(&raw))
    }

    async fn delete_playlist(&self, remote_playlist_id: &str) -> CoreResult<()> {
        self.require_auth("delete_playlist")?;
        let clean_pid = remote_playlist_id
            .strip_prefix("VL")
            .unwrap_or(remote_playlist_id);
        let mut map = serde_json::Map::new();
        map.insert("playlistId".into(), json!(clean_pid));
        let _ = self
            .post_json("playlist/delete", ClientKind::WebRemix, None, map)
            .await?;
        Ok(())
    }

    async fn rate_song(&self, video_id: &str, rating: SongRating) -> CoreResult<()> {
        self.require_auth("rate_song")?;
        let mut map = serde_json::Map::new();
        map.insert("target".into(), json!({ "videoId": video_id }));
        let _ = self
            .post_json(rating.endpoint(), ClientKind::WebRemix, None, map)
            .await?;
        Ok(())
    }

    async fn set_artist_subscription(&self, channel_id: &str, subscribe: bool) -> CoreResult<()> {
        self.require_auth("set_artist_subscription")?;
        let mut map = serde_json::Map::new();
        map.insert("channelIds".into(), json!([channel_id]));
        let ep = if subscribe {
            "subscription/subscribe"
        } else {
            "subscription/unsubscribe"
        };
        let _ = self.post_json(ep, ClientKind::WebRemix, None, map).await?;
        Ok(())
    }

    async fn set_album_library_status(&self, browse_id: &str, save: bool) -> CoreResult<()> {
        self.require_auth("set_album_library_status")?;
        let mut map = serde_json::Map::new();
        map.insert("target".into(), json!({ "playlistId": browse_id }));
        let ep = if save { "like/like" } else { "like/removelike" };
        let _ = self.post_json(ep, ClientKind::WebRemix, None, map).await?;
        Ok(())
    }

    async fn set_playlist_library_status(&self, playlist_id: &str, save: bool) -> CoreResult<()> {
        self.require_auth("set_playlist_library_status")?;
        let clean_pid = playlist_id.strip_prefix("VL").unwrap_or(playlist_id);
        let mut map = serde_json::Map::new();
        map.insert("target".into(), json!({ "playlistId": clean_pid }));
        let ep = if save { "like/like" } else { "like/removelike" };
        let _ = self.post_json(ep, ClientKind::WebRemix, None, map).await?;
        Ok(())
    }

    async fn fetch_playback_tracking(
        &self,
        video_id: &str,
        report_enabled: bool,
    ) -> CoreResult<Option<PlaybackTracking>> {
        // Hard privacy gate: strictly no-op in Anonymous mode or when opt-in toggle is off.
        if !report_enabled || !self.auth.is_authenticated() {
            return Ok(None);
        }
        let mut map = serde_json::Map::new();
        map.insert("videoId".into(), json!(video_id));
        let raw = self
            .post_json("player", ClientKind::WebRemix, None, map)
            .await?;
        Ok(parsers::parse_player_tracking(video_id, &raw))
    }

    async fn report_playback(
        &self,
        tracking: &PlaybackTracking,
        cpn: &str,
        event: PlaybackTelemetryEvent,
        report_enabled: bool,
    ) -> CoreResult<()> {
        // Hard privacy gate: strictly no-op in Anonymous mode or when opt-in toggle is off.
        if !report_enabled || !self.auth.is_authenticated() {
            return Ok(());
        }

        let (base_url, query_pairs): (&str, Vec<(&str, String)>) = match event {
            PlaybackTelemetryEvent::Start => {
                let Some(url) = tracking.playback_url.as_deref() else {
                    return Ok(());
                };
                (
                    url,
                    vec![
                        ("cpn", cpn.to_string()),
                        ("ver", "2".to_string()),
                        ("c", endpoints::WEB_REMIX_CLIENT_NAME.to_string()),
                        ("cver", endpoints::WEB_REMIX_CLIENT_VERSION.to_string()),
                        ("docid", tracking.video_id.clone()),
                    ],
                )
            }
            PlaybackTelemetryEvent::Watchtime {
                elapsed_secs,
                duration_secs,
                playing,
            } => {
                let Some(url) = tracking.watchtime_url.as_deref() else {
                    return Ok(());
                };
                let len = if duration_secs > 0.0 {
                    duration_secs
                } else {
                    f64::from(tracking.length_secs)
                };
                (
                    url,
                    vec![
                        ("cpn", cpn.to_string()),
                        ("ver", "2".to_string()),
                        ("c", endpoints::WEB_REMIX_CLIENT_NAME.to_string()),
                        ("cver", endpoints::WEB_REMIX_CLIENT_VERSION.to_string()),
                        ("docid", tracking.video_id.clone()),
                        ("cmt", format!("{elapsed_secs:.3}")),
                        ("et", format!("{elapsed_secs:.3}")),
                        ("st", "0.000".to_string()),
                        ("len", format!("{len:.3}")),
                        (
                            "state",
                            if playing { "playing" } else { "paused" }.to_string(),
                        ),
                    ],
                )
            }
        };

        let req = self
            .http
            .get(base_url)
            .header("User-Agent", endpoints::WEB_USER_AGENT)
            .query(&query_pairs);
        let req = self.auth.apply_headers(req).await?;
        let _ = req.send().await?;
        Ok(())
    }
}
