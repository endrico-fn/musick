use std::collections::HashSet;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use base64::Engine as _;
use crossterm::event::KeyEvent;
use image::DynamicImage;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use musick_audio::{CavaControl, CavaFrame, CavaRunnerHandle, MprisService, PlayerEngineHandle};
use musick_core::{
    Album, AppCommand, AppPaths, Artist, AuthManager, AuthMode, CacheManager, CacheStats,
    CatalogApi, HomeFeed, ImageService, Lyrics, OAuthPollStatus, PlaybackSnapshot, Playlist,
    PositionTick, QueueSnapshot, SavedCollectionKind, SearchItem, SearchService, Settings,
    Suggestion, SyncEngine, SyncReport, Track, UserStore, YoutubeTarget, extract_youtube_target,
};

use crate::keymap::{Action, InputMode, KeymapEngine};
use crate::text::format_bytes;
use crate::theme::{ACCENT, BORDER, DIM, TEXT, icons_for};
use crate::views::{
    AlbumView, ArtistView, HomeView, LibraryView, PlaylistDetailView, PlaylistsView, QueueView,
    RenderCtx, SettingsView, ViewEffect,
};
use crate::widgets::{
    ActiveModal, BrailleSpinner, ConfirmAction, CoverRenderer, NowPlayingBar, TopBar, render_modal,
};

const MAX_NAV_DEPTH: usize = 12;

pub enum NavPage {
    Artist(Box<ArtistView>),
    Album(AlbumView),
    PlaylistDetail(PlaylistDetailView),
}

pub enum BgMsg {
    HomeLoaded(Result<HomeFeed, String>),
    SearchLoaded(Result<Vec<SearchItem>, String>),
    SuggestionsLoaded(Vec<Suggestion>),
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
    pub active_tab: usize,
    pub nav_stack: Vec<NavPage>,
    pub modal: Option<ActiveModal>,
    pub keymap: KeymapEngine,
    pub spinner: BrailleSpinner,
    pub covers: CoverRenderer,

    pub home: HomeView,
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
    pub should_quit: bool,
    pub status_toast: Option<(String, Instant)>,

    bg_tx: mpsc::UnboundedSender<BgMsg>,
    bg_rx: mpsc::UnboundedReceiver<BgMsg>,
    requested_images: HashSet<String>,
    last_cover_url: Option<String>,
    last_selected_playlist_id: Option<String>,
    oauth_task: Option<AbortHandle>,
    mpris: Option<MprisService>,
    mpris_quit_flag: Option<Arc<AtomicBool>>,
}

