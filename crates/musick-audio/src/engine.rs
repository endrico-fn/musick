use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use rand::RngExt;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::{AbortHandle, JoinHandle};
use tracing::debug;

use musick_core::{
    AppCommand, AuthManager, CatalogApi, Lyrics, LyricsService, PlaybackSnapshot,
    PlaybackTelemetryEvent, PlaybackTracking, PlayerEvent, PositionTick, QueueSnapshot, RadioBatch,
    RepeatMode, Settings, Track, UserStore,
};

use crate::backend::{AudioBackend, BackendEvent};
use crate::queue::PlayQueue;
use crate::resolver::{HybridStreamResolver, ResolvedStream};

const RADIO_AUTOFILL_THRESHOLD: usize = 3;
const CPN_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn generate_cpn() -> String {
    let mut rng = rand::rng();
    (0..16)
        .map(|_| {
            let idx = rng.random_range(0..CPN_ALPHABET.len());
            CPN_ALPHABET[idx] as char
        })
        .collect()
}

#[derive(Clone)]
pub struct PlayerEngineHandle {
    pub cmd_tx: mpsc::Sender<AppCommand>,
    pub pos_rx: watch::Receiver<PositionTick>,
    pub snapshot_rx: watch::Receiver<PlaybackSnapshot>,
    pub queue_rx: watch::Receiver<QueueSnapshot>,
    pub lyrics_rx: watch::Receiver<(Option<String>, Lyrics)>,
    pub events_tx: broadcast::Sender<PlayerEvent>,
}

impl PlayerEngineHandle {
    pub async fn send(&self, cmd: AppCommand) {
        let _ = self.cmd_tx.send(cmd).await;
    }

    pub fn try_send(&self, cmd: AppCommand) {
        let _ = self.cmd_tx.try_send(cmd);
    }

    pub fn snapshot(&self) -> PlaybackSnapshot {
        self.snapshot_rx.borrow().clone()
    }

    pub fn position(&self) -> PositionTick {
        *self.pos_rx.borrow()
    }

    pub fn queue(&self) -> QueueSnapshot {
        self.queue_rx.borrow().clone()
    }

    pub fn lyrics(&self) -> (Option<String>, Lyrics) {
        self.lyrics_rx.borrow().clone()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<PlayerEvent> {
        self.events_tx.subscribe()
    }
}

pub struct PlayerEngineDeps {
    pub backend: Arc<dyn AudioBackend>,
    pub resolver: Arc<HybridStreamResolver>,
    pub catalog: Arc<dyn CatalogApi>,
    pub lyrics: Arc<LyricsService>,
    pub store: Arc<UserStore>,
    pub auth: Arc<AuthManager>,
    pub settings: Settings,
}

enum InternalMsg {
    StreamResolved {
        epoch: u64,
        track: Track,
        result: Result<ResolvedStream, String>,
    },
    RadioStarted {
        epoch: u64,
        playlist_id: Option<String>,
        seed_track: Option<Track>,
        result: Result<RadioBatch, String>,
    },
    LyricsLoaded {
        epoch: u64,
        video_id: String,
        lyrics: Lyrics,
    },
    LyricsTranslated {
        epoch: u64,
        video_id: String,
        lyrics: Lyrics,
    },
    RadioAutofill {
        tracks: Vec<Track>,
        continuation: Option<String>,
        playlist_id: Option<String>,
    },
    TrackingLoaded {
        epoch: u64,
        video_id: String,
        cpn: String,
        tracking: PlaybackTracking,
    },
    GaplessResolved {
        epoch: u64,
        video_id: String,
        url: String,
        user_agent: Option<String>,
    },
}

pub struct PlayerEngine {
    queue: PlayQueue,
    backend: Arc<dyn AudioBackend>,
    resolver: Arc<HybridStreamResolver>,
    catalog: Arc<dyn CatalogApi>,
    lyrics_svc: Arc<LyricsService>,
    store: Arc<UserStore>,
    auth: Arc<AuthManager>,
    settings: Settings,

    snapshot: PlaybackSnapshot,
    position: PositionTick,
    current_lyrics: (Option<String>, Lyrics),

    pos_tx: watch::Sender<PositionTick>,
    snapshot_tx: watch::Sender<PlaybackSnapshot>,
    queue_tx: watch::Sender<QueueSnapshot>,
    lyrics_tx: watch::Sender<(Option<String>, Lyrics)>,
    events_tx: broadcast::Sender<PlayerEvent>,

