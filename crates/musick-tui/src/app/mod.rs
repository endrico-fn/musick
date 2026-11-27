mod actions;
mod bg;
mod collections;
mod effects;
mod modals;
mod mouse;
mod render;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use image::DynamicImage;
use ratatui::layout::Rect;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::AbortHandle;

use musick_audio::{CavaControl, CavaFrame, CavaRunnerHandle, MprisService, PlayerEngineHandle};
use musick_core::{
    Album, AppCommand, AppPaths, Artist, AuthManager, CacheManager, CacheStats, CatalogApi,
    DiscoverFeed, ImageService, Lyrics, OAuthPollStatus, PlaybackSnapshot, PlayerEvent, Playlist,
    PositionTick, QueueSnapshot, SearchItem, SearchService, SessionState, Settings, Suggestion,
    SyncEngine, SyncReport, Track, UserStore,
};

use crate::keymap::{InputMode, KeymapEngine};
use crate::theme::set_active_accent;
use crate::views::{
    AlbumView, ArtistView, LibraryView, PlayerView, PlaylistDetailView, PlaylistsView, QueueView,
    SettingsView,
};
use crate::widgets::{
    ActiveModal, BrailleSpinner, CoverRenderer, PlayerBarControl, PlayerBarHitboxes, RootTab,
};

const MAX_NAV_DEPTH: usize = 12;
const SESSION_KV_KEY: &str = "last_session_v1";

pub enum NavPage {
    Artist(Box<ArtistView>),
    Album(AlbumView),
    PlaylistDetail(PlaylistDetailView),
}

pub enum BgMsg {
    DiscoverLoaded {
        gen_id: u64,
        result: Result<DiscoverFeed, String>,
    },
    SearchLoaded {
        gen_id: u64,
        result: Result<Vec<SearchItem>, String>,
    },
    SuggestionsLoaded {
        gen_id: u64,
        suggestions: Vec<Suggestion>,
    },
    ArtistLoaded {
        browse_id: String,
        result: Result<Box<Artist>, String>,
    },
    AlbumLoaded {
        browse_id: String,
        result: Result<Box<Album>, String>,
    },
    PlaylistLoaded {
        browse_id: String,
        result: Result<Box<Playlist>, String>,
    },
    ImageLoaded {
        url: String,
        image: Arc<DynamicImage>,
    },
    ImageFailed {
        url: String,
    },
    SyncFinished(Result<SyncReport, String>),
    PlaylistUploaded(Result<String, String>),
    PlaylistImported(Result<(String, String, usize), String>),
    OAuthCodeStarted(Result<(String, String, String, u64), String>),
    OAuthPollCompleted(Result<OAuthPollStatus, String>),
    Toast(String),
}

pub struct AppServices {
    pub paths: AppPaths,
    pub store: Arc<UserStore>,
    pub cache: Arc<CacheManager>,
    pub auth: Arc<AuthManager>,
    pub catalog: Arc<dyn CatalogApi>,
    pub search: Arc<SearchService>,
    pub images: Arc<ImageService>,
    pub sync: Arc<SyncEngine>,
    pub engine: PlayerEngineHandle,
    pub cava: Option<CavaRunnerHandle>,
}

pub struct App {
    pub services: AppServices,
    pub settings: Settings,
    pub active_tab: RootTab,
    pub nav_stack: Vec<NavPage>,
    pub modal: Option<ActiveModal>,
    pub keymap: KeymapEngine,
    pub spinner: BrailleSpinner,
    pub covers: CoverRenderer,

    pub player: PlayerView,
    pub playlists: PlaylistsView,
    pub library: LibraryView,
    pub queue: QueueView,
    pub settings_view: SettingsView,

    pub snapshot: PlaybackSnapshot,
    pub position: PositionTick,
    pub queue_snap: QueueSnapshot,
    pub lyrics: Lyrics,
    pub cava_frame: CavaFrame,
    pub cache_stats: CacheStats,

    pub syncing: bool,
    pub pending_sync: usize,
    pub should_quit: bool,
    pub status_toast: Option<(String, Instant)>,
    pub player_bar_focused: bool,
    pub player_bar_control: PlayerBarControl,

