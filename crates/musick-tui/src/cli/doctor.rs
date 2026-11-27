use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use musick_core::{AppPaths, Settings};

async fn probe_command(bin: &str, args: &[&str]) -> Option<String> {
    let fut = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(3), fut)
        .await
        .ok()?
        .ok()?;
    let combined = if out.stdout.is_empty() {
        String::from_utf8_lossy(&out.stderr).into_owned()
    } else {
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let line = combined.lines().next().unwrap_or("").trim().to_string();
    if out.status.success() || !line.is_empty() {
        Some(if line.is_empty() {
            "found".to_string()
        } else {
            line
        })
    } else {
        None
    }
}

fn dir_status(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(meta) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = meta.permissions().mode() & 0o777;
                if mode == 0o700 {
                    format!("ok ({mode:04o}) {}", path.display())
                } else {
                    format!("warn (mode {mode:04o}, expected 0700) {}", path.display())
                }
            }
            #[cfg(not(unix))]
            {
                let _ = meta;
                format!("ok {}", path.display())
            }
        }
        Err(err) => format!("missing ({err}) {}", path.display()),
    }
}

pub async fn run_doctor(
    paths: &AppPaths,
    settings: &Settings,
    offline: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("musick {} doctor", env!("CARGO_PKG_VERSION"));
    println!(
        "Build:    {} {} (mit)",
        std::env::consts::OS,
        std::env::consts::ARCH,
    );

    println!("\nRuntime binaries:");
    let mpv = probe_command("mpv", &["--version"]).await;
    let ytdlp = probe_command("yt-dlp", &["--version"]).await;
    let cava = probe_command("cava", &["-v"]).await;
    let deno = probe_command("deno", &["--version"]).await;
    let pactl = probe_command("pactl", &["--version"]).await;

    let mut missing_required = false;
    match mpv {
        Some(v) => println!("  [ok]   mpv      {v}"),
        None => {
            missing_required = true;
            println!("  [miss] mpv      REQUIRED for audio playback");
        }
    }
    match ytdlp {
        Some(v) => println!("  [ok]   yt-dlp   {v}"),
        None => {
            missing_required = true;
            println!("  [miss] yt-dlp   REQUIRED for stream fallback & captions");
        }
    }
    match cava {
        Some(v) => println!("  [ok]   cava     {v} (optional visualizer)"),
        None => println!("  [opt]  cava     not found (visualizer will stay idle)"),
    }
    match deno {
        Some(v) => println!("  [ok]   deno     {v} (optional yt-dlp JS runtime)"),
        None => println!("  [opt]  deno     not found (optional for yt-dlp)"),
    }
    match pactl {
        Some(v) => println!("  [ok]   pactl    {v} (optional PulseAudio monitor)"),
        None => println!("  [opt]  pactl    not found"),
    }

    println!("\nTerminal & graphics:");
    let term = std::env::var("TERM").unwrap_or_else(|_| "(unset)".to_string());
    let term_prog = std::env::var("TERM_PROGRAM").unwrap_or_else(|_| "(unset)".to_string());
    let kitty = std::env::var_os("KITTY_WINDOW_ID").is_some();
    println!("  TERM={term}  TERM_PROGRAM={term_prog}  kitty={kitty}");
    println!("  image_protocol = {:?}", settings.display.image_protocol);

    println!("\nXDG directories:");
    println!("  config:  {}", dir_status(&paths.config_dir));
    println!("  data:    {}", dir_status(&paths.data_dir));
    println!("  cache:   {}", dir_status(&paths.cache_dir));
    println!("  runtime: {}", dir_status(&paths.runtime_dir));

    println!("\nPrivacy & auth:");
    println!("  auth_mode = {}", settings.privacy.auth_mode.as_str());
    println!(
        "  telemetry = {}",
        if settings.privacy.report_playback_history && settings.privacy.auth_mode.is_authenticated()
        {
            "enabled (opt-in)"
        } else {
            "disabled"
        }
    );

    if !offline {
        println!("\nConnectivity:");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .user_agent(concat!("musick/", env!("CARGO_PKG_VERSION")))
            .build()?;
        match client.head("https://music.youtube.com").send().await {
            Ok(resp) => println!("  [ok]   music.youtube.com (HTTP {})", resp.status()),
            Err(err) => println!("  [warn] music.youtube.com unreachable: {err}"),
        }
    }

    if missing_required {
        println!("\nOne or more required binaries are missing (install mpv and yt-dlp).");
    } else {
        println!("\nAll required checks passed.");
    }
    Ok(())
}
