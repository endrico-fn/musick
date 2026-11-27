use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::{Notify, mpsc, watch};

use musick_audio::{
    CavaControl, CavaRunner, HybridStreamResolver, MprisService, MpvIpcBackend, PlayerEngine,
    PlayerEngineDeps,
};
use musick_core::{
    AppCommand, AppPaths, AuthManager, CacheManager, ImageService, InnerTubeClient,
    KeyringSecretStore, LyricsService, SearchService, Settings, SyncEngine, UserStore,
    build_http_client,
};
use musick_tui::app::{App, AppServices, BgMsg};
use musick_tui::cli::{Cli, init_logging, run_cli_command};
use musick_tui::widgets::{CoverRenderer, write_kitty_delete_all};

const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(33);

fn restore_terminal() {
    write_kitty_delete_all();
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen
    );
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let paths = AppPaths::default_system()?;
    paths.ensure_dirs()?;
    init_logging(cli.log_file.as_deref(), &paths);
    let settings = Settings::load(&paths.settings_file());

    if let Some(cmd) = cli.command {
        return run_cli_command(cmd, &paths, settings).await;
    }

    run_tui(paths, settings).await
}

async fn run_tui(paths: AppPaths, settings: Settings) -> Result<(), Box<dyn std::error::Error>> {
    // always restore terminal state cleanly on panic
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        orig_hook(info);
    }));

    let http = build_http_client();
    let secret_store = Arc::new(KeyringSecretStore::new(&paths));
    let auth = Arc::new(AuthManager::new_anonymous(secret_store, http.clone()));
    let _ = auth.load_for_mode(settings.privacy.auth_mode);

    let store = Arc::new(UserStore::open(&paths)?);
    let cache = Arc::new(CacheManager::open(&paths, settings.cache.clone())?);
    let catalog = Arc::new(InnerTubeClient::new(
        http.clone(),
        auth.clone(),
        cache.clone(),
        settings.display.region.clone(),
    ));
    let search = Arc::new(SearchService::new(
        catalog.clone(),
        store.clone(),
        settings.clone(),
    ));
    let lyrics = Arc::new(LyricsService::new(
        Some(catalog.clone()),
        cache.clone(),
        http.clone(),
        settings.clone(),
    ));
    let images = Arc::new(ImageService::new(http, cache.clone()));
    let sync = Arc::new(SyncEngine::new(
        store.clone(),
        catalog.clone(),
        auth.clone(),
    ));

    let backend = Arc::new(MpvIpcBackend::new(&paths, settings.clone()));
    let resolver = Arc::new(HybridStreamResolver::new(settings.player.audio_quality));

    let (engine_handle, engine_join) = PlayerEngine::spawn(PlayerEngineDeps {
        backend: backend.clone(),
        resolver,
        catalog: catalog.clone(),
        lyrics,
        store: store.clone(),
        auth: auth.clone(),
        settings: settings.clone(),
    });

    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_notify = Arc::new(Notify::new());

    let mpris = if settings.player.mpris_enabled {
        Some(MprisService::spawn_with_quit_flag(
            engine_handle.clone(),
            shutdown_flag.clone(),
        ))
    } else {
        None
    };

    let (cava_handle, cava_join) = CavaRunner::spawn(CavaControl {
        enabled: settings.display.cava_enabled,
        smoothing: settings.display.cava_smoothing,
        audio_source: settings.display.cava_audio_source,
        reduced_motion: settings.display.reduced_motion,
        ..CavaControl::default()
    });

    let janitor_cache = cache.clone();
    let janitor_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15 * 60));
        interval.tick().await;
        loop {
            interval.tick().await;
            let c = janitor_cache.clone();
            let _ = tokio::task::spawn_blocking(move || c.prune_all()).await;
        }
    });

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    )?;
    let covers = CoverRenderer::detect(settings.display.image_protocol);
    let backend_tui = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend_tui)?;
    terminal.clear()?;

    let sync_worker_engine = sync.clone();
    let sync_worker_auth = auth.clone();
    let initial_sync_period = settings.sync.auto_sync_interval_secs.max(60);
    let (sync_interval_tx, mut sync_interval_rx) = watch::channel(initial_sync_period);

    let mut app = App::new(
        AppServices {
            paths,
            store,
            cache,
            auth,
            catalog,
            search,
            images,
            sync,
            engine: engine_handle.clone(),
            cava: Some(cava_handle.clone()),
        },
        settings,
        covers,
    );
    app.attach_mpris(mpris, shutdown_flag.clone());
    app.attach_sync_interval_tx(sync_interval_tx);

    let sync_bg_tx = app.bg_sender();
    let sync_task = tokio::spawn(async move {
        let mut period = initial_sync_period;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(period)) => {
                    if sync_worker_auth.is_authenticated() {
                        let res = sync_worker_engine
                            .sync_now()
                            .await
                            .map_err(|e| e.to_string());
                        let _ = sync_bg_tx.send(BgMsg::SyncFinished(res));
                    }
                }
                changed = sync_interval_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    period = (*sync_interval_rx.borrow_and_update()).max(60);
                }
            }
        }
    });

    app.kick_startup_loads();

    let sig_flag = shutdown_flag.clone();
    let sig_notify = shutdown_notify.clone();
    let sig_task = tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sigterm = signal(SignalKind::terminate()).ok();
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = async {
                    if let Some(ref mut s) = sigterm {
                        let _ = s.recv().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {}
            }
            sig_flag.store(true, Ordering::SeqCst);
            sig_notify.notify_waiters();
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
            sig_flag.store(true, Ordering::SeqCst);
            sig_notify.notify_waiters();
        }
    });

    let input_stop = Arc::new(AtomicBool::new(false));
    let input_stop_clone = input_stop.clone();
    let (input_tx, mut input_rx) = mpsc::channel::<Event>(64);
    let input_thread = std::thread::spawn(move || {
        while !input_stop_clone.load(Ordering::Relaxed) {
            match event::poll(Duration::from_millis(50)) {
                Ok(true) => match event::read() {
                    Ok(ev) => {
                        if input_tx.blocking_send(ev).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        }
    });

    let mut snap_rx = engine_handle.snapshot_rx.clone();
    let mut pos_rx = engine_handle.pos_rx.clone();
    let mut queue_rx = engine_handle.queue_rx.clone();
    let mut lyrics_rx = engine_handle.lyrics_rx.clone();
    let mut cava_rx = cava_handle.frame_rx.clone();
    let mut anim_tick = tokio::time::interval(Duration::from_millis(80));
    anim_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut last_draw = Instant::now() - MIN_FRAME_INTERVAL;
    let mut needs_redraw = true;

    loop {
        if needs_redraw {
            app.poll_engine_and_bg();
            terminal.draw(|f| app.render(f))?;
            last_draw = Instant::now();
            needs_redraw = false;
        }

        if app.should_quit || shutdown_flag.load(Ordering::SeqCst) {
            break;
        }

        let animating = app.is_animating();
        let since_draw = last_draw.elapsed();
        let pace_wait = MIN_FRAME_INTERVAL.saturating_sub(since_draw);
        let idle_sleep = tokio::time::sleep(Duration::from_millis(250));
        tokio::pin!(idle_sleep);

        tokio::select! {
            maybe_ev = input_rx.recv() => {
                if let Some(ev) = maybe_ev {
                    handle_term_event(&mut app, ev);
                    while let Ok(ev2) = input_rx.try_recv() {
                        handle_term_event(&mut app, ev2);
                    }
                    needs_redraw = true;
                } else {
                    break;
                }
            }
            maybe_bg = app.recv_bg() => {
                if let Some(msg) = maybe_bg {
                    app.handle_bg_msg(msg);
                    needs_redraw = true;
                }
            }
            _ = snap_rx.changed() => {
                needs_redraw = true;
            }
            _ = queue_rx.changed() => {
                needs_redraw = true;
            }
            _ = lyrics_rx.changed() => {
                needs_redraw = true;
            }
            _ = pos_rx.changed(), if pace_wait.is_zero() => {
                needs_redraw = true;
            }
            _ = cava_rx.changed(), if animating && pace_wait.is_zero() => {
                needs_redraw = true;
            }
            _ = tokio::time::sleep(pace_wait), if !pace_wait.is_zero() && (animating || pos_rx.has_changed().unwrap_or(false) || cava_rx.has_changed().unwrap_or(false)) => {
                let _ = pos_rx.borrow_and_update();
                let _ = cava_rx.borrow_and_update();
                needs_redraw = true;
            }
            _ = anim_tick.tick(), if animating => {
                app.on_tick();
                needs_redraw = true;
            }
            _ = &mut idle_sleep, if !animating => {
                app.on_tick();
                needs_redraw = true;
            }
            _ = shutdown_notify.notified() => {
                break;
            }
        }
    }

    app.flush_pending_saves();
    input_stop.store(true, Ordering::SeqCst);
    let _ = input_thread.join();
    restore_terminal();
    sig_task.abort();
    janitor_task.abort();
    sync_task.abort();
    app.shutdown_mpris();
    cava_handle.shutdown();
    engine_handle.send(AppCommand::Shutdown).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), engine_join).await;
    let _ = tokio::time::timeout(Duration::from_secs(1), cava_join).await;

    Ok(())
}

fn handle_term_event(app: &mut App, ev: Event) {
    match ev {
        Event::Key(key) => {
            if key.kind == crossterm::event::KeyEventKind::Press {
                app.handle_key(key);
            }
        }
        Event::Mouse(mouse) => {
            app.handle_mouse(mouse);
        }
        Event::Paste(pasted) => {
            app.handle_paste(&pasted);
        }
        Event::Resize(_, _) => {}
        _ => {}
    }
}