    internal_tx: mpsc::UnboundedSender<InternalMsg>,
    load_gen: u64,
    resolve_task: Option<AbortHandle>,
    radio_start_task: Option<AbortHandle>,
    retried_video_id: Option<String>,
    last_stream_from_fast: bool,
    gapless_appended_video_id: Option<String>,

    radio_playlist_id: Option<String>,
    radio_continuation: Option<String>,
    radio_inflight: bool,
    pending_autofill_advance: bool,

    active_tracking: Option<(String, String, PlaybackTracking)>,
    watchtime_reported: bool,
}

impl PlayerEngine {
    pub fn spawn(deps: PlayerEngineDeps) -> (PlayerEngineHandle, JoinHandle<()>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(128);
        let init_snap = PlaybackSnapshot {
            volume: deps.settings.player.volume,
            normalize_audio: deps.settings.player.normalize_audio,
            show_translation: deps.settings.lyrics.translate_to_id,
            ..PlaybackSnapshot::default()
        };
        let (pos_tx, pos_rx) = watch::channel(PositionTick::default());
        let (snapshot_tx, snapshot_rx) = watch::channel(init_snap.clone());
        let (queue_tx, queue_rx) = watch::channel(QueueSnapshot::default());
        let (lyrics_tx, lyrics_rx) = watch::channel((None, Lyrics::default()));
        let (events_tx, _) = broadcast::channel(128);
        let (internal_tx, internal_rx) = mpsc::unbounded_channel();

        let engine = Self {
            queue: PlayQueue::new(),
            backend: deps.backend,
            resolver: deps.resolver,
            catalog: deps.catalog,
            lyrics_svc: deps.lyrics,
            store: deps.store,
            auth: deps.auth,
            settings: deps.settings,
            snapshot: init_snap,
            position: PositionTick::default(),
            current_lyrics: (None, Lyrics::default()),
            pos_tx,
            snapshot_tx,
            queue_tx,
            lyrics_tx,
            events_tx: events_tx.clone(),
            internal_tx,
            load_gen: 0,
            resolve_task: None,
            radio_start_task: None,
            retried_video_id: None,
            last_stream_from_fast: false,
            gapless_appended_video_id: None,
            radio_playlist_id: None,
            radio_continuation: None,
            radio_inflight: false,
            pending_autofill_advance: false,
            active_tracking: None,
            watchtime_reported: false,
        };

        let handle = PlayerEngineHandle {
            cmd_tx,
            pos_rx,
            snapshot_rx,
            queue_rx,
            lyrics_rx,
            events_tx,
        };

        let join = tokio::spawn(engine.run_loop(cmd_rx, internal_rx));
        (handle, join)
    }

