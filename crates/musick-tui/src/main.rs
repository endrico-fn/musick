use std::io::{self, Read};
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use musick_audio::{
    CavaControl, CavaRunner, HybridStreamResolver, MprisService, MpvIpcBackend, PlayerEngine,
    PlayerEngineDeps,
};
use musick_core::{
    AppCommand, AppPaths, AuthManager, AuthMode, CacheManager, ImageService, InnerTubeClient,
    KeyringSecretStore, LyricsService, OAuthPollStatus, SearchService, Settings, SyncEngine,
    UserStore, build_http_client,
};
use musick_tui::app::{App, AppServices, BgMsg};
use musick_tui::text::format_bytes;
use musick_tui::widgets::CoverRenderer;

#[derive(Parser)]
#[command(
    name = "musick",
    version,
    about = "Modern full-Rust TUI YouTube Music streaming player (no cookies by default)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage opt-in YouTube Music authentication (OAuth2 or Browser Auth)
    Auth {
        #[command(subcommand)]
        action: AuthCommands,
    },
    /// Inspect, prune, or clear disk caches (never touches user playlists or library)
    Cache {
        #[command(subcommand)]
        action: CacheCommands,
    },
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Show current authentication status
    Status,
    /// Authenticate via Google OAuth2 TV/Device Flow
    Oauth {
        #[arg(long, env = "MUSICK_OAUTH_CLIENT_ID")]
        client_id: String,
        #[arg(long, env = "MUSICK_OAUTH_CLIENT_SECRET")]
        client_secret: String,
        #[arg(long)]
        page_id: Option<String>,
    },
    /// Import browser DevTools headers or Cookie string (reads from stdin)
    Browser {
        #[arg(long)]
        page_id: Option<String>,
    },
    /// Logout, wipe credentials, purge synced remote cache, and reset to Anonymous
    Logout,
}

#[derive(Subcommand)]
enum CacheCommands {
    /// Display live disk cache usage statistics
    Stats,
    /// Run cache janitor to prune expired and over-budget entries
    Prune,
    /// Clear all caches and rotate visitorData (preserves musick.db)
    Clear,
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let paths = AppPaths::default_system()?;
    paths.ensure_dirs()?;
    let settings = Settings::load(&paths.settings_file());

    if let Some(cmd) = cli.command {
        return run_cli_command(cmd, &paths, settings).await;
    }

    run_tui(paths, settings).await
}

async fn run_cli_command(
    cmd: Commands,
    paths: &AppPaths,
    mut settings: Settings,
) -> Result<(), Box<dyn std::error::Error>> {
    let http = build_http_client();
    let store = Arc::new(KeyringSecretStore::new(paths));
    let auth = AuthManager::new_anonymous(store, http);
    let _ = auth.load_for_mode(settings.privacy.auth_mode);

    match cmd {
        Commands::Auth { action } => match action {
            AuthCommands::Status => {
                println!("Auth mode:   {}", auth.mode().as_str());
                println!("Account key: {}", auth.account_key());
                if let Some(pid) = auth.on_behalf_of_user() {
                    println!("Brand Page:  {pid}");
                }
                println!(
                    "Telemetry:   {}",
                    if settings.privacy.report_playback_history && auth.is_authenticated() {
                        "enabled (opt-in)"
                    } else {
                        "disabled"
                    }
                );
            }
            AuthCommands::Oauth {
                client_id,
                client_secret,
                page_id,
            } => {
                let prompt = auth.start_oauth_device_flow(&client_id).await?;
                println!("Open: {}", prompt.verification_url);
                println!("Code: {}", prompt.user_code);
                println!("Waiting for authorization…");
                let mut wait = Duration::from_secs(prompt.interval.max(2));
                loop {
                    tokio::time::sleep(wait).await;
                    match auth
                        .poll_oauth_device_token(
                            &client_id,
                            &client_secret,
                            &prompt.device_code,
                            page_id.clone(),
                        )
                        .await?
                    {
                        OAuthPollStatus::Pending => {}
                        OAuthPollStatus::SlowDown => wait += Duration::from_secs(3),
                        OAuthPollStatus::Authorized(_) => {
                            settings.privacy.auth_mode = AuthMode::OAuth;
                            settings.save(&paths.settings_file())?;
                            println!("Authorized! Account key: {}", auth.account_key());
                            break;
                        }
                    }
                }
            }
            AuthCommands::Browser { page_id } => {
                println!("Paste DevTools request headers or Cookie string, then press Ctrl-D:");
                let mut raw = String::new();
                io::stdin().read_to_string(&mut raw)?;
                auth.import_browser_headers(&raw, page_id)?;
                settings.privacy.auth_mode = AuthMode::Browser;
                settings.save(&paths.settings_file())?;
                println!(
                    "Browser auth stored securely. Account key: {}",
                    auth.account_key()
                );
            }
            AuthCommands::Logout => {
                let prev = auth.account_key();
                auth.logout()?;
                let user_store = UserStore::open(paths)?;
                user_store.purge_account(&prev)?;
                let cache = CacheManager::open(paths, settings.cache.clone())?;
                cache.clear_visitor_data();
                settings.privacy.auth_mode = AuthMode::Anonymous;
                settings.privacy.report_playback_history = false;
                settings.save(&paths.settings_file())?;
                println!("Logged out cleanly -> anonymous (no cookies). Local playlists kept.");
            }
        },
        Commands::Cache { action } => {
            let cache = CacheManager::open(paths, settings.cache.clone())?;
            match action {
                CacheCommands::Stats => {
                    let st = cache.stats()?;
                    println!(
                        "Catalog: {} entries ({})",
                        st.catalog_entries,
                        format_bytes(st.catalog_bytes)
                    );
                    println!(
                        "Lyrics:  {} entries ({})",
                        st.lyrics_entries,
                        format_bytes(st.lyrics_bytes)
                    );
                    println!(
                        "Images:  {} files ({})",
                        st.image_files,
                        format_bytes(st.image_bytes)
                    );
                    println!("Total:   {}", format_bytes(st.total_bytes));
                }
                CacheCommands::Prune => {
                    let rep = cache.prune_all()?;
                    println!(
                        "Pruned {} catalog, {} lyrics, {} images, {} orphans ({} freed)",
                        rep.catalog_evicted,
                        rep.lyrics_evicted,
                        rep.images_evicted,
                        rep.orphan_files_removed,
                        format_bytes(rep.bytes_freed)
                    );
                }
                CacheCommands::Clear => {
                    cache.clear_cache()?;
                    println!("Cache cleared and visitorData rotated (musick.db untouched).");
                }
            }
        }
    }
    Ok(())
}

