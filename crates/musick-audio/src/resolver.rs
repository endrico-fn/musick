use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::process::Command;
use tokio::sync::watch;
use tracing::debug;

#[cfg(feature = "fast-resolver")]
use musick_core::ensure_private_dir;
use musick_core::{AppPaths, AudioQuality};

use crate::error::{AudioError, AudioResult};

const MAX_RESOLVED_STREAMS: usize = 200;
const CIRCUIT_BREAKER_THRESHOLD: u32 = 2;
const CIRCUIT_BREAKER_COOLDOWN_SECS: f64 = 600.0;
const DEFAULT_STREAM_TTL_SECS: f64 = 3600.0;

pub fn now_unix_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawStream {
    pub url: String,
    pub codec: String,
    pub bitrate: u32,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStream {
    pub video_id: String,
    pub url: String,
    pub expires_at: f64,
    pub codec: String,
    pub bitrate: u32,
    pub sample_rate: u32,
    pub quality: AudioQuality,
}

impl ResolvedStream {
    pub fn quality_label(&self) -> String {
        let mut parts = Vec::new();
        let lower_codec = self.codec.to_ascii_lowercase();
        if !lower_codec.is_empty() && !matches!(lower_codec.as_str(), "none" | "na") {
            let label = if lower_codec.starts_with("mp4a") {
                "aac".to_string()
            } else {
                self.codec
                    .split('.')
                    .next()
                    .unwrap_or(&self.codec)
                    .to_string()
            };
            parts.push(label);
        }
        if self.bitrate > 0 {
            parts.push(format!("{}k", self.bitrate));
        }
        if self.quality == AudioQuality::Saver {
            parts.push("saver".to_string());
        }
        parts.join(" ")
    }
}

/// Extract `expire=` unix timestamp from googlevideo URLs minus 120s safety margin (30s floor).
pub fn parse_stream_expiry(url: &str, now_secs: f64, default_ttl_secs: f64) -> f64 {
    if let Ok(parsed) = url::Url::parse(url) {
        for (k, v) in parsed.query_pairs() {
            if k == "expire"
                && let Ok(exp_u64) = v.parse::<u64>()
            {
                let exp = exp_u64 as f64;
                if exp > now_secs {
                    return (exp - 120.0).max(now_secs + 30.0);
                }
            }
        }
    }
    now_secs + default_ttl_secs
}

#[async_trait]
pub trait StreamBackend: Send + Sync {
    async fn resolve(&self, video_id: &str, quality: AudioQuality) -> AudioResult<RawStream>;
}

// ── Optional Fast-Path: RustyPipe (GPL-3.0, feature-gated `fast-resolver`) ──

#[cfg(feature = "fast-resolver")]
pub struct RustyPipeBackend {
    client: rustypipe::client::RustyPipe,
}

#[cfg(feature = "fast-resolver")]
impl RustyPipeBackend {
    pub fn new(paths: &AppPaths) -> AudioResult<Self> {
        let rp_dir = paths.rustypipe_cache_dir();
        let _ = ensure_private_dir(&rp_dir);
        let client = rustypipe::client::RustyPipe::builder()
            .storage_dir(rp_dir)
            .no_reporter()
            .timeout(Duration::from_secs(12))
            .build()
            .map_err(|e| AudioError::Resolver(format!("RustyPipe init failed: {e}")))?;
        Ok(Self { client })
    }
}

#[cfg(feature = "fast-resolver")]
#[async_trait]
impl StreamBackend for RustyPipeBackend {
    async fn resolve(&self, video_id: &str, quality: AudioQuality) -> AudioResult<RawStream> {
        let player = self
            .client
            .query()
            .player(video_id)
            .await
            .map_err(|e| AudioError::Resolver(format!("RustyPipe player failed: {e}")))?;

        let mut streams = player.audio_streams;
        streams.retain(|s| !s.url.is_empty());
        if streams.is_empty() {
            return Err(AudioError::Resolver(
                "RustyPipe returned zero audio streams".into(),
            ));
        }

        let is_opus = |s: &rustypipe::model::AudioStream| {
            s.mime.to_ascii_lowercase().contains("opus")
                || format!("{:?}", s.codec)
                    .to_ascii_lowercase()
                    .contains("opus")
        };

        let picked = match quality {
            AudioQuality::Best => streams
                .iter()
                .max_by_key(|s| (u32::from(is_opus(s)), s.average_bitrate.max(s.bitrate))),
            AudioQuality::Balanced => streams
                .iter()
                .filter(|s| s.average_bitrate.max(s.bitrate) <= 150_000)
                .max_by_key(|s| (u32::from(is_opus(s)), s.average_bitrate.max(s.bitrate)))
                .or_else(|| {
                    streams
                        .iter()
                        .min_by_key(|s| s.average_bitrate.max(s.bitrate))
                }),
            AudioQuality::Saver => streams
                .iter()
                .filter(|s| {
                    let b = s.average_bitrate.max(s.bitrate);
                    (40_000..=85_000).contains(&b)
                })
                .min_by_key(|s| s.average_bitrate.max(s.bitrate))
                .or_else(|| {
                    streams
                        .iter()
                        .min_by_key(|s| s.average_bitrate.max(s.bitrate))
                }),
        }
        .ok_or_else(|| AudioError::Resolver("No suitable RustyPipe audio stream".into()))?;

        let codec = if is_opus(picked) {
            "opus".to_string()
        } else if picked.mime.to_ascii_lowercase().contains("mp4") {
            "aac".to_string()
        } else {
            format!("{:?}", picked.codec).to_ascii_lowercase()
        };
        let kbps = picked.average_bitrate.max(picked.bitrate) / 1000;

        Ok(RawStream {
            url: picked.url.clone(),
            codec,
            bitrate: kbps,
            sample_rate: 48_000,
        })
    }
}

// ── Fallback Backend: yt-dlp (strictly --no-cookies) ────────────────────────

pub struct YtDlpBackend {
    binary: String,
}

impl Default for YtDlpBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl YtDlpBackend {
    pub fn new() -> Self {
        Self {
            binary: "yt-dlp".to_string(),
        }
    }

    pub fn with_binary(binary: impl Into<String>) -> Self {
        Self {
            binary: binary.into(),
        }
    }
}

fn quality_profile(quality: AudioQuality) -> (&'static str, &'static str) {
    match quality {
        AudioQuality::Best => (
            "bestaudio[format_id!*=drc][acodec^=opus]/bestaudio[format_id!*=drc]/bestaudio/best",
            "lang,quality,acodec:opus,abr,asr",
        ),
        AudioQuality::Balanced => (
            "bestaudio[format_id!*=drc][abr<=145]/bestaudio[format_id!*=drc]/bestaudio/best",
            "lang,+abr:128,acodec:opus",
        ),
        AudioQuality::Saver => (
            "worstaudio[abr>=48]/bestaudio[abr<=80]/bestaudio/best",
            "lang,+abr:64",
        ),
    }
}

#[async_trait]
impl StreamBackend for YtDlpBackend {
    async fn resolve(&self, video_id: &str, quality: AudioQuality) -> AudioResult<RawStream> {
        let target = format!("https://music.youtube.com/watch?v={video_id}");
        let (fmt, sort_spec) = quality_profile(quality);

        let fut = Command::new(&self.binary)
            .args([
                "--no-cookies",
                "--no-playlist",
                "--no-warnings",
                "--quiet",
                "-f",
                fmt,
                "--format-sort",
                sort_spec,
                "--print",
                "%(acodec)s\t%(abr)s\t%(asr)s",
                "-g",
                &target,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output();

        let output = tokio::time::timeout(Duration::from_secs(20), fut)
            .await
            .map_err(|_| AudioError::Resolver("yt-dlp timed out after 20s".into()))?
            .map_err(|e| AudioError::Resolver(format!("Failed to execute yt-dlp: {e}")))?;

        if !output.status.success() {
            let err_txt = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let msg = if err_txt.is_empty() {
                format!("yt-dlp exited with {}", output.status)
            } else {
                err_txt
            };
            return Err(AudioError::Resolver(msg));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if lines.is_empty() {
            return Err(AudioError::Resolver(
                "yt-dlp returned empty stream URL".into(),
            ));
        }

        let mut codec = String::new();
        let mut bitrate = 0u32;
        let mut sample_rate = 0u32;

        let url = if lines[0].contains('\t') && lines.len() >= 2 {
            let parts: Vec<&str> = lines[0].split('\t').collect();
            if let Some(&c) = parts.first()
                && !matches!(c, "NA" | "none" | "None" | "")
            {
                codec = c.to_string();
            }
            if let Some(&abr) = parts.get(1)
                && let Ok(val) = abr.parse::<f64>()
            {
                bitrate = val.round().max(0.0) as u32;
            }
            if let Some(&asr) = parts.get(2)
                && let Ok(val) = asr.parse::<f64>()
            {
                sample_rate = val.round().max(0.0) as u32;
            }
            lines[1].to_string()
        } else {
            lines[0].to_string()
        };

        Ok(RawStream {
            url,
            codec,
            bitrate,
            sample_rate,
        })
    }
}

// ── HybridStreamResolver with Circuit Breaker, Single-Flight & Quality LRU ──

#[derive(Default)]
struct CircuitState {
    consecutive_failures: u32,
    cooldown_until: f64,
}

#[derive(Default)]
struct StreamLru {
    map: HashMap<String, ResolvedStream>,
    order: VecDeque<String>,
}

impl StreamLru {
    fn get(&mut self, key: &str) -> Option<ResolvedStream> {
        let hit = self.map.get(key).cloned()?;
        self.touch(key);
        Some(hit)
    }

    fn insert(&mut self, key: String, val: ResolvedStream) {
        self.map.insert(key.clone(), val);
        self.touch(&key);
        while self.map.len() > MAX_RESOLVED_STREAMS {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            } else {
                break;
            }
        }
    }

    fn remove(&mut self, key: &str) {
        self.map.remove(key);
        self.order.retain(|k| k != key);
    }

    fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    fn touch(&mut self, key: &str) {
        self.order.retain(|k| k != key);
        self.order.push_back(key.to_string());
    }
}

type ResolverOutcome = Option<Result<ResolvedStream, String>>;
type ResolverInflightMap = Mutex<HashMap<String, watch::Receiver<ResolverOutcome>>>;

struct ResolverInflightGuard<'a> {
    map: &'a ResolverInflightMap,
    key: String,
}

impl Drop for ResolverInflightGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.map.lock() {
            guard.remove(&self.key);
        }
    }
}

