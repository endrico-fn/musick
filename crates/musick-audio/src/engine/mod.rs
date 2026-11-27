mod commands;
mod playback;
mod radio;
mod telemetry;

use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::{AbortHandle, JoinHandle};

use musick_core::{
    AppCommand, AuthManager, CatalogApi, Lyrics, LyricsService, PlaybackSnapshot, PlaybackTracking,
    PlayerEvent, PositionTick, QueueSnapshot, RadioBatch, Settings, Track, UserStore,
};

use crate::backend::AudioBackend;
use crate::queue::PlayQueue;
use crate::resolver::{HybridStreamResolver, ResolvedStream};

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

pub(super) enum InternalMsg {
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
    pub(super) queue: PlayQueue,
    pub(super) backend: Arc<dyn AudioBackend>,
    pub(super) resolver: Arc<HybridStreamResolver>,
    pub(super) catalog: Arc<dyn CatalogApi>,
    pub(super) lyrics_svc: Arc<LyricsService>,
    pub(super) store: Arc<UserStore>,
    pub(super) auth: Arc<AuthManager>,
    pub(super) settings: Settings,

    pub(super) snapshot: PlaybackSnapshot,
    pub(super) position: PositionTick,
    pub(super) current_lyrics: (Option<String>, Lyrics),

    pub(super) pos_tx: watch::Sender<PositionTick>,
    pub(super) snapshot_tx: watch::Sender<PlaybackSnapshot>,
    pub(super) queue_tx: watch::Sender<QueueSnapshot>,
    pub(super) lyrics_tx: watch::Sender<(Option<String>, Lyrics)>,
    pub(super) events_tx: broadcast::Sender<PlayerEvent>,

    pub(super) internal_tx: mpsc::UnboundedSender<InternalMsg>,
    pub(super) load_gen: u64,
    pub(super) resolve_task: Option<AbortHandle>,
    pub(super) radio_start_task: Option<AbortHandle>,
    pub(super) retried_video_id: Option<String>,
    pub(super) last_stream_from_fast: bool,
    pub(super) gapless_appended_video_id: Option<String>,

    pub(super) radio_playlist_id: Option<String>,
    pub(super) radio_continuation: Option<String>,
    pub(super) radio_inflight: bool,
    pub(super) pending_autofill_advance: bool,

    pub(super) active_tracking: Option<(String, String, PlaybackTracking)>,
    pub(super) watchtime_reported: bool,
    pub(super) pre_mute_volume: Option<u8>,
    pub(super) pending_restore_seek: Option<f64>,
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
            pre_mute_volume: None,
            pending_restore_seek: None,
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

    pub(super) fn publish_snapshot(&mut self) {
        self.snapshot.upcoming_count = self.queue.upcoming_tracks().len();
        self.snapshot.shuffle = self.queue.shuffle_enabled();
        self.snapshot.repeat = self.queue.repeat_mode();
        let _ = self.snapshot_tx.send(self.snapshot.clone());
        let _ = self
            .events_tx
            .send(PlayerEvent::Snapshot(Box::new(self.snapshot.clone())));
    }

    pub(super) fn publish_queue(&mut self) {
        self.snapshot.queue_rev += 1;
        let qsnap = self.queue.to_snapshot();
        let _ = self.queue_tx.send(qsnap.clone());
        let _ = self.events_tx.send(PlayerEvent::Queue(qsnap));
        self.publish_snapshot();
    }

    pub(super) fn publish_lyrics(&mut self, video_id: Option<String>, lyrics: Lyrics) {
        self.snapshot.lyrics_rev += 1;
        self.current_lyrics = (video_id.clone(), lyrics.clone());
        let active_idx = self.current_lyrics.1.active_line_index(
            (self.position.position_secs + self.snapshot.lyrics_offset_secs).max(0.0),
        );
        self.position.active_lyric_idx = active_idx;
        let _ = self.pos_tx.send(self.position);
        let _ = self.lyrics_tx.send((video_id.clone(), lyrics.clone()));
        let _ = self.events_tx.send(PlayerEvent::Lyrics {
            track_video_id: video_id,
            lyrics,
        });
        self.publish_snapshot();
    }

    pub(super) fn cancel_inflight_loads(&mut self) {
        if let Some(h) = self.resolve_task.take() {
            h.abort();
        }
        if let Some(h) = self.radio_start_task.take() {
            h.abort();
        }
    }
}
