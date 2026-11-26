use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Duration;

use regex::Regex;
use serde_json::Value;
use tokio::process::Command;
use tracing::debug;

use crate::config::{LyricsSource, Settings};
use crate::domain::{LyricLine, Lyrics, Track, clean_media_title};
use crate::innertube::CatalogApi;
use crate::storage::CacheManager;

const LRCLIB_BASE: &str = "https://lrclib.net/api";
const TRANSLATE_ENDPOINT: &str = "https://translate.googleapis.com/translate_a/single";
const USER_AGENT: &str = "musick/0.2.0 (https://github.com/endrico-fn/musick)";

static TS_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\[(\d{1,3}):(\d{2})(?:[.:](\d{1,3}))?\]").ok());
static OFFSET_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)^\[offset:([+-]?\d+)\]$").ok());
static META_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^\[[a-zA-Z]+:.*\]$").ok());
static VTT_TS_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?:(\d{1,2}):)?(\d{2}):(\d{2})[.,](\d{3})\s*-->").ok());
static TAG_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"<[^>]+>").ok());
static PUNCT_ONLY_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"^[\s·•♪♫\-_=~.,!?…()\[\]"']+$"#).ok());
static WORD_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"[a-zA-Z]{2,}").ok());

const ID_STOPWORDS: &[&str] = &[
    "aku", "kamu", "kau", "ku", "mu", "nya", "yang", "dan", "di", "ke", "dari", "untuk", "dengan",
    "tidak", "tak", "bukan", "jangan", "kita", "kami", "mereka", "ini", "itu", "sudah", "telah",
    "akan", "bisa", "dapat", "ada", "jadi", "hanya", "saja", "juga", "lagi", "kalau", "jika",
    "bila", "saat", "ketika", "karena", "atau", "tapi", "tetapi", "selalu", "pernah", "cinta",
    "hati", "rindu", "malam", "hari", "hidup", "dalam", "semua", "sendiri", "bersama", "mungkin",
    "ingin",
];

pub fn should_skip_translation(text: &str) -> bool {
    let clean = text.trim();
    if clean.is_empty() {
        return true;
    }
    if PUNCT_ONLY_RE.as_ref().is_some_and(|re| re.is_match(clean)) {
        return true;
    }
    clean.starts_with('[') && clean.ends_with(']')
}

pub fn looks_indonesian(lines: &[LyricLine]) -> bool {
    let Some(re) = WORD_RE.as_ref() else {
        return false;
    };
    let mut total = 0usize;
    let mut id_hits = 0usize;
    for ln in lines {
        for m in re.find_iter(&ln.text) {
            total += 1;
            let lower = m.as_str().to_ascii_lowercase();
            if ID_STOPWORDS.contains(&lower.as_str()) {
                id_hits += 1;
            }
        }
    }
    if total < 6 {
        return false;
    }
    (id_hits as f64 / total as f64) >= 0.30
}

pub fn parse_lrc(raw: &str, source: &str) -> Lyrics {
    if raw.trim().is_empty() {
        return Lyrics::default();
    }

    let mut offset_sec = 0.0_f64;
    let mut entries = Vec::new();
    let mut plain_lines = Vec::new();

    for raw_line in raw.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(off_re) = OFFSET_RE.as_ref()
            && let Some(caps) = off_re.captures(line)
            && let Ok(ms) = caps[1].parse::<i64>()
        {
            offset_sec = ms as f64 / 1000.0;
            continue;
        }
        let Some(ts_re) = TS_RE.as_ref() else {
            continue;
        };
        let matches: Vec<_> = ts_re.captures_iter(line).collect();
        if matches.is_empty() {
            if !META_RE.as_ref().is_some_and(|re| re.is_match(line)) {
                plain_lines.push(line.to_string());
            }
            continue;
        }
        let text = ts_re.replace_all(line, "").trim().to_string();
        for caps in matches {
            let mins: f64 = caps[1].parse().unwrap_or(0.0);
            let secs: f64 = caps[2].parse().unwrap_or(0.0);
            let frac_ms: f64 = caps
                .get(3)
                .map(|m| {
                    let s = m.as_str();
                    let padded = format!("{s:0<3}");
                    padded[..3].parse::<f64>().unwrap_or(0.0)
                })
                .unwrap_or(0.0);
            let ts = (mins * 60.0 + secs + frac_ms / 1000.0 + offset_sec).max(0.0);
            entries.push(LyricLine::new(ts, text.clone()));
        }
    }

    if !entries.is_empty() {
        entries.sort_by(|a, b| a.time.total_cmp(&b.time));
        return Lyrics {
            lines: entries,
            synced: true,
            source: source.to_string(),
            translation_lang: String::new(),
        };
    }

    if !plain_lines.is_empty() {
        return Lyrics {
            lines: plain_lines
                .into_iter()
                .map(|t| LyricLine::new(0.0, t))
                .collect(),
            synced: false,
            source: source.to_string(),
            translation_lang: String::new(),
        };
    }

    Lyrics::default()
}