pub struct HybridStreamResolver {
    fast: Option<Arc<dyn StreamBackend>>,
    fallback: Arc<dyn StreamBackend>,
    quality: RwLock<AudioQuality>,
    cache: Mutex<StreamLru>,
    inflight: ResolverInflightMap,
    circuit: Mutex<CircuitState>,
    clock: Arc<dyn Fn() -> f64 + Send + Sync>,
}

impl HybridStreamResolver {
    pub fn new(paths: &AppPaths, quality: AudioQuality) -> Self {
        #[cfg(feature = "fast-resolver")]
        let fast: Option<Arc<dyn StreamBackend>> = RustyPipeBackend::new(paths)
            .ok()
            .map(|b| Arc::new(b) as Arc<dyn StreamBackend>);
        #[cfg(not(feature = "fast-resolver"))]
        let fast: Option<Arc<dyn StreamBackend>> = {
            let _ = paths;
            None
        };

        Self::with_backends(
            fast,
            Arc::new(YtDlpBackend::new()),
            quality,
            Arc::new(now_unix_f64),
        )
    }

    pub fn with_backends(
        fast: Option<Arc<dyn StreamBackend>>,
        fallback: Arc<dyn StreamBackend>,
        quality: AudioQuality,
        clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    ) -> Self {
        Self {
            fast,
            fallback,
            quality: RwLock::new(quality),
            cache: Mutex::new(StreamLru::default()),
            inflight: Mutex::new(HashMap::new()),
            circuit: Mutex::new(CircuitState::default()),
            clock,
        }
    }

