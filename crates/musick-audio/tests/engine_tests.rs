#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tempfile::tempdir;
use tokio::sync::broadcast;

use musick_audio::{
    AudioBackend, AudioProperties, AudioResult, BackendEvent, HybridStreamResolver, PlayerEngine,
    PlayerEngineDeps, RawStream, StreamBackend, build_cava_config, parse_cava_frame, resample_bars,
};
use musick_core::{
    Album, AlbumSummary, AppCommand, AppPaths, Artist, ArtistSummary, AudioQuality, AuthManager,
    CacheManager, CacheSettings, CatalogApi, CavaAudioSource, CavaSmoothing, CoreError, CoreResult,
    HomeFeed, Lyrics, LyricsService, MemorySecretStore, PlaybackTelemetryEvent, PlaybackTracking,
    Playlist, PlaylistEditAction, PlaylistEditResult, PlaylistSummary, RadioBatch, SearchCategory,
    SearchItem, Settings, SongRating, Track, UserStore,
};

struct MockAudioBackend {
    events_tx: broadcast::Sender<BackendEvent>,
    loaded_urls: Mutex<Vec<String>>,
    gapless_urls: Mutex<Vec<String>>,
    props: Mutex<AudioProperties>,
}

impl MockAudioBackend {
    fn new() -> Self {
        let (events_tx, _) = broadcast::channel(64);
        Self {
            events_tx,
            loaded_urls: Mutex::new(Vec::new()),
            gapless_urls: Mutex::new(Vec::new()),
            props: Mutex::new(AudioProperties::default()),
        }
    }

    fn emit(&self, ev: BackendEvent) {
        let _ = self.events_tx.send(ev);
    }
}

#[async_trait]
impl AudioBackend for MockAudioBackend {
    async fn start(&self) -> AudioResult<()> {
        Ok(())
    }
    async fn stop(&self) -> AudioResult<()> {
        Ok(())
    }
    async fn load_url(&self, url: &str) -> AudioResult<()> {
        self.loaded_urls.lock().unwrap().push(url.to_string());
        Ok(())
    }
    async fn append_gapless(&self, url: &str) -> AudioResult<()> {
        self.gapless_urls.lock().unwrap().push(url.to_string());
        Ok(())
    }
    async fn clear_gapless_tail(&self) -> AudioResult<()> {
        self.gapless_urls.lock().unwrap().clear();
        Ok(())
    }
    async fn stop_track(&self) -> AudioResult<()> {
        Ok(())
    }
    async fn toggle_pause(&self) -> AudioResult<bool> {
        let mut p = self.props.lock().unwrap();
        p.paused = !p.paused;
        Ok(p.paused)
    }
    async fn set_paused(&self, paused: bool) -> AudioResult<()> {
        self.props.lock().unwrap().paused = paused;
        Ok(())
    }
    async fn seek(&self, _seconds: f64, _relative: bool) -> AudioResult<()> {
        Ok(())
    }
    async fn set_volume(&self, volume: u8) -> AudioResult<u8> {
        let v = volume.clamp(0, 130);
        self.props.lock().unwrap().volume = v;
        Ok(v)
    }
    async fn apply_settings(&self, _settings: &Settings) -> AudioResult<()> {
        Ok(())
    }
    fn subscribe_events(&self) -> broadcast::Receiver<BackendEvent> {
        self.events_tx.subscribe()
    }
    fn properties(&self) -> AudioProperties {
        *self.props.lock().unwrap()
    }
}

struct CountingStreamBackend {
    counter: AtomicUsize,
}