    pub(super) player_bar_hitboxes: PlayerBarHitboxes,
    pub(super) topbar_tab_hitboxes: Vec<(RootTab, Rect)>,
    pub(super) dragging_seekbar: bool,
    pub(super) dragging_volume: bool,

    bg_tx: mpsc::UnboundedSender<BgMsg>,
    bg_rx: mpsc::UnboundedReceiver<BgMsg>,
    engine_events_rx: broadcast::Receiver<PlayerEvent>,
    requested_images: HashSet<String>,
    last_selected_playlist_id: Option<String>,
    settings_dirty_at: Option<Instant>,
    session_dirty_at: Option<Instant>,
    last_cache_stats_at: Instant,
    sync_interval_tx: Option<watch::Sender<u64>>,
    discover_gen: u64,
    discover_task: Option<AbortHandle>,
    search_gen: u64,
    search_task: Option<AbortHandle>,
    autocomplete_gen: u64,
    autocomplete_task: Option<AbortHandle>,
    oauth_task: Option<AbortHandle>,
    mpris: Option<MprisService>,
    mpris_quit_flag: Option<Arc<AtomicBool>>,
}

impl App {
    pub fn new(services: AppServices, settings: Settings, covers: CoverRenderer) -> Self {
        set_active_accent(settings.display.accent);
        let (bg_tx, bg_rx) = mpsc::unbounded_channel();
        let reduced_motion = settings.display.reduced_motion;
        let snapshot = services.engine.snapshot();
        let position = services.engine.position();
        let queue_snap = services.engine.queue();
        let (_, lyrics) = services.engine.lyrics();
        let engine_events_rx = services.engine.subscribe_events();
        let cache_stats = services.cache.stats().unwrap_or_default();
        let pending_sync = services.sync.pending_count();
        let secret_backend = services.auth.secret_backend_label();
        let mut settings_view = SettingsView::new();
        settings_view.secret_backend = secret_backend;

        let mut app = Self {
            services,
            settings,
            active_tab: RootTab::Player,
            nav_stack: Vec::new(),
            modal: None,
            keymap: KeymapEngine::new(),
            spinner: BrailleSpinner::new(reduced_motion),
            covers,
            player: PlayerView::new(),
            playlists: PlaylistsView::new(),
            library: LibraryView::new(),
            queue: QueueView::new(),
            settings_view,
            snapshot,
            position,
            queue_snap,
            lyrics,
            cava_frame: CavaFrame::default(),
            cache_stats,
            syncing: false,
            pending_sync,
            should_quit: false,
            status_toast: None,
            player_bar_focused: false,
            player_bar_control: PlayerBarControl::Seekbar,
            player_bar_hitboxes: PlayerBarHitboxes::default(),
            topbar_tab_hitboxes: Vec::new(),
            dragging_seekbar: false,
            dragging_volume: false,
            bg_tx,
            bg_rx,
            engine_events_rx,
            requested_images: HashSet::new(),
            last_selected_playlist_id: None,
            settings_dirty_at: None,
            session_dirty_at: None,
            last_cache_stats_at: Instant::now(),
            sync_interval_tx: None,
            discover_gen: 0,
            discover_task: None,
            search_gen: 0,
            search_task: None,
            autocomplete_gen: 0,
            autocomplete_task: None,
            oauth_task: None,
            mpris: None,
            mpris_quit_flag: None,
        };
        app.reload_local_collections();
        app
    }

    pub fn attach_mpris(&mut self, mpris: Option<MprisService>, quit_flag: Arc<AtomicBool>) {
        self.mpris = mpris;
        self.mpris_quit_flag = Some(quit_flag);
    }

    pub fn attach_sync_interval_tx(&mut self, tx: watch::Sender<u64>) {
        self.sync_interval_tx = Some(tx);
    }

    pub fn shutdown_mpris(&mut self) {
        if let Some(m) = self.mpris.take() {
            m.shutdown();
        }
    }

    pub async fn recv_bg(&mut self) -> Option<BgMsg> {
        self.bg_rx.recv().await
    }

    fn cancel_oauth_task(&mut self) {
        if let Some(handle) = self.oauth_task.take() {
            handle.abort();
        }
    }

