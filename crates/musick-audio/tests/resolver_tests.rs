#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use async_trait::async_trait;

use musick_audio::{
    AudioError, AudioResult, HybridStreamResolver, RawStream, StreamBackend, parse_stream_expiry,
};
use musick_core::AudioQuality;

struct MockBackend {
    name: &'static str,
    fail: AtomicBool,
    calls: AtomicUsize,
    delay_ms: u64,
}

impl MockBackend {
    fn new(name: &'static str, fail: bool, delay_ms: u64) -> Self {
        Self {
            name,
            fail: AtomicBool::new(fail),
            calls: AtomicUsize::new(0),
            delay_ms,
        }
    }
}

#[async_trait]
impl StreamBackend for MockBackend {
    async fn resolve(&self, video_id: &str, quality: AudioQuality) -> AudioResult<RawStream> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(AudioError::Resolver(format!(
                "{} failed (PO token)",
                self.name
            )));
        }
        Ok(RawStream {
            url: format!(
                "https://rr1.googlevideo.com/videoplayback?id={video_id}&src={}&q={}&expire=20000",
                self.name,
                quality.as_str()
            ),
            codec: "opus".into(),
            bitrate: 131,
            sample_rate: 48_000,
            user_agent: None,
        })
    }
}

#[test]
fn parse_stream_expiry_applies_120s_margin_and_30s_floor() {
    let now = 10_000.0;
    // Normal future expiry: minus 120s safety margin
    let exp = parse_stream_expiry("https://rr1.googlevideo.com/v?expire=15000", now, 3600.0);
    assert!((exp - 14_880.0).abs() < 1e-6);

    // Near-term expiry: clamped to 30s floor
    let exp_near = parse_stream_expiry("https://rr1.googlevideo.com/v?expire=10050", now, 3600.0);
    assert!((exp_near - 10_030.0).abs() < 1e-6);

    // Missing or past expire -> default TTL
    let exp_missing = parse_stream_expiry("https://example.invalid/stream", now, 3600.0);
    assert!((exp_missing - 13_600.0).abs() < 1e-6);
}

#[tokio::test]
async fn circuit_breaker_fallback_single_flight_and_quality_cache_invalidation() {
    let clock_val = Arc::new(AtomicU64::new(10_000));
    let clock_ref = clock_val.clone();

    let fast = Arc::new(MockBackend::new("rustypipe", true, 0));
    let fallback = Arc::new(MockBackend::new("ytdlp", false, 20));

    let resolver = Arc::new(HybridStreamResolver::with_backends(
        Some(fast.clone()),
        fallback.clone(),
        AudioQuality::Best,
        Arc::new(move || clock_ref.load(Ordering::SeqCst) as f64),
    ));

    // 1. Single-flight dedup: 6 concurrent resolves for same video_id invoke backends only once
    let mut handles = Vec::new();
    for _ in 0..6 {
        let r = resolver.clone();
        handles.push(tokio::spawn(
            async move { r.resolve("vid1").await.unwrap() },
        ));
    }
    for h in handles {
        let s = h.await.unwrap();
        assert!(s.url.contains("src=ytdlp"));
        assert_eq!(s.quality_label(), "opus 131k");
    }
    assert_eq!(fast.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fallback.calls.load(Ordering::SeqCst), 1);
    assert!(!resolver.is_fast_circuit_open()); // only 1 failure so far

    // 2. Second consecutive fast failure trips 10-minute (600s) circuit breaker
    let s2 = resolver.resolve("vid2").await.unwrap();
    assert!(s2.url.contains("src=ytdlp"));
    assert_eq!(fast.calls.load(Ordering::SeqCst), 2);
    assert!(resolver.is_fast_circuit_open());

    // 3. While circuit breaker is open, fast backend is skipped completely
    let s3 = resolver.resolve("vid3").await.unwrap();
    assert!(s3.url.contains("src=ytdlp"));
    assert_eq!(fast.calls.load(Ordering::SeqCst), 2); // still 2!
    assert_eq!(fallback.calls.load(Ordering::SeqCst), 3);

    // 4. After 10 minutes (601s), fast backend recovers and is retried
    fast.fail.store(false, Ordering::SeqCst);
    clock_val.store(10_601, Ordering::SeqCst);
    assert!(!resolver.is_fast_circuit_open());
    let s4 = resolver.resolve("vid4").await.unwrap();
    assert!(s4.url.contains("src=rustypipe"));
    assert_eq!(fast.calls.load(Ordering::SeqCst), 3);

    // 5. Changing AudioQuality invalidates cached entries
    resolver.set_quality(AudioQuality::Saver);
    assert!(resolver.get_cached("vid4").is_none());
    let s4_saver = resolver.resolve("vid4").await.unwrap();
    assert_eq!(s4_saver.quality_label(), "opus 131k saver");
}

