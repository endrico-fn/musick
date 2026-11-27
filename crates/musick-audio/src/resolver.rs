use std::num::NonZeroUsize;
use std::process::Stdio;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use lru::LruCache;
use tokio::process::Command;
use tracing::debug;

use musick_core::{AppPaths, AudioQuality, SingleFlight, is_valid_video_id};

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
    pub user_agent: Option<String>,
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
    pub user_agent: Option<String>,
    pub from_fast_path: bool,
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

/// extract `expire=` unix timestamp from googlevideo urls minus 120s safety margin (30s floor)
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
        if !is_valid_video_id(video_id) {
            return Err(AudioError::Resolver(format!(
                "Invalid video ID: {video_id}"
            )));
        }
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
                "%(acodec)s\t%(abr)s\t%(asr)s\t%(http_headers.User-Agent)s",
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
        let mut user_agent = None;

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
            if let Some(&ua) = parts.get(3)
                && !matches!(ua, "NA" | "none" | "None" | "")
            {
                user_agent = Some(ua.to_string());
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
            user_agent,
        })
    }
}

// ── HybridStreamResolver with Circuit Breaker, Single-Flight & Quality LRU ──

#[derive(Default)]
struct CircuitState {
    consecutive_failures: u32,
    cooldown_until: f64,
}

pub struct HybridStreamResolver {
    fast: Option<Arc<dyn StreamBackend>>,
    fallback: Arc<dyn StreamBackend>,
    quality: RwLock<AudioQuality>,
    cache: Mutex<LruCache<String, ResolvedStream>>,
    inflight: SingleFlight<String, Result<ResolvedStream, String>>,
    circuit: Mutex<CircuitState>,
    clock: Arc<dyn Fn() -> f64 + Send + Sync>,
}

impl HybridStreamResolver {
    pub fn new(_paths: &AppPaths, quality: AudioQuality) -> Self {
        Self::with_backends(
            None,
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
        let cap = NonZeroUsize::new(MAX_RESOLVED_STREAMS).unwrap_or(NonZeroUsize::MIN);
        Self {
            fast,
            fallback,
            quality: RwLock::new(quality),
            cache: Mutex::new(LruCache::new(cap)),
            inflight: SingleFlight::new(),
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
        let hit = cache.get(video_id).cloned()?;
        if hit.quality != quality || now >= hit.expires_at {
            cache.pop(video_id);
            return None;
        }
        Some(hit)
    }

    pub fn invalidate(&self, video_id: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.pop(video_id);
        }
    }

    /// evict broken stream url and trip fast-path circuit breaker if applicable
    pub fn report_playback_failure(&self, video_id: &str, was_fast_path: bool) {
        self.invalidate(video_id);
        if was_fast_path && let Ok(mut cb) = self.circuit.lock() {
            cb.consecutive_failures = CIRCUIT_BREAKER_THRESHOLD;
            cb.cooldown_until = (self.clock)() + CIRCUIT_BREAKER_COOLDOWN_SECS;
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
        self.resolve_inner(video_id, false).await
    }

    /// resolve strictly via fallback backend, bypassing fast-path and cache
    pub async fn resolve_fallback_only(&self, video_id: &str) -> AudioResult<ResolvedStream> {
        self.invalidate(video_id);
        self.resolve_inner(video_id, true).await
    }

    async fn resolve_inner(
        &self,
        video_id: &str,
        force_fallback: bool,
    ) -> AudioResult<ResolvedStream> {
        if !force_fallback && let Some(cached) = self.get_cached(video_id) {
            return Ok(cached);
        }

        let inflight_key = if force_fallback {
            format!("fb:{video_id}")
        } else {
            video_id.to_string()
        };

        self.inflight
            .run(inflight_key, || async {
                if !force_fallback && let Some(cached) = self.get_cached(video_id) {
                    return Ok(cached);
                }
                self.resolve_uncached(video_id, force_fallback)
                    .await
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(AudioError::Resolver)
    }

    pub fn prefetch(self: &Arc<Self>, video_id: String) {
        if self.get_cached(&video_id).is_some() || self.inflight.is_inflight(&video_id) {
            return;
        }
        let resolver = self.clone();
        tokio::spawn(async move {
            if let Err(err) = resolver.resolve(&video_id).await {
                debug!("Prefetch failed for {video_id}: {err}");
            }
        });
    }

    async fn resolve_uncached(
        &self,
        video_id: &str,
        force_fallback: bool,
    ) -> AudioResult<ResolvedStream> {
        let quality = self.audio_quality();
        let now = (self.clock)();

        let (raw, from_fast_path) = if !force_fallback
            && let Some(ref fast_backend) = self.fast
            && !self.is_fast_circuit_open()
        {
            match fast_backend.resolve(video_id, quality).await {
                Ok(stream) => {
                    if let Ok(mut cb) = self.circuit.lock() {
                        cb.consecutive_failures = 0;
                        cb.cooldown_until = 0.0;
                    }
                    (stream, true)
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
                    (self.fallback.resolve(video_id, quality).await?, false)
                }
            }
        } else {
            (self.fallback.resolve(video_id, quality).await?, false)
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
            user_agent: raw.user_agent,
            from_fast_path,
        };

        if let Ok(mut cache) = self.cache.lock() {
            cache.put(video_id.to_string(), resolved.clone());
        }
        Ok(resolved)
    }
}
