mod doctor;
mod update;

use std::fs::OpenOptions;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use musick_core::{
    AppPaths, AuthManager, AuthMode, CacheManager, KeyringSecretStore, OAuthPollStatus, Settings,
    UserStore, build_http_client, ensure_private_dir,
};

use crate::text::format_bytes;

pub use update::{parse_sha256sums, release_asset_name};

#[derive(Parser)]
#[command(
    name = "musick",
    version,
    about = "Modern full-Rust TUI YouTube Music streaming player (no cookies by default)"
)]
pub struct Cli {
    /// write debug logs to a file (also enabled via MUSICK_LOG env var)
    #[arg(long, global = true)]
    pub log_file: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// manage opt-in youtube music authentication (oauth2 or browser auth)
    Auth {
        #[command(subcommand)]
        action: AuthCommands,
    },
    /// inspect, prune, or clear disk caches (never touches user playlists or library)
    Cache {
        #[command(subcommand)]
        action: CacheCommands,
    },
    /// check runtime dependencies, terminal graphics, xdg permissions, and connectivity
    Doctor {
        /// skip network connectivity check
        #[arg(long)]
        offline: bool,
    },
    /// download and verify the latest prebuilt linux release from github
    Update {
        /// only check if a newer release is available without modifying the binary
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
pub enum AuthCommands {
    /// show current authentication status
    Status,
    /// authenticate via google oauth2 tv/device flow
    Oauth {
        #[arg(long, env = "MUSICK_OAUTH_CLIENT_ID")]
        client_id: String,
        #[arg(long, env = "MUSICK_OAUTH_CLIENT_SECRET")]
        client_secret: String,
        #[arg(long)]
        page_id: Option<String>,
    },
    /// import browser devtools headers or cookie string (reads from stdin)
    Browser {
        #[arg(long)]
        page_id: Option<String>,
    },
    /// logout, wipe credentials, purge synced remote cache, and reset to anonymous
    Logout,
}

#[derive(Subcommand)]
pub enum CacheCommands {
    /// display live disk cache usage statistics
    Stats,
    /// run cache janitor to prune expired and over-budget entries
    Prune,
    /// clear all caches and rotate visitor data (preserves musick.db)
    Clear,
}

pub fn init_logging(log_file: Option<&Path>, paths: &AppPaths) {
    let env_filter_str = std::env::var("MUSICK_LOG").ok();
    if log_file.is_none() && env_filter_str.as_deref().unwrap_or("").trim().is_empty() {
        return;
    }

    let target_path = log_file
        .map(Path::to_path_buf)
        .unwrap_or_else(|| paths.cache_dir.join("musick.log"));
    if let Some(parent) = target_path.parent() {
        let _ = ensure_private_dir(parent);
    }

    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }

    let Ok(file) = opts.open(&target_path) else {
        return;
    };
    let filter_spec = env_filter_str
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "musick_core=debug,musick_audio=debug,musick_tui=debug".to_string());
    let filter = EnvFilter::try_new(filter_spec).unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(filter)
        .with_writer(Mutex::new(file))
        .try_init();
}

pub async fn run_cli_command(
    cmd: Commands,
    paths: &AppPaths,
    mut settings: Settings,
) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        Commands::Doctor { offline } => return doctor::run_doctor(paths, &settings, offline).await,
        Commands::Update { check } => return update::run_update(check).await,
        _ => {}
    }

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
                            if let Ok(cache) = CacheManager::open(paths, settings.cache.clone()) {
                                cache.clear_visitor_data();
                            }
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
                let creds = auth.import_browser_headers(&raw, page_id)?;
                settings.privacy.auth_mode = AuthMode::Browser;
                settings.save(&paths.settings_file())?;
                if let Ok(cache) = CacheManager::open(paths, settings.cache.clone()) {
                    cache.clear_visitor_data();
                    if let Some(vd) = creds.visitor_id.as_deref() {
                        cache.set_visitor_data(vd);
                    }
                }
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
        Commands::Doctor { .. } | Commands::Update { .. } => {}
    }
    Ok(())
}