    pub fn kick_startup_loads(&mut self) {
        self.trigger_discover_refresh(false);
        if self.settings.privacy.save_history && self.snapshot.track.is_none() {
            if let Ok(Some(raw)) = self.services.store.get_kv(SESSION_KV_KEY)
                && let Ok(sess) = serde_json::from_str::<SessionState>(&raw)
                && !sess.tracks.is_empty()
            {
                let idx = sess.index.min(sess.tracks.len() - 1);
                if let Some(url) = sess.tracks[idx].cover_url() {
                    self.request_cover_image(url);
                }
                self.services.engine.try_send(AppCommand::RestoreSession {
                    tracks: sess.tracks,
                    index: idx,
                    position_secs: sess.position_secs,
                    shuffle: sess.shuffle,
                    repeat: sess.repeat,
                });
            } else if let Ok(mut recent) = self.services.store.recent_plays(1)
                && let Some(last) = recent.pop()
            {
                if let Some(url) = last.cover_url() {
                    self.request_cover_image(url);
                }
                self.services
                    .engine
                    .try_send(AppCommand::RestoreTrack { track: last });
            }
        }
        if self.services.auth.is_authenticated() && self.settings.sync.sync_on_startup {
            self.trigger_sync_now();
        }
    }

    pub fn bg_sender(&self) -> mpsc::UnboundedSender<BgMsg> {
        self.bg_tx.clone()
    }

    pub fn is_animating(&self) -> bool {
        if self.covers.has_pending_renders() {
            return true;
        }
        if self.settings.display.reduced_motion {
            return false;
        }
        let cava_active = self.active_tab == RootTab::Player
            && self.nav_stack.is_empty()
            && self.snapshot.playing
            && !self.snapshot.paused
            && self.settings.display.cava_enabled;
        let busy = self.player.loading_discover
            || self.player.loading_search
            || self.snapshot.resolving
            || self.position.buffering
            || self.snapshot.loading_lyrics
            || self.snapshot.translating_lyrics
            || self.syncing
            || self.nav_stack.last().is_some_and(|p| match p {
                NavPage::Artist(v) => v.loading,
                NavPage::Album(v) => v.loading,
                NavPage::PlaylistDetail(v) => v.loading,
            })
            || matches!(
                self.modal,
                Some(ActiveModal::OAuthWizard { polling: true, .. })
            );
        cava_active || busy
    }