pub fn parse_vtt_captions(raw: &str, source: &str) -> Lyrics {
    if raw.trim().is_empty() {
        return Lyrics::default();
    }

    let mut entries: Vec<LyricLine> = Vec::new();
    let mut cur_time: Option<f64> = None;
    let mut buf: Vec<String> = Vec::new();

    let flush = |cur_time: &mut Option<f64>,
                 buf: &mut Vec<String>,
                 entries: &mut Vec<LyricLine>| {
        if let Some(t) = cur_time.take()
            && !buf.is_empty()
        {
            let joined = buf.join(" ");
            let stripped = match TAG_RE.as_ref() {
                Some(re) => re.replace_all(&joined, "").into_owned(),
                None => joined,
            };
            let text = html_escape::decode_html_entities(&stripped)
                .trim()
                .to_string();
            if !text.is_empty() && entries.last().map(|e| e.text.as_str()) != Some(text.as_str()) {
                entries.push(LyricLine::new(t, text));
            }
        }
        buf.clear();
    };

    for raw_ln in raw.lines() {
        let ln = raw_ln.trim();
        if ln.is_empty() {
            flush(&mut cur_time, &mut buf, &mut entries);
            continue;
        }
        if ln.starts_with("WEBVTT")
            || ln.starts_with("Kind:")
            || ln.starts_with("Language:")
            || ln.starts_with("NOTE")
        {
            continue;
        }
        if let Some(vtt_re) = VTT_TS_RE.as_ref()
            && let Some(caps) = vtt_re.captures(ln)
        {
            flush(&mut cur_time, &mut buf, &mut entries);
            let hrs: f64 = caps
                .get(1)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0.0);
            let mins: f64 = caps[2].parse().unwrap_or(0.0);
            let secs: f64 = caps[3].parse().unwrap_or(0.0);
            let ms: f64 = caps[4].parse().unwrap_or(0.0);
            cur_time = Some(hrs * 3600.0 + mins * 60.0 + secs + ms / 1000.0);
            continue;
        }
        if cur_time.is_none() && ln.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if cur_time.is_some() {
            buf.push(ln.to_string());
        }
    }
    flush(&mut cur_time, &mut buf, &mut entries);

    if entries.is_empty() {
        Lyrics::default()
    } else {
        Lyrics {
            lines: entries,
            synced: true,
            source: source.to_string(),
            translation_lang: String::new(),
        }
    }
}

pub fn parse_json3_captions(data: &Value, source: &str) -> Lyrics {
    let Some(events) = data.get("events").and_then(Value::as_array) else {
        return Lyrics::default();
    };
    let mut entries: Vec<LyricLine> = Vec::new();
    for ev in events {
        let Some(segs) = ev.get("segs").and_then(Value::as_array) else {
            continue;
        };
        let mut raw_text = String::new();
        for s in segs {
            if let Some(u) = s.get("utf8").and_then(Value::as_str) {
                raw_text.push_str(u);
            }
        }
        let decoded = html_escape::decode_html_entities(&raw_text);
        let text = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        let t_ms = ev
            .get("tStartMs")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .max(0.0);
        if entries.last().map(|e| e.text.as_str()) != Some(text.as_str()) {
            entries.push(LyricLine::new(t_ms / 1000.0, text));
        }
    }

    if entries.is_empty() {
        Lyrics::default()
    } else {
        Lyrics {
            lines: entries,
            synced: true,
            source: source.to_string(),
            translation_lang: String::new(),
        }
    }
}

pub fn active_line_index(lyrics: &Lyrics, position_secs: f64) -> Option<usize> {
    lyrics.active_line_index(position_secs)
}

fn parse_translate_payload(data: &Value) -> (String, String) {
    let Some(arr) = data.as_array() else {
        return (String::new(), String::new());
    };
    let src_lang = arr.get(2).and_then(Value::as_str).unwrap_or("").to_string();
    let Some(chunks) = arr.first().and_then(Value::as_array) else {
        return (String::new(), src_lang);
    };
    let mut out = String::new();
    for seg in chunks {
        if let Some(part) = seg
            .as_array()
            .and_then(|s| s.first())
            .and_then(Value::as_str)
        {
            out.push_str(part);
        }
    }
    (out, src_lang)
}

pub struct LyricsService {
    catalog: Option<Arc<dyn CatalogApi>>,
    cache: Arc<CacheManager>,
    http: reqwest::Client,
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

    fn current_settings(&self) -> Settings {
        self.settings.read().map(|g| g.clone()).unwrap_or_default()
    }

