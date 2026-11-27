mod artist_api;
mod catalog;
pub mod endpoints;
pub mod extract;
mod library_api;
pub mod parsers;
mod telemetry;

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::auth::AuthManager;
use crate::config::AuthMode;
use crate::domain::{
    Album, AlbumSummary, Artist, ArtistSummary, DiscoverFeed, Lyrics, Playlist, PlaylistSummary,
    SearchCategory, SearchItem, Track,
};
use crate::error::{CoreError, CoreResult};
use crate::storage::CacheManager;

pub use endpoints::{
    ClientKind, INNERTUBE_BASE_URL, PlaybackTelemetryEvent, PlaybackTracking, PlaylistEditAction,
    PlaylistEditResult, RadioBatch, SongRating, build_context, search_filter_params,
};

#[async_trait]
pub trait CatalogApi: Send + Sync {
    fn set_region(&self, _region: &str) {}
    async fn discover_feed(&self, region: &str) -> CoreResult<DiscoverFeed>;
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
        params: Option<&str>,
        kind: &str,
    ) -> CoreResult<Vec<AlbumSummary>>;
    async fn album(&self, browse_id: &str) -> CoreResult<Album>;
    async fn playlist(&self, playlist_id: &str, limit: usize) -> CoreResult<Playlist>;
    async fn playlist_uncached(&self, playlist_id: &str, limit: usize) -> CoreResult<Playlist> {
        self.playlist(playlist_id, limit).await
    }
    async fn radio(
        &self,
        video_id: Option<&str>,
        playlist_id: Option<&str>,
        continuation: Option<&str>,
        limit: usize,
    ) -> CoreResult<RadioBatch>;
    async fn song_related(&self, video_id: &str) -> CoreResult<Vec<SearchItem>>;
    async fn yt_lyrics(&self, video_id: &str) -> CoreResult<Option<Lyrics>>;

    async fn library_playlists(&self) -> CoreResult<Vec<PlaylistSummary>>;
    async fn library_liked_songs(&self, limit: usize) -> CoreResult<Vec<Track>>;
    async fn library_albums(&self) -> CoreResult<Vec<AlbumSummary>>;
    async fn library_artists(&self) -> CoreResult<Vec<ArtistSummary>>;

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
    pub(super) http: reqwest::Client,
    pub(super) auth: Arc<AuthManager>,
    pub(super) cache: Arc<CacheManager>,
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

    pub(super) fn cache_scope(&self) -> String {
        format!("{}:{}", self.region(), self.auth.account_key())
    }

    fn active_visitor_data(&self) -> Option<String> {
        if let Some(vd) = self.auth.browser_visitor_id() {
            return Some(vd);
        }
        self.cache.get_visitor_data()
    }

    pub fn build_request_body(
        &self,
        client_kind: ClientKind,
        region: &str,
        mut payload: serde_json::Map<String, Value>,
    ) -> Value {
        let vd = self.active_visitor_data();
        let on_behalf = self.auth.on_behalf_of_user();
        let ctx = build_context(client_kind, region, vd.as_deref(), on_behalf.as_deref());
        payload.insert("context".to_string(), ctx);
        Value::Object(payload)
    }

    pub(super) async fn post_json(
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
        let auth_mode = self.auth.mode();
        let base = &self.base_url;
        let url = if auth_mode == AuthMode::Browser {
            let key = endpoints::YTM_API_KEY;
            format!("{base}/{endpoint}?alt=json&key={key}")
        } else {
            format!("{base}/{endpoint}?alt=json")
        };

        let (ua, client_id, client_ver) = match client_kind {
            ClientKind::WebRemix => (
                endpoints::WEB_USER_AGENT,
                endpoints::WEB_REMIX_CLIENT_ID,
                endpoints::web_remix_client_version(),
            ),
            ClientKind::AndroidMusic => (
                endpoints::ANDROID_MUSIC_USER_AGENT,
                endpoints::ANDROID_MUSIC_CLIENT_ID,
                endpoints::ANDROID_MUSIC_CLIENT_VERSION.to_string(),
            ),
        };

        let mut req = self
            .http
            .post(&url)
            .header("User-Agent", ua)
            .header("Accept", "*/*")
            .header("Content-Type", "application/json")
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("X-YouTube-Client-Name", client_id)
            .header("X-YouTube-Client-Version", &client_ver);

        if client_kind == ClientKind::WebRemix {
            req = req.header("Referer", endpoints::YTM_REFERER);
            if auth_mode != AuthMode::Browser {
                req = req
                    .header("Origin", endpoints::YTM_ORIGIN)
                    .header("X-Origin", endpoints::YTM_ORIGIN);
            }
        }

        if let Some(vd) = self.active_visitor_data() {
            req = req.header("X-Goog-Visitor-Id", vd);
        }

        // strict privacy boundary: apply_headers is a no-op in anonymous mode
        req = self.auth.apply_headers(req).await?;
        let resp = req.json(&body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            let lower = txt.to_ascii_lowercase();
            if status.as_u16() == 403
                && (lower.contains("automated queries")
                    || lower.contains("<title>sorry")
                    || lower.contains("captcha"))
            {
                let msg = if self.auth.is_authenticated() {
                    "YouTube Music blocked requests from this network (HTTP 403) — try switching network/VPN, refreshing Browser Auth, or retrying later"
                } else {
                    "YouTube Music blocked anonymous requests from this network (HTTP 403) — try again later, switch network, or sign in in 5 Settings"
                };
                return Err(CoreError::Catalog(msg.to_string()));
            }
            if status.as_u16() == 429 {
                return Err(CoreError::Catalog(
                    "YouTube Music rate-limited requests (HTTP 429) — wait a moment and retry"
                        .to_string(),
                ));
            }
            if lower.trim_start().starts_with("<!doctype")
                || lower.trim_start().starts_with("<html")
            {
                return Err(CoreError::Catalog(format!(
                    "InnerTube {endpoint} returned HTTP {status}"
                )));
            }
            let snippet: String = txt.chars().take(200).collect();
            return Err(CoreError::Catalog(format!(
                "InnerTube {endpoint} HTTP {status}: {snippet}"
            )));
        }

        let json_val: Value = resp.json().await?;
        if let Some(new_vd) = parsers::extract_visitor_data(&json_val) {
            self.cache.set_visitor_data(&new_vd);
        }
        Ok(json_val)
    }

    pub(super) async fn browse(
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

    pub(super) fn require_auth(&self, action: &str) -> CoreResult<()> {
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
    fn set_region(&self, region: &str) {
        InnerTubeClient::set_region(self, region);
    }

    async fn discover_feed(&self, region: &str) -> CoreResult<DiscoverFeed> {
        self.discover_feed_impl(region).await
    }

    async fn search(
        &self,
        category: SearchCategory,
        query: &str,
        limit: usize,
    ) -> CoreResult<Vec<SearchItem>> {
        self.search_impl(category, query, limit).await
    }

    async fn suggestions(&self, query: &str) -> CoreResult<Vec<String>> {
        self.suggestions_impl(query).await
    }

    async fn artist(&self, browse_id: &str) -> CoreResult<Artist> {
        self.artist_impl(browse_id).await
    }

    async fn artist_releases(
        &self,
        browse_id: &str,
        params: Option<&str>,
        kind: &str,
    ) -> CoreResult<Vec<AlbumSummary>> {
        self.artist_releases_impl(browse_id, params, kind).await
    }

    async fn album(&self, browse_id: &str) -> CoreResult<Album> {
        self.album_impl(browse_id).await
    }

    async fn playlist(&self, playlist_id: &str, limit: usize) -> CoreResult<Playlist> {
        self.playlist_impl(playlist_id, limit).await
    }

    async fn playlist_uncached(&self, playlist_id: &str, limit: usize) -> CoreResult<Playlist> {
        self.fetch_playlist_uncached(playlist_id, limit).await
    }

    async fn radio(
        &self,
        video_id: Option<&str>,
        playlist_id: Option<&str>,
        continuation: Option<&str>,
        limit: usize,
    ) -> CoreResult<RadioBatch> {
        self.radio_impl(video_id, playlist_id, continuation, limit)
            .await
    }

    async fn song_related(&self, video_id: &str) -> CoreResult<Vec<SearchItem>> {
        self.song_related_impl(video_id).await
    }

    async fn yt_lyrics(&self, video_id: &str) -> CoreResult<Option<Lyrics>> {
        self.yt_lyrics_impl(video_id).await
    }

    async fn library_playlists(&self) -> CoreResult<Vec<PlaylistSummary>> {
        self.library_playlists_impl().await
    }

    async fn library_liked_songs(&self, limit: usize) -> CoreResult<Vec<Track>> {
        self.library_liked_songs_impl(limit).await
    }

    async fn library_albums(&self) -> CoreResult<Vec<AlbumSummary>> {
        self.library_albums_impl().await
    }

    async fn library_artists(&self) -> CoreResult<Vec<ArtistSummary>> {
        self.library_artists_impl().await
    }

    async fn create_playlist(
        &self,
        title: &str,
        description: &str,
        video_ids: &[String],
    ) -> CoreResult<String> {
        self.create_playlist_impl(title, description, video_ids)
            .await
    }

    async fn edit_playlist(
        &self,
        remote_playlist_id: &str,
        actions: &[PlaylistEditAction],
    ) -> CoreResult<PlaylistEditResult> {
        self.edit_playlist_impl(remote_playlist_id, actions).await
    }

    async fn delete_playlist(&self, remote_playlist_id: &str) -> CoreResult<()> {
        self.delete_playlist_impl(remote_playlist_id).await
    }

    async fn rate_song(&self, video_id: &str, rating: SongRating) -> CoreResult<()> {
        self.rate_song_impl(video_id, rating).await
    }

    async fn set_artist_subscription(&self, channel_id: &str, subscribe: bool) -> CoreResult<()> {
        self.set_artist_subscription_impl(channel_id, subscribe)
            .await
    }

    async fn set_album_library_status(&self, browse_id: &str, save: bool) -> CoreResult<()> {
        self.set_album_library_status_impl(browse_id, save).await
    }

    async fn set_playlist_library_status(&self, playlist_id: &str, save: bool) -> CoreResult<()> {
        self.set_playlist_library_status_impl(playlist_id, save)
            .await
    }

    async fn fetch_playback_tracking(
        &self,
        video_id: &str,
        report_enabled: bool,
    ) -> CoreResult<Option<PlaybackTracking>> {
        self.fetch_playback_tracking_impl(video_id, report_enabled)
            .await
    }

    async fn report_playback(
        &self,
        tracking: &PlaybackTracking,
        cpn: &str,
        event: PlaybackTelemetryEvent,
        report_enabled: bool,
    ) -> CoreResult<()> {
        self.report_playback_impl(tracking, cpn, event, report_enabled)
            .await
    }
}