#[tokio::test]
async fn single_flight_recovers_cleanly_when_leader_is_cancelled_without_deadlock() {
    let slow = Arc::new(MockBackend::new("ytdlp", false, 80));
    let resolver = Arc::new(HybridStreamResolver::with_backends(
        None,
        slow.clone(),
        AudioQuality::Best,
        Arc::new(|| 10_000.0),
    ));

    // Spawn leader, then spawn a waiter, then abort the leader mid-flight!
    let r1 = resolver.clone();
    let leader = tokio::spawn(async move { r1.resolve("vid_cancel").await });
    tokio::time::sleep(std::time::Duration::from_millis(15)).await;

    let r2 = resolver.clone();
    let waiter = tokio::spawn(async move { r2.resolve("vid_cancel").await });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    leader.abort();
    let _ = leader.await;

    // Waiter MUST recover by becoming the new leader and completing within timeout (never deadlocks!).
    let res = tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
        .await
        .expect("Waiter deadlocked after leader cancellation!")
        .unwrap()
        .unwrap();
    assert_eq!(res.video_id, "vid_cancel");
    assert!(res.url.contains("id=vid_cancel"));
}

#[tokio::test]
async fn report_playback_failure_opens_circuit_breaker_and_resolve_fallback_only_bypasses_fast() {
    let clock_val = Arc::new(AtomicU64::new(10_000));
    let clock_ref = clock_val.clone();

    // Fast backend returns Ok(url), simulating a RustyPipe URL that later gets HTTP 403 in mpv.
    let fast = Arc::new(MockBackend::new("rustypipe", false, 0));
    let fallback = Arc::new(MockBackend::new("ytdlp", false, 0));

    let resolver = HybridStreamResolver::with_backends(
        Some(fast.clone()),
        fallback.clone(),
        AudioQuality::Best,
        Arc::new(move || clock_ref.load(Ordering::SeqCst) as f64),
    );

    let first = resolver.resolve("vid_403").await.unwrap();
    assert!(first.from_fast_path);
    assert!(first.url.contains("src=rustypipe"));
    assert!(!resolver.is_fast_circuit_open());

    // Simulate mpv reporting "loading failed" on the fast-path URL.
    resolver.report_playback_failure("vid_403", first.from_fast_path);
    assert!(resolver.get_cached("vid_403").is_none());
    assert!(resolver.is_fast_circuit_open());

    // Explicit fallback-only retry and subsequent resolves must route strictly to yt-dlp.
    let retried = resolver.resolve_fallback_only("vid_403").await.unwrap();
    assert!(!retried.from_fast_path);
    assert!(retried.url.contains("src=ytdlp"));

    let next_track = resolver.resolve("vid_next").await.unwrap();
    assert!(!next_track.from_fast_path);
    assert!(next_track.url.contains("src=ytdlp"));
    assert_eq!(fast.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fallback.calls.load(Ordering::SeqCst), 2);
}