    pub fn audio_quality(&self) -> AudioQuality {
        self.quality.read().map(|g| *g).unwrap_or_default()
    }

    pub fn set_quality(&self, quality: AudioQuality) {
        let changed = self
            .quality
            .write()
            .map(|mut g| {
                let diff = *g != quality;
                *g = quality;
                diff
            })
            .unwrap_or(false);
        if changed && let Ok(mut c) = self.cache.lock() {
            c.clear();
        }
    }

    pub fn get_cached(&self, video_id: &str) -> Option<ResolvedStream> {
        let now = (self.clock)();
        let quality = self.audio_quality();
        let mut cache = self.cache.lock().ok()?;
        let hit = cache.get(video_id)?;
        if hit.quality != quality || now >= hit.expires_at {
            cache.remove(video_id);
            return None;
        }
        Some(hit)
    }

    pub fn invalidate(&self, video_id: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(video_id);
        }
    }

    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
        }
    }

    pub fn is_fast_circuit_open(&self) -> bool {
        let now = (self.clock)();
        self.circuit
            .lock()
            .map(|c| now < c.cooldown_until)
            .unwrap_or(false)
    }

    pub async fn resolve(&self, video_id: &str) -> AudioResult<ResolvedStream> {
        if let Some(cached) = self.get_cached(video_id) {
            return Ok(cached);
        }

        // Lossless single-flight deduplication via `watch` channel + RAII drop guard:
        // zero lost wakeups and guaranteed cleanup if the leader future is cancelled.
        let leader_tx = loop {
            enum Slot {
                Leader(watch::Sender<ResolverOutcome>),
                Waiter(watch::Receiver<ResolverOutcome>),
            }

            let slot = {
                let mut map = self
                    .inflight
                    .lock()
                    .map_err(|_| AudioError::Resolver("Resolver inflight lock poisoned".into()))?;
                if let Some(existing) = map.get(video_id) {
                    Slot::Waiter(existing.clone())
                } else {
                    let (tx, rx) = watch::channel(None);
                    map.insert(video_id.to_string(), rx);
                    Slot::Leader(tx)
                }
            };

            match slot {
                Slot::Leader(tx) => break tx,
                Slot::Waiter(mut rx) => {
                    if let Some(done) = rx.borrow().clone() {
                        return done.map_err(AudioError::Resolver);
                    }
                    if rx.changed().await.is_ok()
                        && let Some(done) = rx.borrow().clone()
                    {
                        return done.map_err(AudioError::Resolver);
                    }
                    if let Some(cached) = self.get_cached(video_id) {
                        return Ok(cached);
                    }
                    // Leader was cancelled before publishing; loop to become leader.
                }
            }
        };

        let _guard = ResolverInflightGuard {
            map: &self.inflight,
            key: video_id.to_string(),
        };

        let result = self.resolve_uncached(video_id).await;
        let outcome = match &result {
            Ok(stream) => Ok(stream.clone()),
            Err(err) => Err(err.to_string()),
        };
        let _ = leader_tx.send(Some(outcome));
        result
    }

    pub fn prefetch(self: &Arc<Self>, video_id: String) {
        if self.get_cached(&video_id).is_some() {
            return;
        }
        if let Ok(map) = self.inflight.lock()
            && map.contains_key(&video_id)
        {
            return;
        }
        let resolver = self.clone();
        tokio::spawn(async move {
            if let Err(err) = resolver.resolve(&video_id).await {
                debug!("Prefetch failed for {video_id}: {err}");
            }
        });
    }

    async fn resolve_uncached(&self, video_id: &str) -> AudioResult<ResolvedStream> {
        let quality = self.audio_quality();
        let now = (self.clock)();

        let raw = if let Some(ref fast_backend) = self.fast
            && !self.is_fast_circuit_open()
        {
            match fast_backend.resolve(video_id, quality).await {
                Ok(stream) => {
                    if let Ok(mut cb) = self.circuit.lock() {
                        cb.consecutive_failures = 0;
                        cb.cooldown_until = 0.0;
                    }
                    stream
                }
                Err(fast_err) => {
                    debug!(
                        "Fast resolver failed for {video_id}, falling back to yt-dlp: {fast_err}"
                    );
                    if let Ok(mut cb) = self.circuit.lock() {
                        cb.consecutive_failures += 1;
                        if cb.consecutive_failures >= CIRCUIT_BREAKER_THRESHOLD {
                            cb.cooldown_until = now + CIRCUIT_BREAKER_COOLDOWN_SECS;
                        }
                    }
                    self.fallback.resolve(video_id, quality).await?
                }
            }
        } else {
            self.fallback.resolve(video_id, quality).await?
        };

        let expires_at = parse_stream_expiry(&raw.url, (self.clock)(), DEFAULT_STREAM_TTL_SECS);
        let resolved = ResolvedStream {
            video_id: video_id.to_string(),
            url: raw.url,
            expires_at,
            codec: raw.codec,
            bitrate: raw.bitrate,
            sample_rate: raw.sample_rate,
            quality,
        };

        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(video_id.to_string(), resolved.clone());
        }
        Ok(resolved)
    }
}
