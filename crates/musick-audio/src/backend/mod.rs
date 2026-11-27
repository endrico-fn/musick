mod ipc;
pub mod mpv;

use std::fs;
use std::path::Path;

use async_trait::async_trait;
use tokio::sync::broadcast;

use musick_core::Settings;

use crate::error::AudioResult;

pub use mpv::MpvIpcBackend;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioProperties {
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub volume: u8,
    pub buffering: bool,
    pub core_idle: bool,
}

impl Default for AudioProperties {
    fn default() -> Self {
        Self {
            position: 0.0,
            duration: 0.0,
            paused: false,
            volume: 80,
            buffering: false,
            core_idle: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackendEvent {
    Properties(AudioProperties),
    Eof,
    FileError(String),
    Disconnected(String),
}

pub fn overdrive_limiter_af(vol: u8) -> String {
    let clamped = f64::from(vol.clamp(100, 130));
    let gain = (clamped / 100.0).powi(3);
    format!(
        "@musicklim:lavfi=[alimiter=level_in={gain:.4}:limit=0.98:attack=2:release=50:level=false]"
    )
}

pub fn normalize_af() -> &'static str {
    "@musicknorm:lavfi=[dynaudnorm=f=150:g=15:p=0.90]"
}

/// safely unlink a stale ipc socket, refusing symlinks and non-sockets
pub fn safe_unlink_socket(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if meta.file_type().is_symlink() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if !meta.file_type().is_socket() {
            return false;
        }
    }
    #[cfg(not(unix))]
    {
        return false;
    }
    fs::remove_file(path).is_ok()
}

#[async_trait]
pub trait AudioBackend: Send + Sync {
    async fn start(&self) -> AudioResult<()>;
    async fn stop(&self) -> AudioResult<()>;
    async fn set_user_agent(&self, _user_agent: Option<&str>) -> AudioResult<()> {
        Ok(())
    }
    async fn load_url(&self, url: &str) -> AudioResult<()>;
    async fn load_url_with_ua(&self, url: &str, user_agent: Option<&str>) -> AudioResult<()> {
        let _ = self.set_user_agent(user_agent).await;
        self.load_url(url).await
    }
    async fn append_gapless(&self, url: &str) -> AudioResult<()>;
    async fn append_gapless_with_ua(&self, url: &str, user_agent: Option<&str>) -> AudioResult<()> {
        if user_agent.is_some() {
            let _ = self.set_user_agent(user_agent).await;
        }
        self.append_gapless(url).await
    }
    async fn clear_gapless_tail(&self) -> AudioResult<()>;
    async fn stop_track(&self) -> AudioResult<()>;
    async fn toggle_pause(&self) -> AudioResult<bool>;
    async fn set_paused(&self, paused: bool) -> AudioResult<()>;
    async fn seek(&self, seconds: f64, relative: bool) -> AudioResult<()>;
    async fn set_volume(&self, volume: u8) -> AudioResult<u8>;
    async fn apply_settings(&self, settings: &Settings) -> AudioResult<()>;
    fn subscribe_events(&self) -> broadcast::Receiver<BackendEvent>;
    fn properties(&self) -> AudioProperties;
}
