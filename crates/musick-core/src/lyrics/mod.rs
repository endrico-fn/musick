mod parsers;
mod translate;

use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde_json::Value;
use tokio::process::Command;
use tracing::debug;

use crate::config::{LyricsSource, Settings};
use crate::domain::{LyricLine, Lyrics, Track, clean_media_title, is_valid_video_id};
use crate::innertube::CatalogApi;
use crate::storage::CacheManager;

pub use parsers::{
    active_line_index, parse_json3_captions, parse_lrc, parse_vtt_captions,
    pick_best_lrclib_candidate,
};
pub use translate::{looks_indonesian, should_skip_translation};

const LRCLIB_BASE: &str = "https://lrclib.net/api";
pub(super) const USER_AGENT: &str = concat!(
    "musick/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/endrico-fn/musick)"
);

pub struct LyricsService {
    catalog: Option<Arc<dyn CatalogApi>>,
    pub(super) cache: Arc<CacheManager>,
    pub(super) http: reqwest::Client,
    settings: RwLock<Settings>,
    allow_ytdlp_captions: bool,
}

impl LyricsService {
    pub fn new(
        catalog: Option<Arc<dyn CatalogApi>>,
        cache: Arc<CacheManager>,
        http: reqwest::Client,
        settings: Settings,
    ) -> Self {
        Self {
            catalog,
            cache,
            http,
            settings: RwLock::new(settings),
            allow_ytdlp_captions: true,
        }
    }

    pub fn with_ytdlp_captions(mut self, enabled: bool) -> Self {
        self.allow_ytdlp_captions = enabled;
        self
    }

    pub fn update_settings(&self, settings: Settings) {
        if let Ok(mut guard) = self.settings.write() {
            *guard = settings;
        }
    }

