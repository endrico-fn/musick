use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::process::Command;

const REPO: &str = "endrico-fn/musick";

pub fn release_asset_name(arch: &str) -> Option<String> {
    let normalized_arch = match arch {
        "x86_64" | "amd64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        _ => return None,
    };
    Some(format!("musick-linux-{normalized_arch}.tar.gz"))
}

pub fn parse_sha256sums(sums_text: &str, asset_name: &str) -> Option<String> {
    for line in sums_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        if name == asset_name && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

pub async fn run_update(check_only: bool) -> Result<(), Box<dyn std::error::Error>> {
    if std::env::consts::OS != "linux" {
        return Err("musick update currently supports Linux prebuilt releases only".into());
    }
    let asset_name = release_asset_name(std::env::consts::ARCH)
        .ok_or_else(|| format!("Unsupported architecture: {}", std::env::consts::ARCH))?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("musick/", env!("CARGO_PKG_VERSION")))
        .build()?;

    let release_url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = client.get(&release_url).send().await?;
    if !resp.status().is_success() {
        return Err(format!("GitHub Releases API returned HTTP {}", resp.status()).into());
    }
    let meta: Value = resp.json().await?;
    let tag = meta
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or("Latest release missing tag_name")?;
    let latest_ver = tag.trim_start_matches('v');
    let current_ver = env!("CARGO_PKG_VERSION");

    println!("Current version: v{current_ver}");
    println!("Latest release:  {tag} ({asset_name})");

    if latest_ver == current_ver {
        println!("musick is already up to date.");
        return Ok(());
    }
    if check_only {
        println!("Update available: v{current_ver} -> {tag}");
        return Ok(());
    }

    let assets = meta
        .get("assets")
        .and_then(Value::as_array)
        .ok_or("Latest release missing assets list")?;

    let find_url = |name: &str| -> Option<String> {
        assets.iter().find_map(|a| {
            if a.get("name").and_then(Value::as_str) == Some(name) {
                a.get("browser_download_url")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            } else {
                None
            }
        })
    };

    let tarball_url = find_url(&asset_name)
        .ok_or_else(|| format!("Asset {asset_name} not found in release {tag}"))?;
    let sums_url = find_url("SHA256SUMS.txt")
        .ok_or_else(|| format!("SHA256SUMS.txt not found in release {tag}"))?;

    println!("Downloading {asset_name} and SHA256SUMS.txt…");
    let sums_text = client.get(&sums_url).send().await?.text().await?;
    let expected_sha = parse_sha256sums(&sums_text, &asset_name)
        .ok_or_else(|| format!("Checksum entry for {asset_name} missing in SHA256SUMS.txt"))?;

    let tarball_bytes = client.get(&tarball_url).send().await?.bytes().await?;
    let actual_sha = hex::encode(Sha256::digest(&tarball_bytes));
    if actual_sha != expected_sha {
        return Err(format!(
            "SHA-256 mismatch for {asset_name}: expected {expected_sha}, got {actual_sha}"
        )
        .into());
    }
    println!("SHA-256 verified: {actual_sha}");

    let current_exe = std::env::current_exe()?;
    let exe_dir = current_exe
        .parent()
        .ok_or("Unable to determine executable parent directory")?;
    let stage_dir = std::env::temp_dir().join(format!("musick-update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&stage_dir);
    fs::create_dir_all(&stage_dir)?;
    let archive_path = stage_dir.join(&asset_name);
    fs::write(&archive_path, &tarball_bytes)?;

    let status = Command::new("tar")
        .args(["-xzf"])
        .arg(&archive_path)
        .arg("-C")
        .arg(&stage_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await?;
    if !status.success() {
        let _ = fs::remove_dir_all(&stage_dir);
        return Err(format!("tar extraction failed with status {status}").into());
    }

    let extracted_bin: PathBuf = stage_dir.join("musick");
    if !extracted_bin.is_file() {
        let _ = fs::remove_dir_all(&stage_dir);
        return Err("Archive did not contain expected 'musick' binary".into());
    }

    let staged_target = exe_dir.join(format!(".musick.new.{}", std::process::id()));
    if let Err(err) = fs::copy(&extracted_bin, &staged_target) {
        let _ = fs::remove_dir_all(&stage_dir);
        return Err(format!(
            "Cannot write to {} ({err}). If installed system-wide, update via your package manager.",
            exe_dir.display()
        )
        .into());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&staged_target, fs::Permissions::from_mode(0o755));
    }

    if let Err(err) = fs::rename(&staged_target, &current_exe) {
        let _ = fs::remove_file(&staged_target);
        let _ = fs::remove_dir_all(&stage_dir);
        return Err(format!("Atomic binary replacement failed: {err}").into());
    }

    let _ = fs::remove_dir_all(&stage_dir);
    println!("Updated musick to {tag} at {}", current_exe.display());
    Ok(())
}