async fn run_tui(paths: AppPaths, settings: Settings) -> Result<(), Box<dyn std::error::Error>> {
    // Install panic hook to always restore terminal state cleanly (Review Focus #5)
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
    let resolver = Arc::new(HybridStreamResolver::new(
        &paths,
        settings.player.audio_quality,
    ));

    let (engine_handle, engine_join) = PlayerEngine::spawn(PlayerEngineDeps {
        backend: backend.clone(),
        resolver,
        catalog: catalog.clone(),
        lyrics,
        store: store.clone(),
        auth: auth.clone(),
        settings: settings.clone(),
    });

    let mpris = if settings.player.mpris_enabled {
        Some(MprisService::spawn(engine_handle.clone()))
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

    // Background Cache Janitor (every 15 minutes; offloaded to blocking pool)
    let janitor_cache = cache.clone();
    let janitor_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15 * 60));
        interval.tick().await; // skip immediate tick
        loop {
            interval.tick().await;
            let c = janitor_cache.clone();
            let _ = tokio::task::spawn_blocking(move || c.prune_all()).await;
        }
    });

    // Enter TUI alternate screen and detect terminal graphics protocol
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let covers = CoverRenderer::detect(settings.display.image_protocol);
    let backend_tui = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend_tui)?;
    terminal.clear()?;

    let sync_worker_engine = sync.clone();
    let sync_worker_auth = auth.clone();
    let sync_period = settings.sync.auto_sync_interval_secs.max(60);

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

    // Background Opt-In Sync Worker (default 5 minutes; strict no-op when Anonymous)
    let sync_bg_tx = app.bg_sender();
    let sync_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(sync_period));
        interval.tick().await;
        loop {
            interval.tick().await;
            if sync_worker_auth.is_authenticated() {
                let res = sync_worker_engine
                    .sync_now()
                    .await
                    .map_err(|e| e.to_string());
                let _ = sync_bg_tx.send(BgMsg::SyncFinished(res));
            }
        }
    });

    app.kick_startup_loads();

    // SIGINT / SIGTERM graceful shutdown flag
    let shutdown_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sig_flag = shutdown_flag.clone();
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
            sig_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
            sig_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    });

    // Adaptive Event Loop: 80ms active (spinner / cava / loading), 250ms idle
    loop {
        app.on_tick();
        terminal.draw(|f| app.render(f))?;

        if app.should_quit || shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }

        let poll_timeout = if app.is_animating() {
            Duration::from_millis(80)
        } else {
            Duration::from_millis(250)
        };

        if event::poll(poll_timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind == crossterm::event::KeyEventKind::Press {
                        app.handle_key(key);
                    }
                }
                Event::Paste(pasted) => {
                    app.handle_paste(&pasted);
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }

    // Clean teardown: restore terminal, stop mpv, stop cava, stop MPRIS & workers
    restore_terminal();
    sig_task.abort();
    janitor_task.abort();
    sync_task.abort();
    if let Some(m) = mpris {
        m.shutdown();
    }
    cava_handle.shutdown();
    engine_handle.send(AppCommand::Shutdown).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), engine_join).await;
    let _ = tokio::time::timeout(Duration::from_secs(1), cava_join).await;

    Ok(())
}