impl CountingStreamBackend {
    fn new() -> Self {
        Self {
            counter: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl StreamBackend for CountingStreamBackend {
    async fn resolve(&self, video_id: &str, _quality: AudioQuality) -> AudioResult<RawStream> {
        let n = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(RawStream {
            url: format!("https://stream.invalid/{video_id}?attempt={n}&expire=9999999999"),
            codec: "opus".into(),
            bitrate: 128,
            sample_rate: 48_000,
            user_agent: None,
        })
    }
}

#[derive(Default)]
struct MockEngineCatalog {
    radio_calls: AtomicUsize,
    seen_continuations: Mutex<Vec<Option<String>>>,
    telemetry_calls: AtomicUsize,
}

#[async_trait]
impl CatalogApi for MockEngineCatalog {
    async fn home_feed(&self, _region: &str) -> CoreResult<HomeFeed> {
        Ok(HomeFeed::default())
    }
    async fn search(
        &self,
        _category: SearchCategory,
        _query: &str,
        _limit: usize,
    ) -> CoreResult<Vec<SearchItem>> {
        Ok(Vec::new())
    }
    async fn suggestions(&self, _query: &str) -> CoreResult<Vec<String>> {
        Ok(Vec::new())
    }
    async fn artist(&self, _browse_id: &str) -> CoreResult<Artist> {
        Err(CoreError::NotFound("unused".into()))
    }
    async fn artist_releases(
        &self,
        _browse_id: &str,
        _params: &str,
        _kind: &str,
    ) -> CoreResult<Vec<AlbumSummary>> {
        Ok(Vec::new())
    }
    async fn album(&self, _browse_id: &str) -> CoreResult<Album> {
        Err(CoreError::NotFound("unused".into()))
    }
    async fn playlist(&self, _playlist_id: &str, _limit: usize) -> CoreResult<Playlist> {
        Err(CoreError::NotFound("unused".into()))
    }
    async fn radio(
        &self,
        _video_id: Option<&str>,
        _playlist_id: Option<&str>,
        continuation: Option<&str>,
        _limit: usize,
    ) -> CoreResult<RadioBatch> {
        let call = self.radio_calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.seen_continuations
            .lock()
            .unwrap()
            .push(continuation.map(str::to_string));

        if call == 1 {
            Ok(RadioBatch {
                tracks: vec![
                    Track::minimal("r1", "Radio 1"),
                    Track::minimal("r2", "Radio 2"),
                ],
                continuation: Some("CONT_PAGE_2".into()),
                playlist_id: Some("RDAMVM_seed".into()),
            })
        } else {
            Ok(RadioBatch {
                tracks: vec![
                    Track::minimal("r2", "Radio 2 dup"),
                    Track::minimal("r3", "Radio 3"),
                    Track::minimal("r4", "Radio 4"),
                    Track::minimal("r5", "Radio 5"),
                ],
                continuation: Some("CONT_PAGE_3".into()),
                playlist_id: Some("RDAMVM_seed".into()),
            })
        }
    }
    async fn song_related(&self, _video_id: &str) -> CoreResult<Vec<SearchItem>> {
        Ok(Vec::new())
    }
    async fn yt_lyrics(&self, _video_id: &str) -> CoreResult<Option<Lyrics>> {
        Ok(None)
    }
    async fn library_playlists(&self) -> CoreResult<Vec<PlaylistSummary>> {
        Ok(Vec::new())
    }
    async fn library_liked_songs(&self, _limit: usize) -> CoreResult<Vec<Track>> {
        Ok(Vec::new())
    }
    async fn library_albums(&self) -> CoreResult<Vec<AlbumSummary>> {
        Ok(Vec::new())
    }
    async fn library_artists(&self) -> CoreResult<Vec<ArtistSummary>> {
        Ok(Vec::new())
    }
    async fn create_playlist(
        &self,
        _title: &str,
        _description: &str,
        _video_ids: &[String],
    ) -> CoreResult<String> {
        Ok("PL1".into())
    }
    async fn edit_playlist(
        &self,
        _remote_playlist_id: &str,
        _actions: &[PlaylistEditAction],
    ) -> CoreResult<PlaylistEditResult> {
        Ok(PlaylistEditResult::default())
    }
    async fn delete_playlist(&self, _remote_playlist_id: &str) -> CoreResult<()> {
        Ok(())
    }
    async fn rate_song(&self, _video_id: &str, _rating: SongRating) -> CoreResult<()> {
        Ok(())
    }
    async fn set_artist_subscription(&self, _channel_id: &str, _subscribe: bool) -> CoreResult<()> {
        Ok(())
    }
    async fn set_album_library_status(&self, _browse_id: &str, _save: bool) -> CoreResult<()> {
        Ok(())
    }
    async fn set_playlist_library_status(&self, _playlist_id: &str, _save: bool) -> CoreResult<()> {
        Ok(())
    }
    async fn fetch_playback_tracking(
        &self,
        video_id: &str,
        report_enabled: bool,
    ) -> CoreResult<Option<PlaybackTracking>> {
        if !report_enabled {
            return Ok(None);
        }
        self.telemetry_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Some(PlaybackTracking {
            video_id: video_id.to_string(),
            playback_url: Some("https://example.invalid/playback".into()),
            watchtime_url: Some("https://example.invalid/watchtime".into()),
            length_secs: 200,
        }))
    }
    async fn report_playback(
        &self,
        _tracking: &PlaybackTracking,
        _cpn: &str,
        _event: PlaybackTelemetryEvent,
        report_enabled: bool,
    ) -> CoreResult<()> {
        if report_enabled {
            self.telemetry_calls.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[tokio::test]
async fn engine_retries_expired_403_once_autofills_radio_continuation_and_gates_telemetry() {
    let dir = tempdir().unwrap();
    let paths = AppPaths::from_roots(
        dir.path().join("cfg"),
        dir.path().join("data"),
        dir.path().join("cache"),
        dir.path().join("rt"),
    );
    let store = Arc::new(UserStore::open(&paths).unwrap());
    let cache = Arc::new(CacheManager::open(&paths, CacheSettings::default()).unwrap());
    let auth = Arc::new(AuthManager::new_anonymous(
        Arc::new(MemorySecretStore::new()),
        musick_core::build_http_client(),
    ));
    let catalog = Arc::new(MockEngineCatalog::default());
    let backend = Arc::new(MockAudioBackend::new());
    let stream_backend = Arc::new(CountingStreamBackend::new());
    let resolver = Arc::new(HybridStreamResolver::with_backends(
        None,
        stream_backend.clone(),
        AudioQuality::Best,
        Arc::new(|| 1000.0),
    ));

    let mut settings = Settings::default();
    settings.lyrics.enabled = false;
    settings.privacy.report_playback_history = true; // MUST stay ignored while Anonymous!

    let lyrics = Arc::new(
        LyricsService::new(
            Some(catalog.clone()),
            cache,
            musick_core::build_http_client(),
            settings.clone(),
        )
        .with_ytdlp_captions(false),
    );

    let (handle, join) = PlayerEngine::spawn(PlayerEngineDeps {
        backend: backend.clone(),
        resolver: resolver.clone(),
        catalog: catalog.clone(),
        lyrics,
        store,
        auth: auth.clone(),
        settings,
    });

    // 1. Start Radio -> loads r1, gaplessly appends r2, and autofills continuation CONT_PAGE_2
    handle
        .send(AppCommand::StartRadio {
            video_id: Some("seed1".into()),
            playlist_id: None,
            seed_track: None,
        })
        .await;

    // Wait for autofill & gapless
    for _ in 0..30 {
        if handle.queue().tracks.len() >= 5 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let q = handle.queue();
    assert_eq!(q.tracks.len(), 5);
    assert_eq!(
        q.tracks
            .iter()
            .map(|t| t.video_id.as_str())
            .collect::<Vec<_>>(),
        vec!["r1", "r2", "r3", "r4", "r5"] // deduplicated r2!
    );
    assert!(
        catalog
            .seen_continuations
            .lock()
            .unwrap()
            .contains(&Some("CONT_PAGE_2".to_string()))
    );
    // Telemetry MUST be strictly 0 in Anonymous mode!
    assert_eq!(catalog.telemetry_calls.load(Ordering::SeqCst), 0);

    // 2. Simulate mid-playback 403/FileError -> engine MUST invalidate cache & retry once!
    let before_loads = backend.loaded_urls.lock().unwrap().len();
    backend.emit(BackendEvent::FileError("HTTP 403 Forbidden".into()));
    for _ in 0..20 {
        if backend.loaded_urls.lock().unwrap().len() > before_loads {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let urls = backend.loaded_urls.lock().unwrap().clone();
    assert!(urls.len() > before_loads);
    assert_ne!(urls[0], urls[urls.len() - 1]); // freshly re-resolved URL!
    assert!(handle.snapshot().error.is_none());

    // 3. Second failure on same track surfaces the error instead of infinite-looping
    backend.emit(BackendEvent::FileError("Stream dead".into()));
    for _ in 0..20 {
        if handle.snapshot().error.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(handle.snapshot().error.as_deref(), Some("Stream dead"));

    // 4. Authenticate + play track -> opt-in telemetry now fires cleanly
    auth.import_browser_headers("cookie: SAPISID=live_tok", None)
        .unwrap();
    handle
        .send(AppCommand::PlayTracks {
            tracks: vec![Track::minimal("t_auth", "Auth Track")],
            start_index: 0,
            autoplay: false,
        })
        .await;
    for _ in 0..30 {
        if catalog.telemetry_calls.load(Ordering::SeqCst) >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(catalog.telemetry_calls.load(Ordering::SeqCst) >= 2);

    handle.send(AppCommand::Shutdown).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), join).await;
}

#[tokio::test]
async fn eof_with_autoplay_autofills_and_advances_and_properties_do_not_resurrect_errored_playback()
{
    let dir = tempdir().unwrap();
    let paths = AppPaths::from_roots(
        dir.path().join("cfg"),
        dir.path().join("data"),
        dir.path().join("cache"),
        dir.path().join("rt"),
    );
    let store = Arc::new(UserStore::open(&paths).unwrap());
    let cache = Arc::new(CacheManager::open(&paths, CacheSettings::default()).unwrap());
    let auth = Arc::new(AuthManager::new_anonymous(
        Arc::new(MemorySecretStore::new()),
        musick_core::build_http_client(),
    ));
    let catalog = Arc::new(MockEngineCatalog::default());
    let backend = Arc::new(MockAudioBackend::new());
    let resolver = Arc::new(HybridStreamResolver::with_backends(
        None,
        Arc::new(CountingStreamBackend::new()),
        AudioQuality::Best,
        Arc::new(|| 1000.0),
    ));

    let mut settings = Settings::default();
    settings.lyrics.enabled = false;
    let lyrics = Arc::new(
        LyricsService::new(
            Some(catalog.clone()),
            cache,
            musick_core::build_http_client(),
            settings.clone(),
        )
        .with_ytdlp_captions(false),
    );

    let (handle, join) = PlayerEngine::spawn(PlayerEngineDeps {
        backend: backend.clone(),
        resolver,
        catalog,
        lyrics,
        store,
        auth,
        settings,
    });

    // 1. Play 1 track with autoplay=true -> autofills radio and advances cleanly on EOF
    handle
        .send(AppCommand::PlayTracks {
            tracks: vec![Track::minimal("solo1", "Solo Track")],
            start_index: 0,
            autoplay: true,
        })
        .await;
    for _ in 0..30 {
        if handle.queue().tracks.len() >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(handle.queue().tracks.len() >= 3);

    // 2. Simulate fatal error on track (two failures so retry exhausts), then emit BackendEvent::Properties
    backend.emit(BackendEvent::FileError("Fatal 1".into()));
    tokio::time::sleep(Duration::from_millis(50)).await;
    backend.emit(BackendEvent::FileError("Fatal 2".into()));
    for _ in 0..30 {
        if handle.snapshot().error.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!handle.snapshot().playing);

    // A late Properties event with paused=false MUST NOT flip playing back to true while errored!
    backend.emit(BackendEvent::Properties(AudioProperties {
        paused: false,
        volume: 85,
        ..AudioProperties::default()
    }));
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(!handle.snapshot().playing);
    assert!(!handle.position().buffering);
    assert_eq!(handle.snapshot().volume, 85);

    handle.send(AppCommand::Shutdown).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), join).await;
}

struct TaggedStreamBackend {
    tag: &'static str,
}

#[async_trait]
impl StreamBackend for TaggedStreamBackend {
    async fn resolve(&self, video_id: &str, _quality: AudioQuality) -> AudioResult<RawStream> {
        Ok(RawStream {
            url: format!(
                "https://stream.invalid/{video_id}?backend={}&expire=9999999999",
                self.tag
            ),
            codec: "opus".into(),
            bitrate: 130,
            sample_rate: 48_000,
            user_agent: Some(format!("UA-{}", self.tag)),
        })
    }
}

#[tokio::test]
async fn fast_path_loading_failed_retries_via_fallback_and_final_error_clears_buffering() {
    let dir = tempdir().unwrap();
    let paths = AppPaths::from_roots(
        dir.path().join("cfg"),
        dir.path().join("data"),
        dir.path().join("cache"),
        dir.path().join("rt"),
    );
    let store = Arc::new(UserStore::open(&paths).unwrap());
    let cache = Arc::new(CacheManager::open(&paths, CacheSettings::default()).unwrap());
    let auth = Arc::new(AuthManager::new_anonymous(
        Arc::new(MemorySecretStore::new()),
        musick_core::build_http_client(),
    ));
    let catalog = Arc::new(MockEngineCatalog::default());
    let backend = Arc::new(MockAudioBackend::new());
    let resolver = Arc::new(HybridStreamResolver::with_backends(
        Some(Arc::new(TaggedStreamBackend { tag: "rustypipe" })),
        Arc::new(TaggedStreamBackend { tag: "ytdlp" }),
        AudioQuality::Best,
        Arc::new(|| 1000.0),
    ));

    let mut settings = Settings::default();
    settings.lyrics.enabled = false;
    let lyrics = Arc::new(
        LyricsService::new(
            Some(catalog.clone()),
            cache,
            musick_core::build_http_client(),
            settings.clone(),
        )
        .with_ytdlp_captions(false),
    );

    let (handle, join) = PlayerEngine::spawn(PlayerEngineDeps {
        backend: backend.clone(),
        resolver: resolver.clone(),
        catalog,
        lyrics,
        store,
        auth,
        settings,
    });

    // 1. Initial play hits fast-path ("rustypipe")
    handle
        .send(AppCommand::PlayTracks {
            tracks: vec![Track::minimal("creed1", "One Last Breath")],
            start_index: 0,
            autoplay: false,
        })
        .await;
    for _ in 0..30 {
        if !backend.loaded_urls.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(backend.loaded_urls.lock().unwrap()[0].contains("backend=rustypipe"));

    // 2. mpv emits "loading failed" -> engine MUST trip circuit breaker & retry directly on "ytdlp"
    backend.emit(BackendEvent::FileError("loading failed".into()));
    for _ in 0..30 {
        if backend.loaded_urls.lock().unwrap().len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let urls = backend.loaded_urls.lock().unwrap().clone();
    assert_eq!(urls.len(), 2);
    assert!(urls[1].contains("backend=ytdlp"));
    assert!(resolver.is_fast_circuit_open());
    assert!(handle.snapshot().error.is_none());

    // 3. If fallback also fails, error is surfaced AND position.buffering MUST be false
    backend.emit(BackendEvent::FileError("loading failed".into()));
    for _ in 0..30 {
        if handle.snapshot().error.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(handle.snapshot().error.as_deref(), Some("loading failed"));
    assert!(!handle.position().buffering);
    assert!(!handle.snapshot().playing);

    handle.send(AppCommand::Shutdown).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), join).await;
}

#[test]
fn cava_config_and_frame_parsing_work_cleanly() {
    let cfg = build_cava_config(
        15,
        "pipewire",
        1000,
        30,
        CavaSmoothing::Smooth,
        CavaAudioSource::Auto,
    );
    assert!(cfg.contains("bars = 16")); // rounded up to even
    assert!(cfg.contains("method = pipewire"));
    assert!(cfg.contains("source = auto"));
    assert!(cfg.contains("monstercat = 1"));

    let parsed = parse_cava_frame("\x1b[0m0;500;1000;\n", 1000).unwrap();
    assert_eq!(parsed.len(), 3);
    assert!((parsed[0] - 0.0).abs() < 1e-4);
    assert!((parsed[1] - 0.5).abs() < 1e-4);
    assert!((parsed[2] - 1.0).abs() < 1e-4);
    assert!(parse_cava_frame("not;numbers", 1000).is_none());

    let resampled = resample_bars(&[0.0, 1.0], 5);
    assert_eq!(resampled.len(), 5);
    assert!((resampled[2] - 0.5).abs() < 1e-4);
}