    fn cache_key(video_id: &str, source: LyricsSource) -> String {
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
                vec![("track_name", title), ("artist_name", a.to_string())]
            } else {
                vec![("q", title)]
            };
            if let Some(Value::Array(results)) = self
                .get_json(&format!("{LRCLIB_BASE}/search"), &search_params)
                .await
            {
                data = results
                    .iter()
                    .find(|r| {
                        r.get("syncedLyrics")
                            .and_then(Value::as_str)
                            .is_some_and(|s| !s.trim().is_empty())
                    })
                    .or_else(|| results.first())
                    .cloned();
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
        if !self.allow_ytdlp_captions {
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

    pub async fn translate_lyrics(
        &self,
        mut lyrics: Lyrics,
        video_id: Option<&str>,
        target_lang: &str,
    ) -> Lyrics {
        if lyrics.is_empty() {
            return lyrics;
        }
        if lyrics.translation_lang == target_lang
            && (lyrics.has_translations() || looks_indonesian(&lyrics.lines))
        {
            return lyrics;
        }

        if target_lang == "id" && looks_indonesian(&lyrics.lines) {
            lyrics.translation_lang = target_lang.to_string();
        } else {
            let texts: Vec<String> = lyrics.lines.iter().map(|l| l.text.clone()).collect();
            let (tr_map, ok) = self.translate_texts(&texts, target_lang).await;
            if !ok {
                return lyrics;
            }
            for line in &mut lyrics.lines {
                if let Some(tr) = tr_map.get(&line.text) {
                    line.translation = tr.clone();
                }
            }
            lyrics.translation_lang = target_lang.to_string();
        }

        if let Some(vid) = video_id {
            let source = self.current_settings().lyrics.source;
            self.cache
                .put_lyrics(&Self::cache_key(vid, source), &lyrics);
        }
        lyrics
    }

    async fn translate_texts(
        &self,
        texts: &[String],
        target_lang: &str,
    ) -> (HashMap<String, String>, bool) {
        let mut unique = Vec::new();
        let mut seen = HashSet::new();
        for t in texts {
            if !should_skip_translation(t) && seen.insert(t.as_str()) {
                unique.push(t.clone());
            }
        }
        if unique.is_empty() {
            return (HashMap::new(), true);
        }

        let mut batches: Vec<Vec<String>> = Vec::new();
        let mut cur: Vec<String> = Vec::new();
        let mut cur_chars = 0usize;
        for item in unique {
            if !cur.is_empty() && (cur.len() >= 25 || cur_chars + item.len() + 1 > 1200) {
                batches.push(cur);
                cur = vec![item.clone()];
                cur_chars = item.len();
            } else {
                cur_chars += item.len() + 1;
                cur.push(item);
            }
        }
        if !cur.is_empty() {
            batches.push(cur);
        }

        let mut mapping = HashMap::new();
        for batch in batches {
            let Ok((parts, src_lang)) = self.translate_chunk(&batch, target_lang).await else {
                return (HashMap::new(), false);
            };
            if src_lang
                .split('-')
                .next()
                .is_some_and(|s| s.eq_ignore_ascii_case(target_lang))
            {
                return (HashMap::new(), true);
            }

            if parts.len() == batch.len() {
                for (orig, tr) in batch.into_iter().zip(parts) {
                    let clean = tr.trim();
                    if !clean.is_empty() && !clean.eq_ignore_ascii_case(orig.trim()) {
                        mapping.insert(orig, clean.to_string());
                    }
                }
            } else {
                // Per-line fallback when batch newline segmentation shifts
                for orig in batch {
                    let Ok((single, _)) = self
                        .translate_chunk(std::slice::from_ref(&orig), target_lang)
                        .await
                    else {
                        return (HashMap::new(), false);
                    };
                    let joined = single.join(" ");
                    let clean = joined.trim();
                    if !clean.is_empty() && !clean.eq_ignore_ascii_case(orig.trim()) {
                        mapping.insert(orig, clean.to_string());
                    }
                }
            }
        }

        (mapping, true)
    }

    async fn translate_chunk(
        &self,
        lines: &[String],
        target_lang: &str,
    ) -> Result<(Vec<String>, String), ()> {
        let q = lines.join("\n");
        let resp = self
            .http
            .get(TRANSLATE_ENDPOINT)
            .header("User-Agent", USER_AGENT)
            .query(&[
                ("client", "gtx"),
                ("sl", "auto"),
                ("tl", target_lang),
                ("dt", "t"),
                ("q", q.as_str()),
            ])
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .map_err(|_| ())?;
        if !resp.status().is_success() {
            return Err(());
        }
        let val: Value = resp.json().await.map_err(|_| ())?;
        let (joined, src_lang) = parse_translate_payload(&val);
        let parts: Vec<String> = if joined.is_empty() {
            Vec::new()
        } else {
            joined.lines().map(str::to_string).collect()
        };
        Ok((parts, src_lang))
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