impl App {
    pub fn new(services: AppServices, settings: Settings, covers: CoverRenderer) -> Self {
        let (bg_tx, bg_rx) = mpsc::unbounded_channel();
        let cover_size = settings.display.cover_size;
        let reduced_motion = settings.display.reduced_motion;
        let snapshot = services.engine.snapshot();
        let position = services.engine.position();
        let queue_snap = services.engine.queue();
        let (_, lyrics) = services.engine.lyrics();
        let cache_stats = services.cache.stats().unwrap_or_default();

        let mut app = Self {
            services,
            settings,
            active_tab: 0,
            nav_stack: Vec::new(),
            modal: None,
            keymap: KeymapEngine::new(),
            spinner: BrailleSpinner::new(reduced_motion),
            covers,
            home: HomeView::new(),
            playlists: PlaylistsView::new(),
            library: LibraryView::new(cover_size),
            queue: QueueView::new(),
            settings_view: SettingsView::new(),
            snapshot,
            position,
            queue_snap,
            lyrics,
            cava_frame: CavaFrame::default(),
            cache_stats,
            syncing: false,
            should_quit: false,
            status_toast: None,
            bg_tx,
            bg_rx,
            requested_images: HashSet::new(),
            last_cover_url: None,
            last_selected_playlist_id: None,
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
        self.trigger_home_refresh();
        if self.services.auth.is_authenticated() && self.settings.sync.sync_on_startup {
            self.trigger_sync_now();
        }
    }

    pub fn bg_sender(&self) -> mpsc::UnboundedSender<BgMsg> {
        self.bg_tx.clone()
    }

    pub fn is_animating(&self) -> bool {
        if self.settings.display.reduced_motion {
            return false;
        }
        let cava_active = self.active_tab == 0
            && self.nav_stack.is_empty()
            && self.snapshot.playing
            && !self.snapshot.paused
            && self.settings.display.cava_enabled;
        let busy = self.home.loading_home
            || self.home.loading_search
            || self.snapshot.resolving
            || self.position.buffering
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
                NavPage::PlaylistDetail(v) if v.is_insert_mode() => InputMode::Insert,
                _ => InputMode::Normal,
            };
        }
        match self.active_tab {
            0 if self.home.is_insert_mode() => InputMode::Insert,
            1 if self.playlists.is_insert_mode() => InputMode::Insert,
            2 if self.library.is_insert_mode() => InputMode::Insert,
            _ => InputMode::Normal,
        }
    }

    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.status_toast = Some((msg.into(), Instant::now() + Duration::from_secs(4)));
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
        self.poll_engine_and_bg();
    }

    pub fn poll_engine_and_bg(&mut self) {
        let new_snap = self.services.engine.snapshot();
        if new_snap != self.snapshot {
            let track_changed = new_snap.track_rev != self.snapshot.track_rev;
            let mut settings_dirty = false;
            if new_snap.volume != self.settings.player.volume {
                self.settings.player.volume = new_snap.volume;
                settings_dirty = true;
            }
            if new_snap.show_translation != self.settings.lyrics.translate_to_id {
                self.settings.lyrics.translate_to_id = new_snap.show_translation;
                settings_dirty = true;
            }
            self.snapshot = new_snap;
            if settings_dirty {
                let _ = self.settings.save(&self.services.paths.settings_file());
            }
            if track_changed {
                if let Some(url) = self.snapshot.track.as_ref().and_then(Track::cover_url) {
                    self.request_cover_image(url);
                }
                self.reload_recent_plays();
            }
            self.sync_cava_state();
        }

        self.position = self.services.engine.position();
        let new_q = self.services.engine.queue();
        if new_q != self.queue_snap {
            self.queue_snap = new_q;
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

        // Keep right pane of PlaylistsView synced with selected playlist
        self.sync_selected_playlist_tracks(false);
    }

    fn sync_cava_state(&self) {
        if let Some(ref cava) = self.services.cava {
            let visible = self.active_tab == 0 && self.nav_stack.is_empty();
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

    pub fn handle_bg_msg(&mut self, msg: BgMsg) {
        match msg {
            BgMsg::HomeLoaded(Ok(feed)) => {
                self.home.set_home_feed(feed);
                self.refresh_cache_stats();
            }
            BgMsg::HomeLoaded(Err(err)) => {
                self.home.loading_home = false;
                self.set_toast(format!("Home load error: {err}"));
            }
            BgMsg::SearchLoaded(Ok(items)) => {
                self.home.set_search_results(items);
                self.refresh_cache_stats();
            }
            BgMsg::SearchLoaded(Err(err)) => {
                self.home.loading_search = false;
                self.set_toast(format!("Search error: {err}"));
            }
            BgMsg::SuggestionsLoaded(sugs) => {
                self.home.set_suggestions(sugs);
            }
            BgMsg::ArtistLoaded { browse_id, result } => {
                let mut cover_urls = Vec::new();
                let mut err_toast = None;
                if let Some(NavPage::Artist(view)) = self.nav_stack.last_mut()
                    && view.browse_id == browse_id
                {
                    match result {
                        Ok(art) => {
                            if let Some(url) = art.summary.cover_url() {
                                cover_urls.push(url);
                            }
                            for al in art.albums.iter().chain(art.singles.iter()).take(48) {
                                if let Some(u) = al.cover_url() {
                                    cover_urls.push(u);
                                }
                            }
                            view.set_artist(*art);
                        }
                        Err(err) => {
                            view.loading = false;
                            err_toast = Some(format!("Artist load failed: {err}"));
                        }
                    }
                }
                for url in cover_urls {
                    self.request_cover_image(url);
                }
                if let Some(msg) = err_toast {
                    self.set_toast(msg);
                }
            }
            BgMsg::AlbumLoaded { browse_id, result } => {
                let mut cover_url = None;
                let mut err_toast = None;
                if let Some(NavPage::Album(view)) = self.nav_stack.last_mut()
                    && view.browse_id == browse_id
                {
                    match result {
                        Ok(alb) => {
                            cover_url = alb.summary.cover_url();
                            view.set_album(*alb);
                        }
                        Err(err) => {
                            view.loading = false;
                            err_toast = Some(format!("Album load failed: {err}"));
                        }
                    }
                }
                if let Some(url) = cover_url {
                    self.request_cover_image(url);
                }
                if let Some(msg) = err_toast {
                    self.set_toast(msg);
                }
            }
            BgMsg::PlaylistLoaded { browse_id, result } => {
                let mut cover_url = None;
                let mut err_toast = None;
                if let Some(NavPage::PlaylistDetail(view)) = self.nav_stack.last_mut()
                    && view.browse_id == browse_id
                {
                    match result {
                        Ok(pl) => {
                            cover_url = pl.summary.cover_url();
                            view.set_playlist(*pl);
                        }
                        Err(err) => {
                            view.loading = false;
                            err_toast = Some(format!("Playlist load failed: {err}"));
                        }
                    }
                }
                if let Some(url) = cover_url {
                    self.request_cover_image(url);
                }
                if let Some(msg) = err_toast {
                    self.set_toast(msg);
                }
            }
            BgMsg::ImageLoaded { url, image } => {
                self.covers.insert_image(url, image);
                self.refresh_cache_stats();
            }
            BgMsg::ImageFailed { url } => {
                self.requested_images.remove(&url);
            }
            BgMsg::SyncFinished(res) => {
                self.syncing = false;
                match res {
                    Ok(rep) if rep.skipped_anonymous => {
                        self.set_toast("Anonymous mode — sign in to sync with YouTube Music");
                    }
                    Ok(rep) => {
                        self.reload_local_collections();
                        self.set_toast(format!(
                            "Synced: {}↑ {} playlists · {} likes",
                            rep.pushed, rep.pulled_playlists, rep.pulled_likes
                        ));
                    }
                    Err(err) => self.set_toast(format!("Sync error: {err}")),
                }
            }
            BgMsg::PlaylistUploaded(res) => match res {
                Ok(title) => {
                    self.reload_local_collections();
                    self.set_toast(format!("Uploaded \"{title}\" to YouTube Music"));
                }
                Err(err) => self.set_toast(format!("Upload failed: {err}")),
            },
            BgMsg::PlaylistImported(res) => match res {
                Ok((id, title, count)) => {
                    self.reload_local_collections();
                    self.select_user_playlist(&id);
                    self.set_toast(format!("Imported \"{title}\" ({count} tracks)"));
                }
                Err(err) => self.set_toast(format!("Import failed: {err}")),
            },
            BgMsg::OAuthCodeStarted(res) => {
                // Ignore stale result if the user already closed OAuthWizard
                if !matches!(self.modal, Some(ActiveModal::OAuthWizard { .. })) {
                    return;
                }
                match res {
                    Ok((device_code, user_code, verification_url, interval)) => {
                        if let Some(ActiveModal::OAuthWizard {
                            client_id,
                            client_secret,
                            page_id,
                            device_user_code,
                            verification_url: v_url,
                            polling,
                            error,
                            ..
                        }) = self.modal.as_mut()
                        {
                            *device_user_code = Some(user_code);
                            *v_url = Some(verification_url);
                            *polling = true;
                            *error = None;
                            let cid = client_id.clone();
                            let csec = client_secret.clone();
                            let pid = if page_id.trim().is_empty() {
                                None
                            } else {
                                Some(page_id.trim().to_string())
                            };
                            self.spawn_oauth_poller(cid, csec, device_code, pid, interval);
                        }
                    }
                    Err(err) => {
                        if let Some(ActiveModal::OAuthWizard { error, polling, .. }) =
                            self.modal.as_mut()
                        {
                            *polling = false;
                            *error = Some(err);
                        }
                    }
                }
            }
            BgMsg::OAuthPollCompleted(res) => {
                self.oauth_task = None;
                // Ignore late authorization if the user already cancelled OAuthWizard
                if !matches!(self.modal, Some(ActiveModal::OAuthWizard { .. })) {
                    return;
                }
                match res {
                    Ok(OAuthPollStatus::Authorized(_)) => {
                        self.modal = None;
                        self.settings.privacy.auth_mode = AuthMode::OAuth;
                        let _ = self.settings.save(&self.services.paths.settings_file());
                        self.services.engine.try_send(AppCommand::ApplySettings {
                            settings: self.settings.clone(),
                        });
                        self.set_toast("Signed in via OAuth2 — syncing library…");
                        self.trigger_sync_now();
                        self.trigger_home_refresh();
                    }
                    Ok(_) => {}
                    Err(err) => {
                        if let Some(ActiveModal::OAuthWizard { error, polling, .. }) =
                            self.modal.as_mut()
                        {
                            *polling = false;
                            *error = Some(err);
                        }
                    }
                }
            }
            BgMsg::Toast(msg) => self.set_toast(msg),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        let mode = self.input_mode();
        let Some(action) = self.keymap.handle_key(mode, key) else {
            return;
        };
        self.dispatch_action(action);
    }

    pub fn handle_paste(&mut self, pasted: &str) {
        if self.input_mode() == InputMode::Normal {
            return;
        }
        let allow_newlines = matches!(
            self.modal,
            Some(ActiveModal::BrowserAuthImport {
                active_field: 0,
                ..
            })
        );
        for ch in pasted.chars() {
            if ch == '\r' || (ch == '\n' && !allow_newlines) {
                continue;
            }
            self.dispatch_action(Action::InputChar(ch));
        }
    }

    fn switch_root_tab(&mut self, idx: usize) {
        if idx < 5 {
            self.active_tab = idx;
            self.nav_stack.clear();
            if idx == 1 || idx == 2 {
                self.reload_local_collections();
            }
            if idx == 4 {
                self.refresh_cache_stats();
            }
            self.sync_cava_state();
        }
    }

    pub fn dispatch_action(&mut self, action: Action) {
        if self.modal.is_some() {
            self.handle_modal_action(action);
            return;
        }

        // Global Normal-mode actions (never fire in Insert/Modal because KeymapEngine isolates them!)
        match action {
            Action::Quit => {
                self.should_quit = true;
                return;
            }
            Action::ToggleHelp => {
                self.modal = Some(ActiveModal::KeymapHelp { scroll: 0 });
                return;
            }
            Action::SwitchTab(idx) => {
                self.switch_root_tab(idx);
                return;
            }
            Action::NextTab => {
                self.switch_root_tab((self.active_tab + 1) % 5);
                return;
            }
            Action::PrevTab => {
                self.switch_root_tab((self.active_tab + 4) % 5);
                return;
            }
            Action::ToggleLyrics => {
                self.active_tab = 0;
                self.nav_stack.clear();
                self.home.toggle_expanded_lyrics();
                self.sync_cava_state();
                return;
            }
            Action::SyncNow => {
                self.trigger_sync_now();
                return;
            }
            Action::ToggleTranslation => {
                self.services
                    .engine
                    .try_send(AppCommand::ToggleLyricsTranslation);
                return;
            }
            Action::TogglePause => {
                self.services.engine.try_send(AppCommand::TogglePause);
                return;
            }
            Action::NextTrack => {
                self.services.engine.try_send(AppCommand::Next);
                return;
            }
            Action::PrevTrack => {
                self.services.engine.try_send(AppCommand::Previous);
                return;
            }
            Action::SeekForward => {
                self.services
                    .engine
                    .try_send(AppCommand::SeekRelative { seconds: 5.0 });
                return;
            }
            Action::SeekBackward => {
                self.services
                    .engine
                    .try_send(AppCommand::SeekRelative { seconds: -5.0 });
                return;
            }
            Action::VolumeUp => {
                self.services
                    .engine
                    .try_send(AppCommand::AdjustVolume { delta: 5 });
                return;
            }
            Action::VolumeDown => {
                self.services
                    .engine
                    .try_send(AppCommand::AdjustVolume { delta: -5 });
                return;
            }
            Action::ToggleShuffle => {
                self.services.engine.try_send(AppCommand::ToggleShuffle);
                return;
            }
            Action::CycleRepeat => {
                self.services.engine.try_send(AppCommand::CycleRepeat);
                return;
            }
            Action::GoBack => {
                if !self.nav_stack.is_empty() {
                    self.nav_stack.pop();
                    self.sync_cava_state();
                }
                return;
            }
            Action::Escape if !self.nav_stack.is_empty() => {
                if let Some(NavPage::PlaylistDetail(v)) = self.nav_stack.last_mut()
                    && v.has_filter()
                {
                    let eff = v.handle_action(Action::Escape);
                    self.apply_effect(eff);
                    return;
                }
                self.nav_stack.pop();
                self.sync_cava_state();
                return;
            }
            Action::StartSearch
                if (!self.nav_stack.is_empty()
                    && !matches!(self.nav_stack.last(), Some(NavPage::PlaylistDetail(_))))
                    || (self.nav_stack.is_empty() && matches!(self.active_tab, 3 | 4)) =>
            {
                // Global `/` jumps to Home inline search from views without their own filter
                self.active_tab = 0;
                self.nav_stack.clear();
                let eff = self.home.handle_action(Action::StartSearch);
                self.apply_effect(eff);
                return;
            }
            _ => {}
        }

        // Delegate to top detail view or active root tab
        let effect = if let Some(top) = self.nav_stack.last_mut() {
            match top {
                NavPage::Artist(v) => v.handle_action(action),
                NavPage::Album(v) => v.handle_action(action),
                NavPage::PlaylistDetail(v) => v.handle_action(action),
            }
        } else {
            match self.active_tab {
                0 => self.home.handle_action(action),
                1 => {
                    self.sync_selected_playlist_tracks(false);
                    let eff = self.playlists.handle_action(action);
                    self.sync_selected_playlist_tracks(false);
                    eff
                }
                2 => self.library.handle_action(action),
                3 => self.queue.handle_action(action, &self.queue_snap),
                4 => self.settings_view.handle_action(
                    action,
                    &self.settings,
                    self.services.auth.mode(),
                ),
                _ => ViewEffect::None,
            }
        };

        self.apply_effect(effect);
    }

    fn handle_modal_action(&mut self, action: Action) {
        let Some(mut modal) = self.modal.take() else {
            return;
        };
        let mut keep = true;

        match &mut modal {
            ActiveModal::CreatePlaylist { input } => match action {
                Action::InputChar(c) => input.push(c),
                Action::InputBackspace => {
                    input.pop();
                }
                Action::InputCancel => keep = false,
                Action::InputSubmit => {
                    let name = input.trim().to_string();
                    keep = false;
                    if !name.is_empty() {
                        match self.services.sync.create_playlist(&name, "", true) {
                            Ok(pl) => {
                                self.reload_local_collections();
                                self.select_user_playlist(&pl.id);
                                self.set_toast(format!("Created playlist \"{}\"", pl.title));
                            }
                            Err(err) => self.set_toast(format!("Create failed: {err}")),
                        }
                    }
                }
                _ => {}
            },
            ActiveModal::RenamePlaylist { playlist_id, input } => match action {
                Action::InputChar(c) => input.push(c),
                Action::InputBackspace => {
                    input.pop();
                }
                Action::InputCancel => keep = false,
                Action::InputSubmit => {
                    let name = input.trim().to_string();
                    let pid = playlist_id.clone();
                    keep = false;
                    if !name.is_empty() {
                        match self.services.sync.rename_playlist(&pid, &name, None) {
                            Ok(_) => {
                                self.reload_local_collections();
                                self.set_toast(format!("Renamed to \"{name}\""));
                            }
                            Err(err) => self.set_toast(format!("Rename failed: {err}")),
                        }
                    }
                }
                _ => {}
            },
            ActiveModal::ImportPlaylistUrl { input } => match action {
                Action::InputChar(c) => input.push(c),
                Action::InputBackspace => {
                    input.pop();
                }
                Action::InputCancel => keep = false,
                Action::InputSubmit => {
                    let raw = input.trim().to_string();
                    keep = false;
                    if !raw.is_empty() {
                        self.import_playlist_input(raw);
                    }
                }
                _ => {}
            },
            ActiveModal::AddToPlaylist {
                track,
                playlists,
                cursor,
            } => match action {
                Action::InputCancel => keep = false,
                Action::InputDown | Action::InputChar('j') => {
                    if !playlists.is_empty() {
                        *cursor = (*cursor + 1).min(playlists.len() - 1);
                    }
                }
                Action::InputUp | Action::InputChar('k') => {
                    *cursor = cursor.saturating_sub(1);
                }
                Action::InputSubmit => {
                    if let Some(pl) = playlists.get(*cursor) {
                        match self
                            .services
                            .sync
                            .add_tracks_to_playlist(&pl.id, std::slice::from_ref(track))
                        {
                            Ok(_) => {
                                let title = pl.title.clone();
                                self.reload_local_collections();
                                self.set_toast(format!("Added to \"{title}\""));
                            }
                            Err(err) => self.set_toast(format!("Add failed: {err}")),
                        }
                    }
                    keep = false;
                }
                _ => {}
            },
            ActiveModal::Confirm { action: conf, .. } => match action {
                Action::InputCancel | Action::InputChar('n') | Action::InputChar('N') => {
                    keep = false;
                }
                Action::InputSubmit | Action::InputChar('y') | Action::InputChar('Y') => {
                    let c = conf.clone();
                    keep = false;
                    match c {
                        ConfirmAction::DeletePlaylist { playlist_id } => {
                            if let Ok(true) = self.services.sync.delete_playlist(&playlist_id) {
                                self.reload_local_collections();
                                self.set_toast("Playlist deleted");
                            }
                        }
                        ConfirmAction::ClearCache => self.apply_effect(ViewEffect::ClearCache),
                        ConfirmAction::Logout => self.apply_effect(ViewEffect::Logout),
                    }
                }
                _ => {}
            },
            ActiveModal::OAuthWizard {
                client_id,
                client_secret,
                page_id,
                active_field,
                device_user_code,
                polling,
                error,
                ..
            } => match action {
                Action::InputCancel => {
                    self.cancel_oauth_task();
                    keep = false;
                }
                Action::InputTab if device_user_code.is_none() => {
                    *active_field = (*active_field + 1) % 3;
                }
                Action::InputBackTab if device_user_code.is_none() => {
                    *active_field = (*active_field + 2) % 3;
                }
                Action::InputChar(c) if device_user_code.is_none() => match *active_field {
                    0 => client_id.push(c),
                    1 => client_secret.push(c),
                    _ => page_id.push(c),
                },
                Action::InputBackspace if device_user_code.is_none() => match *active_field {
                    0 => {
                        client_id.pop();
                    }
                    1 => {
                        client_secret.pop();
                    }
                    _ => {
                        page_id.pop();
                    }
                },
                Action::InputSubmit if device_user_code.is_none() => {
                    if client_id.trim().is_empty() || client_secret.trim().is_empty() {
                        *error = Some("Client ID and Client Secret are required".into());
                    } else {
                        self.cancel_oauth_task();
                        *polling = true;
                        *error = None;
                        let cid = client_id.trim().to_string();
                        let auth = self.services.auth.clone();
                        let tx = self.bg_tx.clone();
                        let jh = tokio::spawn(async move {
                            let res = auth
                                .start_oauth_device_flow(&cid)
                                .await
                                .map(|p| {
                                    (p.device_code, p.user_code, p.verification_url, p.interval)
                                })
                                .map_err(|e| e.to_string());
                            let _ = tx.send(BgMsg::OAuthCodeStarted(res));
                        });
                        self.oauth_task = Some(jh.abort_handle());
                    }
                }
                _ => {}
            },
            ActiveModal::BrowserAuthImport {
                raw_headers,
                page_id,
                active_field,
                error,
            } => match action {
                Action::InputCancel => keep = false,
                Action::InputTab => *active_field = (*active_field + 1) % 2,
                Action::InputBackTab => *active_field = (*active_field + 1) % 2,
                Action::InputChar(c) => {
                    if *active_field == 0 {
                        raw_headers.push(c);
                    } else {
                        page_id.push(c);
                    }
                }
                Action::InputBackspace => {
                    if *active_field == 0 {
                        raw_headers.pop();
                    } else {
                        page_id.pop();
                    }
                }
                Action::InputSubmit => {
                    let pid = if page_id.trim().is_empty() {
                        None
                    } else {
                        Some(page_id.trim().to_string())
                    };
                    match self.services.auth.import_browser_headers(raw_headers, pid) {
                        Ok(_) => {
                            keep = false;
                            self.settings.privacy.auth_mode = AuthMode::Browser;
                            let _ = self.settings.save(&self.services.paths.settings_file());
                            self.services.engine.try_send(AppCommand::ApplySettings {
                                settings: self.settings.clone(),
                            });
                            self.set_toast("Browser Auth imported — syncing library…");
                            self.trigger_sync_now();
                            self.trigger_home_refresh();
                        }
                        Err(err) => *error = Some(err.to_string()),
                    }
                }
                _ => {}
            },
            ActiveModal::KeymapHelp { scroll } => match action {
                Action::InputCancel | Action::InputChar('?') | Action::InputChar('q') => {
                    keep = false;
                }
                Action::InputDown | Action::InputChar('j') => *scroll = scroll.saturating_add(1),
                Action::InputUp | Action::InputChar('k') => *scroll = scroll.saturating_sub(1),
                _ => {}
            },
        }

        if keep {
            self.modal = Some(modal);
        }
    }

    fn apply_effect(&mut self, effect: ViewEffect) {
        match effect {
            ViewEffect::None => {}
            ViewEffect::Command(cmd) => {
                self.services.engine.try_send(cmd);
            }
            ViewEffect::OpenArtist {
                browse_id,
                name_hint,
            } => {
                let saved = self
                    .services
                    .store
                    .is_collection_saved(
                        &self.services.auth.account_key(),
                        SavedCollectionKind::Artist,
                        &browse_id,
                    )
                    .unwrap_or(false);
                self.push_nav(NavPage::Artist(Box::new(ArtistView::new(
                    browse_id.clone(),
                    name_hint,
                    self.settings.display.cover_size,
                    saved,
                ))));
                let cat = self.services.catalog.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    let res = cat
                        .artist(&browse_id)
                        .await
                        .map(Box::new)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(BgMsg::ArtistLoaded {
                        browse_id,
                        result: res,
                    });
                });
            }
            ViewEffect::OpenAlbum {
                browse_id,
                title_hint,
            } => {
                let saved = self
                    .services
                    .store
                    .is_collection_saved(
                        &self.services.auth.account_key(),
                        SavedCollectionKind::Album,
                        &browse_id,
                    )
                    .unwrap_or(false);
                self.push_nav(NavPage::Album(AlbumView::new(
                    browse_id.clone(),
                    title_hint,
                    saved,
                )));
                let cat = self.services.catalog.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    let res = cat
                        .album(&browse_id)
                        .await
                        .map(Box::new)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(BgMsg::AlbumLoaded {
                        browse_id,
                        result: res,
                    });
                });
            }
            ViewEffect::OpenPlaylist {
                browse_id,
                title_hint,
            } => {
                let saved = self
                    .services
                    .store
                    .is_collection_saved(
                        &self.services.auth.account_key(),
                        SavedCollectionKind::Playlist,
                        &browse_id,
                    )
                    .unwrap_or(false);
                self.push_nav(NavPage::PlaylistDetail(PlaylistDetailView::new(
                    browse_id.clone(),
                    title_hint,
                    saved,
                )));
                let cat = self.services.catalog.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    let res = cat
                        .playlist(&browse_id, 200)
                        .await
                        .map(Box::new)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(BgMsg::PlaylistLoaded {
                        browse_id,
                        result: res,
                    });
                });
            }
            ViewEffect::OpenUserPlaylist { playlist_id } => {
                self.active_tab = 1;
                self.nav_stack.clear();
                self.reload_local_collections();
                self.select_user_playlist(&playlist_id);
            }
            ViewEffect::PlayUserPlaylistDirect { playlist_id } => {
                match self.services.store.playlist_tracks(&playlist_id) {
                    Ok(tracks) if !tracks.is_empty() => {
                        self.services.engine.try_send(AppCommand::PlayTracks {
                            tracks,
                            start_index: 0,
                            autoplay: true,
                        });
                    }
                    Ok(_) => self.set_toast("Playlist is empty"),
                    Err(err) => self.set_toast(format!("Playlist load error: {err}")),
                }
            }
            ViewEffect::PlayAlbumDirect { browse_id } => {
                let cat = self.services.catalog.clone();
                let eng = self.services.engine.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    match cat.album(&browse_id).await {
                        Ok(alb) if !alb.tracks.is_empty() => {
                            eng.send(AppCommand::PlayTracks {
                                tracks: alb.tracks,
                                start_index: 0,
                                autoplay: true,
                            })
                            .await;
                        }
                        Ok(_) => {
                            let _ = tx.send(BgMsg::Toast("Album has no playable tracks".into()));
                        }
                        Err(err) => {
                            let _ = tx.send(BgMsg::Toast(format!("Album play error: {err}")));
                        }
                    }
                });
            }
            ViewEffect::PlayPlaylistDirect { browse_id } => {
                let cat = self.services.catalog.clone();
                let eng = self.services.engine.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    match cat.playlist(&browse_id, 100).await {
                        Ok(pl) if !pl.tracks.is_empty() => {
                            eng.send(AppCommand::PlayTracks {
                                tracks: pl.tracks,
                                start_index: 0,
                                autoplay: true,
                            })
                            .await;
                        }
                        Ok(_) => {
                            let _ = tx.send(BgMsg::Toast("Playlist is empty".into()));
                        }
                        Err(err) => {
                            let _ = tx.send(BgMsg::Toast(format!("Playlist play error: {err}")));
                        }
                    }
                });
            }
            ViewEffect::PlayArtistDirect { browse_id } => {
                let cat = self.services.catalog.clone();
                let eng = self.services.engine.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    match cat.artist(&browse_id).await {
                        Ok(art) if !art.songs.is_empty() => {
                            eng.send(AppCommand::PlayTracks {
                                tracks: art.songs,
                                start_index: 0,
                                autoplay: true,
                            })
                            .await;
                        }
                        Ok(_) => {
                            let _ = tx.send(BgMsg::Toast("Artist has no top songs".into()));
                        }
                        Err(err) => {
                            let _ = tx.send(BgMsg::Toast(format!("Artist play error: {err}")));
                        }
                    }
                });
            }
            ViewEffect::TriggerSearch { category, query } => {
                let search = self.services.search.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    let res = search
                        .search(category, &query, 40)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(BgMsg::SearchLoaded(res));
                });
            }
            ViewEffect::TriggerAutocomplete { query } => {
                let search = self.services.search.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    let sugs = search.autocomplete(&query, 7).await;
                    let _ = tx.send(BgMsg::SuggestionsLoaded(sugs));
                });
            }
            ViewEffect::RefreshHome => self.trigger_home_refresh(),
            ViewEffect::OpenModal(m) => self.modal = Some(m),
            ViewEffect::SyncNow => self.trigger_sync_now(),
            ViewEffect::ToggleLike(track) => match self.services.sync.toggle_like(&track) {
                Ok(true) => {
                    self.reload_local_collections();
                    self.set_toast(format!("Liked \"{}\"", track.title));
                }
                Ok(false) => {
                    self.reload_local_collections();
                    self.set_toast(format!("Removed \"{}\" from Liked Songs", track.title));
                }
                Err(err) => self.set_toast(format!("Like failed: {err}")),
            },
            ViewEffect::ToggleSave {
                kind,
                browse_id,
                title,
                subtitle,
                thumb_url,
            } => match self
                .services
                .sync
                .toggle_save_collection(kind, &browse_id, &title, subtitle, thumb_url)
            {
                Ok(now_saved) => {
                    self.update_nav_saved_state(kind, &browse_id, now_saved);
                    self.reload_local_collections();
                    if now_saved {
                        self.set_toast(format!("Saved \"{title}\" to Library"));
                    } else {
                        self.set_toast(format!("Removed \"{title}\" from Library"));
                    }
                }
                Err(err) => self.set_toast(format!("Save failed: {err}")),
            },
            ViewEffect::AddToPlaylistPicker(track) => {
                let playlists = self
                    .services
                    .store
                    .list_playlists(&self.services.auth.account_key())
                    .unwrap_or_default();
                self.modal = Some(ActiveModal::AddToPlaylist {
                    track,
                    playlists,
                    cursor: 0,
                });
            }
            ViewEffect::RemovePlaylistTrack {
                playlist_id,
                position,
            } => match self.services.sync.remove_track_at(&playlist_id, position) {
                Ok(Some(_)) => {
                    self.reload_local_collections();
                    self.set_toast("Removed track from playlist");
                }
                Ok(None) => {
                    self.reload_local_collections();
                }
                Err(err) => self.set_toast(format!("Remove failed: {err}")),
            },
            ViewEffect::MovePlaylistTrack {
                playlist_id,
                from,
                to,
            } => match self.services.sync.move_track(&playlist_id, from, to) {
                Ok(true) => {
                    self.reload_local_collections();
                }
                Ok(false) => {
                    self.playlists.track_cursor = from;
                    self.reload_local_collections();
                }
                Err(err) => {
                    self.playlists.track_cursor = from;
                    self.set_toast(format!("Reorder failed: {err}"));
                }
            },
            ViewEffect::ExportPlaylist { playlist_id } => {
                let dir = self.services.paths.data_dir.join("exports");
                let _ = std::fs::create_dir_all(&dir);
                if let Ok(Some(pl)) = self.services.store.get_playlist(&playlist_id) {
                    let safe: String = pl
                        .title
                        .chars()
                        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                        .collect();
                    let json_p = dir.join(format!("{safe}.json"));
                    let m3u_p = dir.join(format!("{safe}.m3u8"));
                    let ok1 = self
                        .services
                        .store
                        .export_playlist_json(&playlist_id)
                        .and_then(|s| musick_core::atomic_write_text(&json_p, &s));
                    let ok2 = self
                        .services
                        .store
                        .export_playlist_m3u8(&playlist_id)
                        .and_then(|s| musick_core::atomic_write_text(&m3u_p, &s));
                    if ok1.is_ok() && ok2.is_ok() {
                        self.set_toast(format!("Exported to {}", dir.display()));
                    } else {
                        self.set_toast("Playlist export failed");
                    }
                }
            }
            ViewEffect::UploadPlaylist { playlist_id } => {
                let sync = self.services.sync.clone();
                let tx = self.bg_tx.clone();
                tokio::spawn(async move {
                    let res = sync
                        .promote_local_to_synced(&playlist_id)
                        .await
                        .map(|pl| pl.title)
                        .map_err(|e| e.to_string());
                    let _ = tx.send(BgMsg::PlaylistUploaded(res));
                });
            }
            ViewEffect::CopyUrl(url) => {
                write_osc52_clipboard(&url);
                self.set_toast(format!("Copied URL: {url}"));
            }
            ViewEffect::UpdateSettings(next) => {
                let mut s = *next;
                let region_changed = s.display.region != self.settings.display.region;
                let mpris_changed = s.player.mpris_enabled != self.settings.player.mpris_enabled;
                s.normalize();
                self.settings = s.clone();
                self.spinner.reduced_motion = s.display.reduced_motion;
                let proto_changed = self.covers.set_protocol(s.display.image_protocol);
                self.library.cover_size = s.display.cover_size;
                self.services.catalog.set_region(&s.display.region);
                self.services.cache.update_settings(s.cache.clone());
                self.services.search.update_settings(s.clone());
                self.services.engine.try_send(AppCommand::ApplySettings {
                    settings: s.clone(),
                });
                let _ = s.save(&self.services.paths.settings_file());
                self.sync_cava_state();
                if mpris_changed {
                    self.sync_mpris_state();
                }
                if proto_changed {
                    self.requested_images.clear();
                    self.re_request_active_covers();
                }
                if region_changed {
                    self.trigger_home_refresh();
                }
            }
            ViewEffect::ClearCache => {
                self.services.images.clear_memory_cache();
                self.covers.clear_cache();
                self.requested_images.clear();
                match self.services.cache.clear_cache() {
                    Ok(()) => {
                        self.refresh_cache_stats();
                        self.re_request_active_covers();
                        self.set_toast("Cache cleared & visitorData rotated (Playlists safe)");
                    }
                    Err(err) => self.set_toast(format!("Clear cache failed: {err}")),
                }
            }
            ViewEffect::PruneCache => match self.services.cache.prune_all() {
                Ok(rep) => {
                    self.refresh_cache_stats();
                    self.set_toast(format!("Pruned {}", format_bytes(rep.bytes_freed)));
                }
                Err(err) => self.set_toast(format!("Prune failed: {err}")),
            },
            ViewEffect::ClearHistory => {
                let _ = self.services.store.clear_play_history();
                let _ = self.services.store.clear_search_history();
                self.reload_recent_plays();
                self.set_toast("Local history cleared");
            }
            ViewEffect::Logout => {
                self.cancel_oauth_task();
                let prev_acct = self.services.auth.account_key();
                if let Err(err) = self.services.auth.logout() {
                    self.set_toast(format!("Logout failed: {err}"));
                    return;
                }
                let _ = self.services.store.purge_account(&prev_acct);
                self.services.cache.clear_visitor_data();
                self.settings.privacy.auth_mode = AuthMode::Anonymous;
                self.settings.privacy.report_playback_history = false;
                let _ = self.settings.save(&self.services.paths.settings_file());
                self.services.engine.try_send(AppCommand::ApplySettings {
                    settings: self.settings.clone(),
                });
                self.reload_local_collections();
                self.trigger_home_refresh();
                self.set_toast("Logged out -> no cookies ● (local playlists kept)");
            }
            ViewEffect::StatusToast(msg) => self.set_toast(msg),
        }
    }

    fn push_nav(&mut self, page: NavPage) {
        if self.nav_stack.len() >= MAX_NAV_DEPTH {
            self.nav_stack.remove(0);
        }
        self.nav_stack.push(page);
        self.sync_cava_state();
    }

    fn trigger_home_refresh(&mut self) {
        self.home.loading_home = true;
        let cat = self.services.catalog.clone();
        let region = self.settings.display.region.clone();
        let tx = self.bg_tx.clone();
        tokio::spawn(async move {
            let res = cat.home_feed(&region).await.map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::HomeLoaded(res));
        });
    }

    fn trigger_sync_now(&mut self) {
        if self.syncing {
            return;
        }
        if !self.services.auth.is_authenticated() {
            self.set_toast("Anonymous mode — sign in via 5 Settings to enable 2-way sync");
            return;
        }
        self.syncing = true;
        let sync = self.services.sync.clone();
        let tx = self.bg_tx.clone();
        tokio::spawn(async move {
            let res = sync.sync_now().await.map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::SyncFinished(res));
        });
    }

    fn import_playlist_input(&mut self, raw: String) {
        let path = std::path::Path::new(&raw);
        if path.is_file()
            && let Ok(content) = std::fs::read_to_string(path)
        {
            let acct = "local";
            let res = if raw.ends_with(".json") {
                self.services.store.import_playlist_json(acct, &content)
            } else {
                let fallback = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Imported");
                self.services
                    .store
                    .import_playlist_m3u8(acct, fallback, &content)
            };
            match res {
                Ok(pl) => {
                    self.reload_local_collections();
                    self.select_user_playlist(&pl.id);
                    self.set_toast(format!(
                        "Imported \"{}\" ({} tracks)",
                        pl.title, pl.track_count
                    ));
                }
                Err(err) => self.set_toast(format!("Import file error: {err}")),
            }
            return;
        }

        let pid = match extract_youtube_target(&raw) {
            Some(YoutubeTarget::Playlist(id)) => id,
            _ => raw,
        };
        let cat = self.services.catalog.clone();
        let sync = self.services.sync.clone();
        let tx = self.bg_tx.clone();
        tokio::spawn(async move {
            let res = match cat.playlist(&pid, 200).await {
                Ok(remote) => match sync.create_playlist(
                    &remote.summary.title,
                    remote.description.as_deref().unwrap_or(""),
                    false,
                ) {
                    Ok(local_pl) => {
                        let _ = sync.add_tracks_to_playlist(&local_pl.id, &remote.tracks);
                        Ok((local_pl.id, remote.summary.title, remote.tracks.len()))
                    }
                    Err(err) => Err(format!("Save import failed: {err}")),
                },
                Err(err) => Err(format!("Import URL failed: {err}")),
            };
            let _ = tx.send(BgMsg::PlaylistImported(res));
        });
    }

    fn spawn_oauth_poller(
        &mut self,
        client_id: String,
        client_secret: String,
        device_code: String,
        page_id: Option<String>,
        interval_secs: u64,
    ) {
        self.cancel_oauth_task();
        let auth = self.services.auth.clone();
        let tx = self.bg_tx.clone();
        let jh = tokio::spawn(async move {
            let mut wait = Duration::from_secs(interval_secs.max(2));
            for _ in 0..120 {
                tokio::time::sleep(wait).await;
                match auth
                    .poll_oauth_device_token(
                        &client_id,
                        &client_secret,
                        &device_code,
                        page_id.clone(),
                    )
                    .await
                {
                    Ok(OAuthPollStatus::Pending) => {}
                    Ok(OAuthPollStatus::SlowDown) => wait += Duration::from_secs(3),
                    Ok(OAuthPollStatus::Authorized(c)) => {
                        let _ = tx.send(BgMsg::OAuthPollCompleted(Ok(
                            OAuthPollStatus::Authorized(c),
                        )));
                        return;
                    }
                    Err(err) => {
                        let _ = tx.send(BgMsg::OAuthPollCompleted(Err(err.to_string())));
                        return;
                    }
                }
            }
            let _ = tx.send(BgMsg::OAuthPollCompleted(Err(
                "OAuth device code timed out".into(),
            )));
        });
        self.oauth_task = Some(jh.abort_handle());
    }

    fn sync_mpris_state(&mut self) {
        if self.settings.player.mpris_enabled {
            if self.mpris.is_none()
                && let Some(ref flag) = self.mpris_quit_flag
            {
                self.mpris = Some(MprisService::spawn_with_quit_flag(
                    self.services.engine.clone(),
                    flag.clone(),
                ));
            }
        } else if let Some(m) = self.mpris.take() {
            m.shutdown();
        }
    }

    fn re_request_active_covers(&mut self) {
        let mut urls = Vec::new();
        if let Some(url) = self.snapshot.track.as_ref().and_then(Track::cover_url) {
            urls.push(url);
        }
        if let Some(top) = self.nav_stack.last() {
            match top {
                NavPage::Artist(v) => {
                    if let Some(ref art) = v.artist {
                        if let Some(u) = art.summary.cover_url() {
                            urls.push(u);
                        }
                        for al in art.albums.iter().chain(art.singles.iter()).take(48) {
                            if let Some(u) = al.cover_url() {
                                urls.push(u);
                            }
                        }
                    }
                }
                NavPage::Album(v) => {
                    if let Some(u) = v.album.as_ref().and_then(|a| a.summary.cover_url()) {
                        urls.push(u);
                    }
                }
                NavPage::PlaylistDetail(v) => {
                    if let Some(u) = v.playlist.as_ref().and_then(|p| p.summary.cover_url()) {
                        urls.push(u);
                    }
                }
            }
        }
        for col in self
            .library
            .saved_albums
            .iter()
            .chain(self.library.saved_artists.iter())
            .chain(self.library.saved_playlists.iter())
            .take(48)
        {
            if let Some(ref u) = col.thumb_url {
                urls.push(u.clone());
            }
        }
        for url in urls {
            self.request_cover_image(url);
        }
    }

    fn request_cover_image(&mut self, url: String) {
        if self.covers.has_image(&url) || !self.requested_images.insert(url.clone()) {
            return;
        }
        self.last_cover_url = Some(url.clone());
        let images = self.services.images.clone();
        let tx = self.bg_tx.clone();
        tokio::spawn(async move {
            if let Some(img) = images.fetch(&url).await {
                let _ = tx.send(BgMsg::ImageLoaded { url, image: img });
            } else {
                let _ = tx.send(BgMsg::ImageFailed { url });
            }
        });
    }

    pub fn reload_local_collections(&mut self) {
        let acct = self.services.auth.account_key();
        let pls = self
            .services
            .store
            .list_playlists(&acct)
            .unwrap_or_default();
        self.playlists.set_playlists(pls.clone());
        self.library.user_playlists = pls;
        self.library.liked_songs = self
            .services
            .store
            .list_liked_songs(&acct)
            .unwrap_or_default();
        self.library.saved_albums = self
            .services
            .store
            .list_saved_collections(&acct, SavedCollectionKind::Album)
            .unwrap_or_default();
        self.library.saved_artists = self
            .services
            .store
            .list_saved_collections(&acct, SavedCollectionKind::Artist)
            .unwrap_or_default();
        self.library.saved_playlists = self
            .services
            .store
            .list_saved_collections(&acct, SavedCollectionKind::Playlist)
            .unwrap_or_default();
        self.reload_recent_plays();

        let cover_urls: Vec<String> = self
            .library
            .saved_albums
            .iter()
            .chain(self.library.saved_artists.iter())
            .chain(self.library.saved_playlists.iter())
            .take(48)
            .filter_map(|c| c.thumb_url.clone())
            .collect();
        for url in cover_urls {
            self.request_cover_image(url);
        }

        for page in &mut self.nav_stack {
            let (kind, bid, saved_slot) = match page {
                NavPage::Artist(v) => (
                    SavedCollectionKind::Artist,
                    v.browse_id.as_str(),
                    &mut v.saved,
                ),
                NavPage::Album(v) => (
                    SavedCollectionKind::Album,
                    v.browse_id.as_str(),
                    &mut v.saved,
                ),
                NavPage::PlaylistDetail(v) => (
                    SavedCollectionKind::Playlist,
                    v.browse_id.as_str(),
                    &mut v.saved,
                ),
            };
            if let Ok(is_saved) = self.services.store.is_collection_saved(&acct, kind, bid) {
                *saved_slot = is_saved;
            }
        }

        self.sync_selected_playlist_tracks(true);
    }

    fn sync_selected_playlist_tracks(&mut self, force: bool) {
        let changed = self.playlists.selected_playlist().map(|p| p.id.as_str())
            != self.last_selected_playlist_id.as_deref();
        if changed || force {
            let cur_pl_id = self.playlists.selected_playlist().map(|p| p.id.clone());
            self.last_selected_playlist_id = cur_pl_id.clone();
            if changed {
                self.playlists.on_playlist_selection_changed();
            }
            let tracks = cur_pl_id
                .as_deref()
                .and_then(|id| self.services.store.playlist_tracks(id).ok())
                .unwrap_or_default();
            self.playlists.set_selected_tracks(tracks);
        }
    }

    fn select_user_playlist(&mut self, playlist_id: &str) {
        self.playlists.select_by_id(playlist_id);
        self.sync_selected_playlist_tracks(true);
    }

    fn update_nav_saved_state(&mut self, kind: SavedCollectionKind, browse_id: &str, saved: bool) {
        for page in &mut self.nav_stack {
            match (page, kind) {
                (NavPage::Artist(v), SavedCollectionKind::Artist) if v.browse_id == browse_id => {
                    v.saved = saved;
                }
                (NavPage::Album(v), SavedCollectionKind::Album) if v.browse_id == browse_id => {
                    v.saved = saved;
                }
                (NavPage::PlaylistDetail(v), SavedCollectionKind::Playlist)
                    if v.browse_id == browse_id =>
                {
                    v.saved = saved;
                }
                _ => {}
            }
        }
    }

    fn reload_recent_plays(&mut self) {
        self.library.recent_tracks = self.services.store.recent_plays(50).unwrap_or_default();
    }

    fn refresh_cache_stats(&mut self) {
        self.cache_stats = self.services.cache.stats().unwrap_or_default();
    }

    fn active_footer_hints(&self) -> &'static [(&'static str, &'static str)] {
        if let Some(top) = self.nav_stack.last() {
            match top {
                NavPage::Artist(v) => v.footer_hints(),
                NavPage::Album(v) => v.footer_hints(),
                NavPage::PlaylistDetail(v) => v.footer_hints(),
            }
        } else {
            match self.active_tab {
                0 => self.home.footer_hints(),
                1 => self.playlists.footer_hints(),
                2 => self.library.footer_hints(),
                3 => self.queue.footer_hints(),
                4 => self.settings_view.footer_hints(),
                _ => &[],
            }
        }
    }

    pub fn render(&mut self, frame: &mut Frame<'_>) {
        let full = frame.area();
        if full.width < 24 || full.height < 10 {
            return;
        }

        // Outer sharp app frame matching legacy #app-frame aesthetic with native transparent bg
        let frame_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .border_style(Style::default().fg(BORDER).bg(Color::Reset))
            .style(Style::default().bg(Color::Reset));
        let inner = frame_block.inner(full);
        frame.render_widget(frame_block, full);

        if inner.width < 20 || inner.height < 8 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // TopBar
                Constraint::Min(4),    // Active View Body
                Constraint::Length(5), // NowPlayingBar
                Constraint::Length(1), // Footer Hints
            ])
            .split(inner);

        let icons = icons_for(self.settings.display.icons);
        let auth_mode = self.services.auth.mode();
        let pending_sync = self.services.sync.pending_count();
        let toast_str = self.status_toast.as_ref().map(|(s, _)| s.as_str());

        // 1. TopBar (full width so divider joins outer frame!)
        frame.render_widget(
            TopBar {
                active_tab: self.active_tab,
                icons,
                auth_mode,
                syncing: self.syncing,
                pending_sync,
                spinner: self.spinner,
                status_toast: toast_str,
            },
            chunks[0],
        );

        // 1-cell horizontal inset for main view, player bar, and footer
        let body_area = inset_h(chunks[1], 1);
        let player_area = inset_h(chunks[2], 1);
        let footer_area = inset_h(chunks[3], 1);

        let ctx = RenderCtx {
            snapshot: &self.snapshot,
            position: self.position,
            queue: &self.queue_snap,
            lyrics: &self.lyrics,
            cava: &self.cava_frame,
            settings: &self.settings,
            icons,
            spinner: self.spinner,
            auth_mode,
            syncing: self.syncing,
            pending_sync,
            cache_stats: &self.cache_stats,
        };

        // 2. Active View Body
        if let Some(top) = self.nav_stack.last_mut() {
            match top {
                NavPage::Artist(v) => v.render(frame, body_area, &ctx, &mut self.covers),
                NavPage::Album(v) => v.render(frame, body_area, &ctx, &mut self.covers),
                NavPage::PlaylistDetail(v) => v.render(frame, body_area, &ctx, &mut self.covers),
            }
        } else {
            match self.active_tab {
                0 => self.home.render(frame, body_area, &ctx, &mut self.covers),
                1 => self.playlists.render(frame, body_area, &ctx),
                2 => self
                    .library
                    .render(frame, body_area, &ctx, &mut self.covers),
                3 => self.queue.render(frame, body_area, &ctx),
                4 => self.settings_view.render(frame, body_area, &ctx),
                _ => {}
            }
        }

        // 3. Persistent Now Playing Control Bar
        frame.render_widget(
            NowPlayingBar {
                snapshot: &self.snapshot,
                position: self.position,
                icons,
                spinner: self.spinner,
                focused: false,
            },
            player_area,
        );

        // 4. Footer Key Hints + Pending Chord Indicator
        let mut f_spans = Vec::new();
        if let Some(chord) = self.keymap.pending_label() {
            f_spans.push(Span::styled(
                format!("[{chord}] "),
                Style::default()
                    .fg(ACCENT)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        for (i, &(k, label)) in self.active_footer_hints().iter().enumerate() {
            if i > 0 {
                f_spans.push(Span::styled(
                    "  ",
                    Style::default().fg(DIM).bg(Color::Reset),
                ));
            }
            f_spans.push(Span::styled(
                k,
                Style::default()
                    .fg(TEXT)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            ));
            f_spans.push(Span::styled(
                format!(" {label}"),
                Style::default().fg(DIM).bg(Color::Reset),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(f_spans)), footer_area);

        // 5. Active Modal Overlay (rendered last on top)
        if let Some(ref modal) = self.modal {
            render_modal(frame, full, modal, self.spinner);
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.cancel_oauth_task();
        self.shutdown_mpris();
    }
}

fn inset_h(r: Rect, pad: u16) -> Rect {
    let double = pad.saturating_mul(2);
    if r.width <= double {
        return r;
    }
    Rect {
        x: r.x + pad,
        y: r.y,
        width: r.width - double,
        height: r.height,
    }
}

fn write_osc52_clipboard(text: &str) {
    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let mut stdout = std::io::stdout();
    let _ = write!(stdout, "\x1b]52;c;{b64}\x07");
    let _ = stdout.flush();
}