    async fn run_loop(
        mut self,
        mut cmd_rx: mpsc::Receiver<AppCommand>,
        mut internal_rx: mpsc::UnboundedReceiver<InternalMsg>,
    ) {
        let mut backend_rx = self.backend.subscribe_events();
        loop {
            tokio::select! {
                maybe_cmd = cmd_rx.recv() => {
                    let Some(cmd) = maybe_cmd else {
                        break;
                    };
                    if matches!(cmd, AppCommand::Shutdown) {
                        self.cancel_inflight_loads();
                        let _ = self.backend.stop().await;
                        break;
                    }
                    self.handle_command(cmd).await;
                }
                maybe_ev = backend_rx.recv() => {
                    match maybe_ev {
                        Ok(ev) => self.handle_backend_event(ev).await,
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                maybe_int = internal_rx.recv() => {
                    if let Some(msg) = maybe_int {
                        self.handle_internal(msg).await;
                    }
                }
            }
        }
    }

    fn publish_snapshot(&mut self) {
        self.snapshot.upcoming_count = self.queue.upcoming_tracks().len();
        self.snapshot.shuffle = self.queue.shuffle_enabled();
        self.snapshot.repeat = self.queue.repeat_mode();
        let _ = self.snapshot_tx.send(self.snapshot.clone());
        let _ = self
            .events_tx
            .send(PlayerEvent::Snapshot(Box::new(self.snapshot.clone())));
    }

    fn publish_queue(&mut self) {
        self.snapshot.queue_rev += 1;
        let qsnap = self.queue.to_snapshot();
        let _ = self.queue_tx.send(qsnap.clone());
        let _ = self.events_tx.send(PlayerEvent::Queue(qsnap));
        self.publish_snapshot();
    }

    fn publish_lyrics(&mut self, video_id: Option<String>, lyrics: Lyrics) {
        self.snapshot.lyrics_rev += 1;
        self.current_lyrics = (video_id.clone(), lyrics.clone());
        let active_idx = self
            .current_lyrics
            .1
            .active_line_index(self.position.position_secs);
        self.position.active_lyric_idx = active_idx;
        let _ = self.pos_tx.send(self.position);
        let _ = self.lyrics_tx.send((video_id.clone(), lyrics.clone()));
        let _ = self.events_tx.send(PlayerEvent::Lyrics {
            track_video_id: video_id,
            lyrics,
        });
        self.publish_snapshot();
    }

    fn cancel_inflight_loads(&mut self) {
        if let Some(h) = self.resolve_task.take() {
            h.abort();
        }
        if let Some(h) = self.radio_start_task.take() {
            h.abort();
        }
    }

    async fn handle_command(&mut self, cmd: AppCommand) {
        match cmd {
            AppCommand::PlayTracks {
                tracks,
                start_index,
                autoplay,
            } => {
                self.radio_continuation = None;
                self.radio_playlist_id = None;
                self.snapshot.autoplay_active = autoplay && self.settings.player.autoplay;
                self.queue.set_tracks(tracks, start_index);
                self.publish_queue();
                if let Some(cur) = self.queue.current().cloned() {
                    self.load_track(cur, false).await;
                }
            }
            AppCommand::PlayNext { tracks } => {
                let was_empty = self.queue.is_empty();
                self.queue.play_next_many(tracks);
                self.invalidate_gapless_tail().await;
                self.publish_queue();
                if was_empty && let Some(cur) = self.queue.current().cloned() {
                    self.load_track(cur, false).await;
                } else {
                    self.schedule_prefetch_and_gapless();
                }
            }
            AppCommand::AppendTracks { tracks } => {
                let was_empty = self.queue.is_empty();
                self.queue.append(tracks);
                self.publish_queue();
                if was_empty && let Some(cur) = self.queue.current().cloned() {
                    self.load_track(cur, false).await;
                } else {
                    self.schedule_prefetch_and_gapless();
                }
            }
            AppCommand::StartRadio {
                video_id,
                playlist_id,
                seed_track,
            } => {
                self.cancel_inflight_loads();
                self.load_gen += 1;
                let epoch = self.load_gen;
                self.snapshot.resolving = true;
                self.snapshot.restored = false;
                self.snapshot.error = None;
                self.publish_snapshot();

                let catalog = self.catalog.clone();
                let tx = self.internal_tx.clone();
                let pid = playlist_id.clone();
                let jh = tokio::spawn(async move {
                    let res = catalog
                        .radio(video_id.as_deref(), pid.as_deref(), None, 25)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(InternalMsg::RadioStarted {
                        epoch,
                        playlist_id,
                        seed_track,
                        result: res,
                    });
                });
                self.radio_start_task = Some(jh.abort_handle());
            }
            AppCommand::RestoreTrack { track } => {
                if !self.queue.is_empty() || self.snapshot.track.is_some() {
                    return;
                }
                self.cancel_inflight_loads();
                let dur = track.duration.map(f64::from).unwrap_or(0.0);
                self.queue.set_tracks(vec![track.clone()], 0);
                self.snapshot.track = Some(track);
                self.snapshot.track_rev += 1;
                self.snapshot.restored = true;
                self.snapshot.playing = false;
                self.snapshot.paused = true;
                self.snapshot.resolving = false;
                self.snapshot.error = None;
                self.snapshot.audio_info.clear();
                self.position = PositionTick {
                    position_secs: 0.0,
                    duration_secs: dur,
                    buffering: false,
                    active_lyric_idx: None,
                };
                let _ = self.pos_tx.send(self.position);
                self.publish_queue();
            }
            AppCommand::TogglePause => {
                if (self.snapshot.restored
                    || self.snapshot.track.is_none()
                    || self.snapshot.error.is_some())
                    && let Some(cur) = self.queue.current().cloned()
                {
                    self.load_track(cur, false).await;
                    return;
                }
                if let Ok(paused) = self.backend.toggle_pause().await {
                    self.snapshot.paused = paused;
                    self.snapshot.playing = !paused && self.snapshot.track.is_some();
                    self.publish_snapshot();
                }
            }
            AppCommand::Next => {
                self.flush_watchtime_if_needed().await;
                self.invalidate_gapless_tail().await;
                if let Some(next) = self.queue.advance(true) {
                    self.pending_autofill_advance = false;
                    self.publish_queue();
                    self.load_track(next, false).await;
                } else if self.snapshot.autoplay_active && self.settings.player.autoplay {
                    self.pending_autofill_advance = true;
                    self.snapshot.resolving = true;
                    self.publish_snapshot();
                    self.maybe_autofill_radio().await;
                }
            }
            AppCommand::Previous => {
                if self.position.position_secs > 4.0 {
                    let _ = self.backend.seek(0.0, false).await;
                    return;
                }
                if let Some(prev) = self.queue.retreat() {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    self.load_track(prev, false).await;
                } else if self.snapshot.track.is_some() && !self.snapshot.restored {
                    let _ = self.backend.seek(0.0, false).await;
                }
            }
            AppCommand::SeekRelative { seconds } => {
                let _ = self.backend.seek(seconds, true).await;
            }
            AppCommand::SeekAbsolute { seconds } => {
                let _ = self.backend.seek(seconds.max(0.0), false).await;
            }
            AppCommand::SetVolume { volume } => {
                if let Ok(v) = self.backend.set_volume(volume).await {
                    self.snapshot.volume = v;
                    self.settings.player.volume = v;
                    self.publish_snapshot();
                }
            }
            AppCommand::AdjustVolume { delta } => {
                let target = (i16::from(self.snapshot.volume) + delta).clamp(0, 130) as u8;
                if let Ok(v) = self.backend.set_volume(target).await {
                    self.snapshot.volume = v;
                    self.settings.player.volume = v;
                    self.publish_snapshot();
                }
            }
            AppCommand::ToggleShuffle => {
                self.queue.toggle_shuffle();
                self.invalidate_gapless_tail().await;
                self.publish_queue();
                self.schedule_prefetch_and_gapless();
            }
            AppCommand::CycleRepeat => {
                self.queue.cycle_repeat();
                self.invalidate_gapless_tail().await;
                self.publish_queue();
                self.schedule_prefetch_and_gapless();
            }
            AppCommand::JumpToQueueIndex { index } => {
                if let Some(track) = self.queue.jump(index) {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    self.load_track(track, false).await;
                }
            }
            AppCommand::MoveQueueItem { from, to } => {
                if self.queue.move_item(from, to) {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    self.schedule_prefetch_and_gapless();
                }
            }
            AppCommand::RemoveQueueItem { index } => {
                let removed_active = self.queue.index() == Some(index);
                if self.queue.remove(index) {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    if removed_active {
                        if let Some(cur) = self.queue.current().cloned() {
                            self.load_track(cur, false).await;
                        } else {
                            self.cancel_inflight_loads();
                            self.load_gen += 1;
                            let _ = self.backend.stop_track().await;
                            self.snapshot.track = None;
                            self.snapshot.playing = false;
                            self.snapshot.resolving = false;
                            self.snapshot.restored = false;
                            self.position.buffering = false;
                            let _ = self.pos_tx.send(self.position);
                            self.publish_snapshot();
                        }
                    } else {
                        self.schedule_prefetch_and_gapless();
                    }
                }
            }
            AppCommand::PromoteQueueItemToNext { index } => {
                if self.queue.move_to_next(index) {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    self.schedule_prefetch_and_gapless();
                }
            }
            AppCommand::UndoQueue => {
                if self.queue.undo() {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    self.schedule_prefetch_and_gapless();
                }
            }
            AppCommand::DeduplicateQueue => {
                let removed = self.queue.deduplicate();
                if removed > 0 {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                    let _ = self.events_tx.send(PlayerEvent::StatusToast(format!(
                        "Removed {removed} duplicate track(s)"
                    )));
                }
            }
            AppCommand::ClearUpcomingQueue => {
                let removed = self.queue.clear_upcoming();
                if removed > 0 {
                    self.invalidate_gapless_tail().await;
                    self.publish_queue();
                }
            }
            AppCommand::ClearQueue => {
                self.cancel_inflight_loads();
                self.load_gen += 1;
                self.queue.clear();
                self.invalidate_gapless_tail().await;
                let _ = self.backend.stop_track().await;
                self.snapshot.track = None;
                self.snapshot.playing = false;
                self.snapshot.paused = false;
                self.snapshot.resolving = false;
                self.snapshot.restored = false;
                self.snapshot.autoplay_active = false;
                self.snapshot.error = None;
                self.position = PositionTick::default();
                let _ = self.pos_tx.send(self.position);
                self.publish_queue();
                self.publish_lyrics(None, Lyrics::default());
            }
            AppCommand::ToggleLyricsTranslation => {
                self.snapshot.show_translation = !self.snapshot.show_translation;
                self.settings.lyrics.translate_to_id = self.snapshot.show_translation;
                self.lyrics_svc.update_settings(self.settings.clone());

                if self.snapshot.show_translation
                    && !self.current_lyrics.1.is_empty()
                    && self.current_lyrics.1.translation_lang != "id"
                    && let Some(vid) = self.current_lyrics.0.clone()
                {
                    self.snapshot.translating_lyrics = true;
                    self.publish_snapshot();
                    let lyr = self.current_lyrics.1.clone();
                    let lyr_svc = self.lyrics_svc.clone();
                    let tx = self.internal_tx.clone();
                    let epoch = self.load_gen;
                    tokio::spawn(async move {
                        let translated = lyr_svc.translate_lyrics(lyr, Some(&vid), "id").await;
                        let _ = tx.send(InternalMsg::LyricsTranslated {
                            epoch,
                            video_id: vid,
                            lyrics: translated,
                        });
                    });
                } else {
                    self.publish_snapshot();
                }
            }
            AppCommand::ApplySettings { settings } => {
                self.settings = settings.clone();
                self.resolver.set_quality(settings.player.audio_quality);
                self.lyrics_svc.update_settings(settings.clone());
                self.snapshot.normalize_audio = settings.player.normalize_audio;
                self.snapshot.volume = settings.player.volume;
                self.snapshot.show_translation = settings.lyrics.translate_to_id;
                let _ = self.backend.apply_settings(&settings).await;
                self.publish_snapshot();
            }
            AppCommand::Shutdown => {}
        }
    }

    async fn load_track(&mut self, track: Track, is_retry: bool) {
        self.cancel_inflight_loads();
        self.load_gen += 1;
        let epoch = self.load_gen;
        self.pending_autofill_advance = false;
        if !is_retry {
            self.retried_video_id = None;
        }
        self.gapless_appended_video_id = None;
        self.active_tracking = None;
        self.watchtime_reported = false;

        self.snapshot.track = Some(track.clone());
        self.snapshot.track_rev += 1;
        self.snapshot.resolving = true;
        self.snapshot.playing = false;
        self.snapshot.restored = false;
        self.snapshot.error = None;
        self.snapshot.audio_info.clear();
        self.position = PositionTick {
            position_secs: 0.0,
            duration_secs: track.duration.map(f64::from).unwrap_or(0.0),
            buffering: true,
            active_lyric_idx: None,
        };
        let _ = self.pos_tx.send(self.position);
        self.publish_snapshot();

        if !is_retry {
            self.publish_lyrics(Some(track.video_id.clone()), Lyrics::default());
            self.spawn_lyrics_load(epoch, track.clone());
        }

        if self.settings.privacy.save_history && !is_retry {
            let _ = self.store.record_play(&track);
        }

        let force_fallback = is_retry && self.last_stream_from_fast;
        let resolver = self.resolver.clone();
        let tx = self.internal_tx.clone();
        let jh = tokio::spawn(async move {
            let res = if force_fallback {
                resolver.resolve_fallback_only(&track.video_id).await
            } else {
                resolver.resolve(&track.video_id).await
            }
            .map_err(|e| e.to_string());
            let _ = tx.send(InternalMsg::StreamResolved {
                epoch,
                track,
                result: res,
            });
        });
        self.resolve_task = Some(jh.abort_handle());
    }

    /// evict stale or fast-path url and retry once via fallback on playback failure
    async fn handle_load_or_playback_error(&mut self, track: &Track, err: String) {
        if self.retried_video_id.as_deref() != Some(track.video_id.as_str()) {
            debug!(
                "Playback error on {} (fast={}), invalidating URL and retrying once: {err}",
                track.video_id, self.last_stream_from_fast
            );
            self.retried_video_id = Some(track.video_id.clone());
            self.resolver
                .report_playback_failure(&track.video_id, self.last_stream_from_fast);
            Box::pin(self.load_track(track.clone(), true)).await;
            return;
        }
        self.position.buffering = false;
        let _ = self.pos_tx.send(self.position);
        self.snapshot.resolving = false;
        self.snapshot.playing = false;
        self.snapshot.error = Some(err);
        self.publish_snapshot();
    }

    async fn handle_backend_event(&mut self, ev: BackendEvent) {
        match ev {
            BackendEvent::Properties(props) => {
                if self.snapshot.resolving {
                    return;
                }
                let active_idx = self.current_lyrics.1.active_line_index(props.position);
                self.position = PositionTick {
                    position_secs: props.position,
                    duration_secs: if props.duration > 0.0 {
                        props.duration
                    } else {
                        self.position.duration_secs
                    },
                    buffering: props.buffering,
                    active_lyric_idx: active_idx,
                };
                let _ = self.pos_tx.send(self.position);

                let should_update_play_state =
                    self.snapshot.error.is_none() && !self.snapshot.resolving;
                if props.paused != self.snapshot.paused || props.volume != self.snapshot.volume {
                    self.snapshot.paused = props.paused;
                    self.snapshot.volume = props.volume;
                    if should_update_play_state {
                        self.snapshot.playing = !props.paused && self.snapshot.track.is_some();
                    }
                    self.publish_snapshot();
                }

                // Opt-in watchtime ping once elapsed >= 30s
                if !self.watchtime_reported && props.position >= 30.0 {
                    self.flush_watchtime_if_needed().await;
                }
            }
            BackendEvent::Eof => {
                if self.snapshot.resolving {
                    return;
                }
                self.flush_watchtime_if_needed().await;
                if let Some(next) = self.queue.advance(false) {
                    self.pending_autofill_advance = false;
                    self.publish_queue();
                    let was_gapless =
                        self.gapless_appended_video_id.as_deref() == Some(next.video_id.as_str());
                    if was_gapless && self.queue.repeat_mode() != RepeatMode::One {
                        self.transition_metadata_after_gapless(next).await;
                    } else {
                        self.load_track(next, false).await;
                    }
                } else if self.snapshot.autoplay_active && self.settings.player.autoplay {
                    self.pending_autofill_advance = true;
                    self.snapshot.playing = false;
                    self.snapshot.resolving = true;
                    self.publish_snapshot();
                    self.maybe_autofill_radio().await;
                } else {
                    self.snapshot.playing = false;
                    self.publish_snapshot();
                }
            }
            BackendEvent::FileError(err) => {
                if self.snapshot.resolving {
                    return;
                }
                if let Some(cur) = self.snapshot.track.clone() {
                    self.handle_load_or_playback_error(&cur, err).await;
                } else {
                    self.position.buffering = false;
                    let _ = self.pos_tx.send(self.position);
                    self.snapshot.error = Some(err);
                    self.publish_snapshot();
                }
            }
            BackendEvent::Disconnected(err) => {
                self.position.buffering = false;
                let _ = self.pos_tx.send(self.position);
                self.snapshot.playing = false;
                self.snapshot.error = Some(err);
                self.publish_snapshot();
            }
        }
    }

    async fn transition_metadata_after_gapless(&mut self, track: Track) {
        self.cancel_inflight_loads();
        self.load_gen += 1;
        let epoch = self.load_gen;
        self.retried_video_id = None;
        self.gapless_appended_video_id = None;
        self.active_tracking = None;
        self.watchtime_reported = false;

        self.snapshot.track = Some(track.clone());
        self.snapshot.track_rev += 1;
        self.snapshot.playing = true;
        self.snapshot.paused = false;
        self.snapshot.resolving = false;
        self.snapshot.restored = false;
        self.snapshot.error = None;
        if let Some(cached) = self.resolver.get_cached(&track.video_id) {
            self.snapshot.audio_info = cached.quality_label();
            self.last_stream_from_fast = cached.from_fast_path;
        } else {
            self.last_stream_from_fast = false;
        }
        self.position = PositionTick {
            position_secs: 0.0,
            duration_secs: track.duration.map(f64::from).unwrap_or(0.0),
            buffering: false,
            active_lyric_idx: None,
        };
        let _ = self.pos_tx.send(self.position);
        self.publish_snapshot();

        if self.settings.privacy.save_history {
            let _ = self.store.record_play(&track);
        }
        self.publish_lyrics(Some(track.video_id.clone()), Lyrics::default());
        self.spawn_lyrics_load(epoch, track.clone());
        self.spawn_telemetry_start(epoch, track.video_id);
        self.schedule_prefetch_and_gapless();
        self.maybe_autofill_radio().await;
    }

    async fn handle_internal(&mut self, msg: InternalMsg) {
        match msg {
            InternalMsg::StreamResolved {
                epoch,
                track,
                result,
            } => {
                if epoch != self.load_gen {
                    return;
                }
                self.resolve_task = None;
                match result {
                    Ok(stream) => {
                        self.last_stream_from_fast = stream.from_fast_path;
                        self.snapshot.audio_info = stream.quality_label();
                        self.snapshot.resolving = false;
                        let _ = self
                            .backend
                            .set_user_agent(stream.user_agent.as_deref())
                            .await;
                        if let Err(err) = self.backend.load_url(&stream.url).await {
                            self.handle_load_or_playback_error(&track, err.to_string())
                                .await;
                            return;
                        }
                        self.snapshot.playing = true;
                        self.snapshot.paused = false;
                        self.publish_snapshot();

                        self.spawn_telemetry_start(epoch, track.video_id.clone());
                        self.schedule_prefetch_and_gapless();
                        self.maybe_autofill_radio().await;
                    }
                    Err(err) => {
                        self.position.buffering = false;
                        let _ = self.pos_tx.send(self.position);
                        self.snapshot.resolving = false;
                        self.snapshot.playing = false;
                        self.snapshot.error = Some(err);
                        self.publish_snapshot();
                    }
                }
            }
            InternalMsg::RadioStarted {
                epoch,
                playlist_id,
                seed_track,
                result,
            } => {
                if epoch != self.load_gen {
                    return;
                }
                self.radio_start_task = None;
                match result {
                    Ok(batch) => {
                        let mut tracks = batch.tracks;
                        if let Some(seed) = seed_track
                            && !tracks.iter().any(|t| t.video_id == seed.video_id)
                        {
                            tracks.insert(0, seed);
                        }
                        self.radio_continuation = batch.continuation;
                        self.radio_playlist_id = batch.playlist_id.or(playlist_id);
                        self.snapshot.autoplay_active = true;
                        self.queue.set_tracks(tracks, 0);
                        self.publish_queue();
                        if let Some(cur) = self.queue.current().cloned() {
                            self.load_track(cur, false).await;
                        } else {
                            self.snapshot.resolving = false;
                            self.publish_snapshot();
                        }
                    }
                    Err(err) => {
                        self.snapshot.resolving = false;
                        self.snapshot.error = Some(format!("Radio failed: {err}"));
                        self.publish_snapshot();
                    }
                }
            }
            InternalMsg::LyricsLoaded {
                epoch,
                video_id,
                lyrics,
            } => {
                if epoch == self.load_gen
                    && self
                        .snapshot
                        .track
                        .as_ref()
                        .is_some_and(|t| t.video_id == video_id)
                {
                    self.publish_lyrics(Some(video_id), lyrics);
                }
            }
            InternalMsg::LyricsTranslated {
                epoch,
                video_id,
                lyrics,
            } => {
                self.snapshot.translating_lyrics = false;
                if epoch == self.load_gen
                    && self
                        .snapshot
                        .track
                        .as_ref()
                        .is_some_and(|t| t.video_id == video_id)
                {
                    self.publish_lyrics(Some(video_id), lyrics);
                } else {
                    self.publish_snapshot();
                }
            }
            InternalMsg::RadioAutofill {
                tracks,
                continuation,
                playlist_id,
            } => {
                self.radio_inflight = false;
                if continuation.is_some() {
                    self.radio_continuation = continuation;
                }
                if playlist_id.is_some() {
                    self.radio_playlist_id = playlist_id;
                }
                let existing: HashSet<String> = self
                    .queue
                    .tracks()
                    .iter()
                    .map(|t| t.video_id.clone())
                    .collect();
                let fresh: Vec<Track> = tracks
                    .into_iter()
                    .filter(|t| !existing.contains(&t.video_id))
                    .collect();
                if !fresh.is_empty() {
                    self.queue.append(fresh);
                    self.publish_queue();
                    if self.pending_autofill_advance {
                        self.pending_autofill_advance = false;
                        if let Some(next) = self.queue.advance(false) {
                            self.publish_queue();
                            self.load_track(next, false).await;
                            return;
                        }
                    }
                    self.schedule_prefetch_and_gapless();
                } else if self.pending_autofill_advance {
                    self.pending_autofill_advance = false;
                    self.snapshot.resolving = false;
                    self.publish_snapshot();
                }
            }
            InternalMsg::TrackingLoaded {
                epoch,
                video_id,
                cpn,
                tracking,
            } => {
                if epoch == self.load_gen {
                    self.active_tracking = Some((video_id, cpn, tracking));
                }
            }
            InternalMsg::GaplessResolved {
                epoch,
                video_id,
                url,
                user_agent,
            } => {
                if epoch == self.load_gen
                    && self.settings.player.gapless
                    && self.queue.repeat_mode() != RepeatMode::One
                    && self
                        .queue
                        .peek_next()
                        .is_some_and(|t| t.video_id == video_id)
                    && self.gapless_appended_video_id.as_deref() != Some(video_id.as_str())
                {
                    if user_agent.is_some() {
                        let _ = self.backend.set_user_agent(user_agent.as_deref()).await;
                    }
                    if self.backend.append_gapless(&url).await.is_ok() {
                        self.gapless_appended_video_id = Some(video_id);
                    }
                }
            }
        }
    }

    fn spawn_lyrics_load(&self, epoch: u64, track: Track) {
        let lyr_svc = self.lyrics_svc.clone();
        let tx = self.internal_tx.clone();
        tokio::spawn(async move {
            let lyr = lyr_svc.fetch(&track).await;
            let _ = tx.send(InternalMsg::LyricsLoaded {
                epoch,
                video_id: track.video_id,
                lyrics: lyr,
            });
        });
    }

    fn spawn_telemetry_start(&self, epoch: u64, video_id: String) {
        // Hard privacy gate: strictly off in Anonymous mode or when report_playback_history is false.
        if !self.settings.privacy.report_playback_history || !self.auth.is_authenticated() {
            return;
        }
        let catalog = self.catalog.clone();
        let tx = self.internal_tx.clone();
        let cpn = generate_cpn();
        tokio::spawn(async move {
            if let Ok(Some(tracking)) = catalog.fetch_playback_tracking(&video_id, true).await {
                let _ = catalog
                    .report_playback(&tracking, &cpn, PlaybackTelemetryEvent::Start, true)
                    .await;
                let _ = tx.send(InternalMsg::TrackingLoaded {
                    epoch,
                    video_id,
                    cpn,
                    tracking,
                });
            }
        });
    }

    async fn flush_watchtime_if_needed(&mut self) {
        if self.watchtime_reported
            || !self.settings.privacy.report_playback_history
            || !self.auth.is_authenticated()
        {
            return;
        }
        let Some((_, ref cpn, ref tracking)) = self.active_tracking else {
            return;
        };
        if self.position.position_secs < 5.0 {
            return;
        }
        self.watchtime_reported = true;
        let catalog = self.catalog.clone();
        let tracking = tracking.clone();
        let cpn = cpn.clone();
        let elapsed = self.position.position_secs;
        let duration = self.position.duration_secs;
        let playing = self.snapshot.playing && !self.snapshot.paused;
        tokio::spawn(async move {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                catalog.report_playback(
                    &tracking,
                    &cpn,
                    PlaybackTelemetryEvent::Watchtime {
                        elapsed_secs: elapsed,
                        duration_secs: duration,
                        playing,
                    },
                    true,
                ),
            )
            .await;
        });
    }

    async fn invalidate_gapless_tail(&mut self) {
        if self.gapless_appended_video_id.take().is_some() {
            let _ = self.backend.clear_gapless_tail().await;
        }
    }

    fn schedule_prefetch_and_gapless(&self) {
        if !self.settings.player.prefetch {
            return;
        }
        let Some(next_track) = self.queue.peek_next().cloned() else {
            return;
        };
        if self.settings.player.gapless
            && self.queue.repeat_mode() != RepeatMode::One
            && self.gapless_appended_video_id.as_deref() != Some(next_track.video_id.as_str())
        {
            let resolver = self.resolver.clone();
            let tx = self.internal_tx.clone();
            let vid = next_track.video_id;
            let epoch = self.load_gen;
            tokio::spawn(async move {
                if let Ok(stream) = resolver.resolve(&vid).await {
                    let _ = tx.send(InternalMsg::GaplessResolved {
                        epoch,
                        video_id: vid,
                        url: stream.url,
                        user_agent: stream.user_agent,
                    });
                }
            });
        } else {
            self.resolver.prefetch(next_track.video_id);
        }
    }

    async fn maybe_autofill_radio(&mut self) {
        if !self.snapshot.autoplay_active
            || !self.settings.player.autoplay
            || self.radio_inflight
            || self.queue.upcoming_tracks().len() > RADIO_AUTOFILL_THRESHOLD
        {
            return;
        }
        let Some(seed_vid) = self
            .queue
            .tracks()
            .last()
            .or_else(|| self.queue.current())
            .map(|t| t.video_id.clone())
        else {
            return;
        };

        self.radio_inflight = true;
        let catalog = self.catalog.clone();
        let tx = self.internal_tx.clone();
        let pid = self.radio_playlist_id.clone();
        let cont = self.radio_continuation.clone();

        tokio::spawn(async move {
            let res = catalog
                .radio(Some(&seed_vid), pid.as_deref(), cont.as_deref(), 20)
                .await;
            match res {
                Ok(batch) => {
                    let _ = tx.send(InternalMsg::RadioAutofill {
                        tracks: batch.tracks,
                        continuation: batch.continuation,
                        playlist_id: batch.playlist_id,
                    });
                }
                Err(err) => {
                    debug!("Radio autofill failed: {err}");
                    let _ = tx.send(InternalMsg::RadioAutofill {
                        tracks: Vec::new(),
                        continuation: None,
                        playlist_id: None,
                    });
                }
            }
        });
    }
}