    pub fn input_mode(&self) -> InputMode {
        if self.modal.is_some() {
            return InputMode::Modal;
        }
        if let Some(top) = self.nav_stack.last() {
            return match top {
                NavPage::Artist(v) if v.is_insert_mode() => InputMode::Insert,
                NavPage::Album(v) if v.is_insert_mode() => InputMode::Insert,
                NavPage::PlaylistDetail(v) if v.is_insert_mode() => InputMode::Insert,
                _ => InputMode::Normal,
            };
        }
        match self.active_tab {
            RootTab::Player if self.player.is_insert_mode() => InputMode::Insert,
            RootTab::Playlists if self.playlists.is_insert_mode() => InputMode::Insert,
            RootTab::Library if self.library.is_insert_mode() => InputMode::Insert,
            RootTab::Settings if self.settings_view.is_insert_mode() => InputMode::Insert,
            _ => InputMode::Normal,
        }
    }

    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.status_toast = Some((msg.into(), Instant::now() + Duration::from_secs(4)));
    }

    fn persist_session_now(&self) {
        if !self.settings.privacy.save_history || self.queue_snap.tracks.is_empty() {
            return;
        }
        let idx = self
            .queue_snap
            .index
            .unwrap_or(0)
            .min(self.queue_snap.tracks.len().saturating_sub(1));
        let sess = SessionState {
            tracks: self.queue_snap.tracks.clone(),
            index: idx,
            position_secs: self.position.position_secs.max(0.0),
            shuffle: self.snapshot.shuffle,
            repeat: self.snapshot.repeat,
        };
        if let Ok(json) = serde_json::to_string(&sess) {
            let _ = self.services.store.set_kv(SESSION_KV_KEY, &json);
        }
    }

    pub fn flush_pending_saves(&mut self) {
        if self.settings_dirty_at.take().is_some() {
            let _ = self.settings.save(&self.services.paths.settings_file());
        }
        if self.settings.privacy.save_history && !self.queue_snap.tracks.is_empty() {
            self.session_dirty_at = None;
            self.persist_session_now();
        }
    }

    pub fn on_tick(&mut self) {
        self.spinner.reduced_motion = self.settings.display.reduced_motion;
        if self.is_animating() {
            self.spinner.advance();
        }
        if let Some((_, exp)) = &self.status_toast
            && Instant::now() >= *exp
        {
            self.status_toast = None;
        }
        if let Some(dirty_at) = self.settings_dirty_at
            && dirty_at.elapsed() >= Duration::from_millis(400)
        {
            self.settings_dirty_at = None;
            let _ = self.settings.save(&self.services.paths.settings_file());
        }
        if let Some(dirty_at) = self.session_dirty_at
            && dirty_at.elapsed() >= Duration::from_secs(2)
        {
            self.session_dirty_at = None;
            self.persist_session_now();
        }
        self.poll_engine_and_bg();
    }

    pub fn poll_engine_and_bg(&mut self) {
        let new_snap = self.services.engine.snapshot();
        if new_snap != self.snapshot {
            let track_changed = new_snap.track_rev != self.snapshot.track_rev;
            let mut settings_dirty = false;
            if !new_snap.muted && new_snap.volume != self.settings.player.volume {
                self.settings.player.volume = new_snap.volume;
                settings_dirty = true;
            }
            if new_snap.show_translation != self.settings.lyrics.translate_to_id {
                self.settings.lyrics.translate_to_id = new_snap.show_translation;
                settings_dirty = true;
            }
            self.snapshot = new_snap;
            if settings_dirty {
                self.settings_dirty_at = Some(Instant::now());
            }
            if track_changed {
                self.player.lyrics_scroll = 0;
                if let Some(url) = self.snapshot.track.as_ref().and_then(Track::cover_url) {
                    self.request_cover_image(url);
                }
                self.reload_recent_plays();
                if self.settings.privacy.save_history {
                    self.session_dirty_at = Some(Instant::now());
                }
            }
            self.sync_cava_state();
        }

        loop {
            match self.engine_events_rx.try_recv() {
                Ok(PlayerEvent::StatusToast(msg)) => self.set_toast(msg),
                Ok(_) => {}
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(
                    broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed,
                ) => break,
            }
        }

        self.position = self.services.engine.position();
        let new_q = self.services.engine.queue();
        if new_q != self.queue_snap {
            self.queue_snap = new_q;
            if self.settings.privacy.save_history && !self.queue_snap.tracks.is_empty() {
                self.session_dirty_at = Some(Instant::now());
            }
        }
        let (_, new_lyr) = self.services.engine.lyrics();
        if new_lyr != self.lyrics {
            self.lyrics = new_lyr;
        }
        if let Some(ref cava) = self.services.cava {
            self.cava_frame = cava.frame();
        }

        while let Ok(msg) = self.bg_rx.try_recv() {
            self.handle_bg_msg(msg);
        }

        self.sync_selected_playlist_tracks(false);
        self.ensure_visible_covers();
    }

    fn sync_cava_state(&self) {
        if let Some(ref cava) = self.services.cava {
            let visible = self.active_tab == RootTab::Player && self.nav_stack.is_empty();
            cava.update(CavaControl {
                enabled: self.settings.display.cava_enabled,
                playing: self.snapshot.playing,
                paused: self.snapshot.paused,
                visible,
                reduced_motion: self.settings.display.reduced_motion,
                bar_count: 32,
                smoothing: self.settings.display.cava_smoothing,
                audio_source: self.settings.display.cava_audio_source,
            });
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.flush_pending_saves();
        if let Some(h) = self.discover_task.take() {
            h.abort();
        }
        if let Some(h) = self.search_task.take() {
            h.abort();
        }
        if let Some(h) = self.autocomplete_task.take() {
            h.abort();
        }
        self.cancel_oauth_task();
        self.shutdown_mpris();
    }
}