    pub(super) fn current_settings(&self) -> Settings {
        self.settings.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub(super) fn cache_key(video_id: &str, source: LyricsSource) -> String {
        format!("{video_id}:{}", source.as_str())
    }

    pub async fn fetch(&self, track: &Track) -> Lyrics {
        let settings = self.current_settings();
        if !settings.lyrics.enabled {
            return Lyrics::default();
        }
        let key = Self::cache_key(&track.video_id, settings.lyrics.source);
        let mut lyr = if let Some(hit) = self.cache.get_lyrics(&key) {
            hit
        } else {
            let loaded = self.load_uncached(track, &settings).await;
            if !loaded.is_empty() {
                self.cache.put_lyrics(&key, &loaded);
            }
            loaded
        };

        if !lyr.is_empty() && settings.lyrics.translate_to_id && lyr.translation_lang != "id" {
            lyr = self
                .translate_lyrics(lyr, Some(&track.video_id), "id")
                .await;
        }

        if lyr.is_empty() {
            return Lyrics::default();
        }
        if lyr.synced || !settings.lyrics.synced_only {
            lyr
        } else {
            Lyrics::default()
        }
    }

    async fn load_uncached(&self, track: &Track, settings: &Settings) -> Lyrics {
        match settings.lyrics.source {
            LyricsSource::Lrclib => self.fetch_lrclib(track).await,
            LyricsSource::Youtube => {
                let mut lyr = self.fetch_yt_music(track).await;
                if lyr.is_empty() || !lyr.synced {
                    let cap = self
                        .fetch_yt_captions(track, &settings.display.region)
                        .await;
                    if !cap.is_empty() && (lyr.is_empty() || cap.synced) {
                        lyr = cap;
                    }
                }
                if lyr.is_empty() || !lyr.synced {
                    let lrc = self.fetch_lrclib(track).await;
                    if !lrc.is_empty() && (lyr.is_empty() || (lrc.synced && !lyr.synced)) {
                        lyr = lrc;
                    }
                }
                lyr
            }
            LyricsSource::Auto => {
                let mut lyr = self.fetch_lrclib(track).await;
                if lyr.is_empty() || !lyr.synced {
                    let yt = self.fetch_yt_music(track).await;
                    if !yt.is_empty() && (lyr.is_empty() || yt.synced) {
                        lyr = yt;
                    }
                }
                if lyr.is_empty() || !lyr.synced {
                    let cap = self
                        .fetch_yt_captions(track, &settings.display.region)
                        .await;
                    if !cap.is_empty() && (lyr.is_empty() || cap.synced) {
                        lyr = cap;
                    }
                }
                lyr
            }
        }
    }

    async fn fetch_yt_music(&self, track: &Track) -> Lyrics {
        let Some(cat) = &self.catalog else {
            return Lyrics::default();
        };
        match cat.yt_lyrics(&track.video_id).await {
            Ok(Some(lyr)) => lyr,
            Ok(None) => Lyrics::default(),
            Err(err) => {
                debug!("YTM lyrics fetch failed for {}: {err}", track.video_id);
                Lyrics::default()
            }
        }
    }

    async fn fetch_lrclib(&self, track: &Track) -> Lyrics {
        let title = clean_media_title(&track.title, false);
        if title.is_empty() {
            return Lyrics::default();
        }
        let artist = track
            .artists
            .first()
            .map(|a| a.name.as_str())
            .filter(|a| !a.is_empty() && *a != "Unknown artist");

        let mut params: Vec<(&str, String)> = vec![("track_name", title.clone())];
        if let Some(a) = artist {
            params.push(("artist_name", a.to_string()));
        }
        if let Some(al) = track.album.as_deref().filter(|s| !s.is_empty()) {
            params.push(("album_name", al.to_string()));
        }
        if let Some(dur) = track.duration.filter(|d| *d > 0) {
            params.push(("duration", dur.to_string()));
        }

        let mut data = self.get_json(&format!("{LRCLIB_BASE}/get"), &params).await;
        if data.is_none() {
            let search_params: Vec<(&str, String)> = if let Some(a) = artist {
                vec![
                    ("track_name", title.clone()),
                    ("artist_name", a.to_string()),
                ]
            } else {
                vec![("q", title.clone())]
            };
            if let Some(Value::Array(results)) = self
                .get_json(&format!("{LRCLIB_BASE}/search"), &search_params)
                .await
            {
                data = pick_best_lrclib_candidate(&results, &title, artist, track.duration);
            }
        }

        let Some(obj) = data.as_ref().and_then(Value::as_object) else {
            return Lyrics::default();
        };

        if let Some(synced) = obj
            .get("syncedLyrics")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            let parsed = parse_lrc(synced, "LRCLIB");
            if !parsed.is_empty() {
                return parsed;
            }
        }
        if let Some(plain) = obj
            .get("plainLyrics")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            let lines: Vec<LyricLine> = plain
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| LyricLine::new(0.0, s))
                .collect();
            if !lines.is_empty() {
                return Lyrics {
                    lines,
                    synced: false,
                    source: "LRCLIB".to_string(),
                    translation_lang: String::new(),
                };
            }
        }
        Lyrics::default()
    }

    async fn fetch_yt_captions(&self, track: &Track, region: &str) -> Lyrics {
        if !self.allow_ytdlp_captions || !is_valid_video_id(&track.video_id) {
            return Lyrics::default();
        }
        let target = format!("https://www.youtube.com/watch?v={}", track.video_id);
        let child = Command::new("yt-dlp")
            .args([
                "--no-cookies",
                "--no-playlist",
                "--no-warnings",
                "--quiet",
                "--skip-download",
                "--print",
                "%(subtitles)j",
                &target,
            ])
            .kill_on_drop(true)
            .output();

        let Ok(Ok(output)) = tokio::time::timeout(Duration::from_secs(12), child).await else {
            return Lyrics::default();
        };
        if !output.status.success() || output.stdout.is_empty() {
            return Lyrics::default();
        }
        let Ok(Value::Object(subs)) = serde_json::from_slice::<Value>(&output.stdout) else {
            return Lyrics::default();
        };

        let reg = region.to_ascii_lowercase();
        let mut pref: Vec<String> = Vec::new();
        if !reg.is_empty() && reg != "zz" {
            pref.push(reg);
        }
        for l in ["en", "en-US", "en-GB", "id", "orig"] {
            if !pref.iter().any(|p| p == l) {
                pref.push(l.to_string());
            }
        }

        let mut lang_order: Vec<String> = pref
            .iter()
            .filter(|k| subs.contains_key(*k))
            .cloned()
            .collect();
        for k in subs.keys() {
            if !lang_order.contains(k) && !k.to_ascii_lowercase().contains("live_chat") {
                lang_order.push(k.clone());
            }
        }

        for lang in lang_order {
            let Some(tracks) = subs.get(&lang).and_then(Value::as_array) else {
                continue;
            };
            let pick = ["json3", "vtt", "srt"].into_iter().find_map(|ext| {
                tracks.iter().find(|t| {
                    t.get("ext").and_then(Value::as_str) == Some(ext)
                        && t.get("url").and_then(Value::as_str).is_some()
                })
            });
            let Some(item) = pick else {
                continue;
            };
            let Some(url) = item.get("url").and_then(Value::as_str) else {
                continue;
            };
            let ext = item.get("ext").and_then(Value::as_str).unwrap_or("");
            let Ok(resp) = self
                .http
                .get(url)
                .header("User-Agent", USER_AGENT)
                .timeout(Duration::from_secs(8))
                .send()
                .await
            else {
                continue;
            };
            if !resp.status().is_success() {
                continue;
            }
            let parsed = if ext == "json3" {
                resp.json::<Value>()
                    .await
                    .map(|v| parse_json3_captions(&v, "YouTube Captions"))
                    .unwrap_or_default()
            } else {
                resp.text()
                    .await
                    .map(|t| parse_vtt_captions(&t, "YouTube Captions"))
                    .unwrap_or_default()
            };
            if !parsed.is_empty() {
                return parsed;
            }
        }
        Lyrics::default()
    }

    async fn get_json(&self, url: &str, params: &[(&str, String)]) -> Option<Value> {
        let resp = self
            .http
            .get(url)
            .header("User-Agent", USER_AGENT)
            .query(params)
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.json::<Value>().await.ok()
    }
}
