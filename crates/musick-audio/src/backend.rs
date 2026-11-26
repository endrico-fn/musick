use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio::task::JoinHandle;
use tracing::debug;

use musick_core::{AppPaths, Settings, ensure_private_dir};

use crate::error::{AudioError, AudioResult};

const OBSERVED_PROPERTIES: &[(u64, &str)] = &[
    (1, "time-pos"),
    (2, "duration"),
    (3, "pause"),
    (4, "volume"),
    (5, "paused-for-cache"),
    (6, "core-idle"),
];

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

/// Safely unlink a stale IPC socket. Refuses symlinks, regular files, and directories (Review Focus #5).
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
    async fn load_url(&self, url: &str) -> AudioResult<()>;
    async fn append_gapless(&self, url: &str) -> AudioResult<()>;
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

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<AudioResult<Value>>>>>;

struct RunningState {
    child: Option<Child>,
    writer: OwnedWriteHalf,
    read_task: JoinHandle<()>,
}

pub struct MpvIpcBackend {
    socket_path: PathBuf,
    spawn_process: bool,
    settings: RwLock<Settings>,
    props: Arc<RwLock<AudioProperties>>,
    target_volume: Arc<RwLock<u8>>,
    normalize_enabled: Mutex<bool>,
    running: Mutex<Option<RunningState>>,
    pending: PendingMap,
    req_id: AtomicU64,
    stopping: Arc<AtomicBool>,
    events_tx: broadcast::Sender<BackendEvent>,
}

impl MpvIpcBackend {
    pub fn new(paths: &AppPaths, settings: Settings) -> Self {
        Self::with_socket(paths.mpv_socket_path(), settings, true)
    }

    pub fn with_socket(socket_path: PathBuf, settings: Settings, spawn_process: bool) -> Self {
        let init_vol = settings.player.volume.clamp(0, 130);
        let init_norm = settings.player.normalize_audio;
        let (events_tx, _) = broadcast::channel(256);
        let props = AudioProperties {
            volume: init_vol,
            ..AudioProperties::default()
        };
        Self {
            socket_path,
            spawn_process,
            settings: RwLock::new(settings),
            props: Arc::new(RwLock::new(props)),
            target_volume: Arc::new(RwLock::new(init_vol)),
            normalize_enabled: Mutex::new(init_norm),
            running: Mutex::new(None),
            pending: Arc::new(Mutex::new(HashMap::new())),
            req_id: AtomicU64::new(10),
            stopping: Arc::new(AtomicBool::new(false)),
            events_tx,
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    fn current_settings(&self) -> Settings {
        self.settings.read().map(|g| g.clone()).unwrap_or_default()
    }

    async fn is_running_inner(&self) -> bool {
        let mut guard = self.running.lock().await;
        if let Some(ref mut st) = *guard {
            if let Some(ref mut child) = st.child
                && matches!(child.try_wait(), Ok(Some(_)))
            {
                return false;
            }
            return !st.read_task.is_finished();
        }
        false
    }

    pub async fn send_command(
        &self,
        cmd: Vec<Value>,
        wait: bool,
        timeout: Duration,
    ) -> AudioResult<Option<Value>> {
        let rid = self.req_id.fetch_add(1, Ordering::SeqCst) + 1;
        let rx = if wait {
            let (tx, rx) = oneshot::channel();
            self.pending.lock().await.insert(rid, tx);
            Some(rx)
        } else {
            None
        };

        let mut line = serde_json::to_string(&json!({
            "command": cmd,
            "request_id": rid,
        }))?;
        line.push('\n');

        let write_res = {
            let mut guard = self.running.lock().await;
            let Some(st) = guard.as_mut() else {
                if wait {
                    self.pending.lock().await.remove(&rid);
                }
                return Ok(None);
            };
            st.writer.write_all(line.as_bytes()).await
        };

        if let Err(err) = write_res {
            if wait {
                self.pending.lock().await.remove(&rid);
            }
            return Err(AudioError::Io(err));
        }

        if let Some(rx) = rx {
            match tokio::time::timeout(timeout, rx).await {
                Ok(Ok(res)) => Ok(Some(res?)),
                Ok(Err(_)) => Err(AudioError::Backend("mpv IPC channel closed".into())),
                Err(_) => {
                    self.pending.lock().await.remove(&rid);
                    Err(AudioError::Backend("mpv IPC command timed out".into()))
                }
            }
        } else {
            Ok(None)
        }
    }

    fn emit_props(&self) {
        let p = self.properties();
        let _ = self.events_tx.send(BackendEvent::Properties(p));
    }
}

#[async_trait]
impl AudioBackend for MpvIpcBackend {
    async fn start(&self) -> AudioResult<()> {
        if self.is_running_inner().await {
            return Ok(());
        }
        self.stop().await?;
        self.stopping.store(false, Ordering::SeqCst);

        let settings = self.current_settings();
        let target_vol = self.target_volume.read().map(|v| *v).unwrap_or(80);
        let child = if self.spawn_process {
            if let Some(parent) = self.socket_path.parent() {
                let _ = ensure_private_dir(parent);
            }
            safe_unlink_socket(&self.socket_path);

            let mpv_vol = target_vol.min(100);
            let mut cmd = Command::new("mpv");
            cmd.args([
                "--idle=yes",
                "--no-video",
                "--audio-display=no",
                "--audio-client-name=musick",
                "--really-quiet",
                "--ytdl=no",
                "--cache=yes",
                "--demuxer-max-bytes=128MiB",
                &format!("--demuxer-readahead-secs={}", settings.player.cache_secs),
                "--network-timeout=15",
                "--audio-channels=stereo",
                "--audio-pitch-correction=yes",
                "--volume-max=130",
                &format!("--volume={mpv_vol}"),
                &format!(
                    "--gapless-audio={}",
                    if settings.player.gapless { "yes" } else { "no" }
                ),
                &format!("--input-ipc-server={}", self.socket_path.display()),
            ]);

            let mut af_filters = Vec::new();
            if settings.player.normalize_audio {
                af_filters.push(normalize_af().to_string());
            }
            if target_vol > 100 {
                af_filters.push(overdrive_limiter_af(target_vol));
            }
            if !af_filters.is_empty() {
                cmd.arg(format!("--af={}", af_filters.join(",")));
            }
            if !settings.player.use_mpv_config {
                cmd.arg("--no-config");
            }
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);

            let mut spawned = cmd.spawn().map_err(|e| {
                AudioError::Backend(format!("Unable to launch mpv (is mpv in PATH?): {e}"))
            })?;

            let mut ready = false;
            for _ in 0..80 {
                if let Ok(Some(status)) = spawned.try_wait() {
                    return Err(AudioError::Backend(format!(
                        "mpv exited early with status {status}"
                    )));
                }
                if self.socket_path.exists() {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = fs::set_permissions(
                            &self.socket_path,
                            fs::Permissions::from_mode(0o600),
                        );
                    }
                    ready = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            if !ready {
                let _ = spawned.kill().await;
                safe_unlink_socket(&self.socket_path);
                return Err(AudioError::Backend(
                    "Timed out waiting for mpv IPC socket".into(),
                ));
            }
            Some(spawned)
        } else {
            None
        };

        let stream = UnixStream::connect(&self.socket_path).await.map_err(|e| {
            AudioError::Backend(format!(
                "Failed to connect to mpv IPC socket {}: {e}",
                self.socket_path.display()
            ))
        })?;
        let (read_half, write_half) = stream.into_split();

        let pending = self.pending.clone();
        let props = self.props.clone();
        let target_vol_arc = self.target_volume.clone();
        let stopping = self.stopping.clone();
        let events_tx = self.events_tx.clone();

        let read_task = tokio::spawn(async move {
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if let Ok(msg) = serde_json::from_str::<Value>(trimmed) {
                            handle_ipc_message(msg, &pending, &props, &target_vol_arc, &events_tx)
                                .await;
                        }
                    }
                    Err(err) => {
                        debug!("mpv IPC read loop terminated: {err}");
                        break;
                    }
                }
            }

            // Fail any outstanding requests
            let mut map = pending.lock().await;
            for (_, tx) in map.drain() {
                let _ = tx.send(Err(AudioError::Backend("mpv IPC disconnected".into())));
            }
            if !stopping.load(Ordering::SeqCst) {
                let _ = events_tx.send(BackendEvent::Disconnected(
                    "mpv IPC disconnected unexpectedly".into(),
                ));
            }
        });

        *self.running.lock().await = Some(RunningState {
            child,
            writer: write_half,
            read_task,
        });
        *self.normalize_enabled.lock().await = settings.player.normalize_audio;

        for &(obs_id, prop) in OBSERVED_PROPERTIES {
            let _ = self
                .send_command(
                    vec![json!("observe_property"), json!(obs_id), json!(prop)],
                    false,
                    Duration::from_secs(2),
                )
                .await;
        }
        Ok(())
    }

    async fn stop(&self) -> AudioResult<()> {
        self.stopping.store(true, Ordering::SeqCst);
        let state = self.running.lock().await.take();
        if let Some(st) = state {
            st.read_task.abort();
            drop(st.writer);
            if let Some(mut child) = st.child {
                let _ = child.start_kill();
                let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
            }
        }
        self.pending.lock().await.clear();
        if self.spawn_process {
            safe_unlink_socket(&self.socket_path);
        }
        self.stopping.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn load_url(&self, url: &str) -> AudioResult<()> {
        self.start().await?;
        if let Ok(mut p) = self.props.write() {
            p.position = 0.0;
            p.duration = 0.0;
            p.paused = false;
            p.buffering = true;
        }
        self.emit_props();
        let _ = self
            .send_command(
                vec![json!("set_property"), json!("pause"), json!(false)],
                false,
                Duration::from_secs(2),
            )
            .await;
        let _ = self
            .send_command(
                vec![json!("loadfile"), json!(url), json!("replace")],
                true,
                Duration::from_secs(3),
            )
            .await?;
        Ok(())
    }

    async fn append_gapless(&self, url: &str) -> AudioResult<()> {
        if !self.is_running_inner().await {
            return Ok(());
        }
        let _ = self
            .send_command(
                vec![json!("loadfile"), json!(url), json!("append")],
                false,
                Duration::from_secs(2),
            )
            .await?;
        Ok(())
    }

    async fn clear_gapless_tail(&self) -> AudioResult<()> {
        if !self.is_running_inner().await {
            return Ok(());
        }
        let _ = self
            .send_command(vec![json!("playlist-clear")], false, Duration::from_secs(2))
            .await?;
        Ok(())
    }

    async fn stop_track(&self) -> AudioResult<()> {
        if self.is_running_inner().await {
            let _ = self
                .send_command(vec![json!("stop")], false, Duration::from_secs(2))
                .await;
        }
        if let Ok(mut p) = self.props.write() {
            p.position = 0.0;
            p.duration = 0.0;
            p.buffering = false;
            p.core_idle = true;
        }
        self.emit_props();
        Ok(())
    }

    async fn toggle_pause(&self) -> AudioResult<bool> {
        let next = !self.properties().paused;
        self.set_paused(next).await?;
        Ok(next)
    }

    async fn set_paused(&self, paused: bool) -> AudioResult<()> {
        if let Ok(mut p) = self.props.write() {
            p.paused = paused;
        }
        if self.is_running_inner().await {
            let _ = self
                .send_command(
                    vec![json!("set_property"), json!("pause"), json!(paused)],
                    false,
                    Duration::from_secs(2),
                )
                .await;
        }
        self.emit_props();
        Ok(())
    }

    async fn seek(&self, seconds: f64, relative: bool) -> AudioResult<()> {
        if !self.is_running_inner().await {
            return Ok(());
        }
        let mode = if relative { "relative" } else { "absolute" };
        let _ = self
            .send_command(
                vec![json!("seek"), json!(seconds), json!(mode)],
                false,
                Duration::from_secs(2),
            )
            .await?;
        Ok(())
    }

    async fn set_volume(&self, volume: u8) -> AudioResult<u8> {
        let vol = volume.clamp(0, 130);
        let prev = {
            let mut guard = self
                .target_volume
                .write()
                .map_err(|_| AudioError::Backend("Volume lock poisoned".into()))?;
            let old = *guard;
            *guard = vol;
            old
        };
        if let Ok(mut p) = self.props.write() {
            p.volume = vol;
        }

        if self.is_running_inner().await {
            if vol > 100 {
                if prev > 100 {
                    let _ = self
                        .send_command(
                            vec![json!("af"), json!("remove"), json!("@musicklim")],
                            false,
                            Duration::from_secs(2),
                        )
                        .await;
                }
                let _ = self
                    .send_command(
                        vec![json!("set_property"), json!("volume"), json!(100)],
                        false,
                        Duration::from_secs(2),
                    )
                    .await;
                let _ = self
                    .send_command(
                        vec![json!("af"), json!("add"), json!(overdrive_limiter_af(vol))],
                        false,
                        Duration::from_secs(2),
                    )
                    .await;
            } else {
                if prev > 100 {
                    let _ = self
                        .send_command(
                            vec![json!("af"), json!("remove"), json!("@musicklim")],
                            false,
                            Duration::from_secs(2),
                        )
                        .await;
                }
                let _ = self
                    .send_command(
                        vec![json!("set_property"), json!("volume"), json!(vol)],
                        false,
                        Duration::from_secs(2),
                    )
                    .await;
            }
        }
        self.emit_props();
        Ok(vol)
    }

    async fn apply_settings(&self, settings: &Settings) -> AudioResult<()> {
        if let Ok(mut guard) = self.settings.write() {
            *guard = settings.clone();
        }
        // Hot-reload volume and dynamic normalization live without interrupting playback.
        // (`use_mpv_config`, `gapless`, and `cache_secs` apply on next restart/track.)
        let _ = self.set_volume(settings.player.volume).await?;
        if self.is_running_inner().await {
            let mut norm_guard = self.normalize_enabled.lock().await;
            let want = settings.player.normalize_audio;
            if want != *norm_guard {
                *norm_guard = want;
                if want {
                    let _ = self
                        .send_command(
                            vec![json!("af"), json!("add"), json!(normalize_af())],
                            false,
                            Duration::from_secs(2),
                        )
                        .await;
                } else {
                    let _ = self
                        .send_command(
                            vec![json!("af"), json!("remove"), json!("@musicknorm")],
                            false,
                            Duration::from_secs(2),
                        )
                        .await;
                }
            }
        }
        Ok(())
    }

    fn subscribe_events(&self) -> broadcast::Receiver<BackendEvent> {
        self.events_tx.subscribe()
    }

    fn properties(&self) -> AudioProperties {
        self.props.read().map(|p| *p).unwrap_or_default()
    }
}

impl Drop for MpvIpcBackend {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.running.try_lock()
            && let Some(mut st) = guard.take()
        {
            st.read_task.abort();
            if let Some(ref mut child) = st.child {
                let _ = child.start_kill();
            }
        }
        if self.spawn_process {
            safe_unlink_socket(&self.socket_path);
        }
    }
}

async fn handle_ipc_message(
    msg: Value,
    pending: &PendingMap,
    props: &Arc<RwLock<AudioProperties>>,
    target_vol: &Arc<RwLock<u8>>,
    events_tx: &broadcast::Sender<BackendEvent>,
) {
    if let Some(rid) = msg.get("request_id").and_then(Value::as_u64)
        && let Some(tx) = pending.lock().await.remove(&rid)
    {
        if msg.get("error").and_then(Value::as_str) == Some("success") {
            let _ = tx.send(Ok(msg.get("data").cloned().unwrap_or(Value::Null)));
        } else {
            let err_str = msg
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("mpv error")
                .to_string();
            let _ = tx.send(Err(AudioError::Backend(err_str)));
        }
    }

    let Some(ev) = msg.get("event").and_then(Value::as_str) else {
        return;
    };

    if ev == "property-change" {
        let name = msg.get("name").and_then(Value::as_str).unwrap_or("");
        let data = msg.get("data");
        let mut changed = false;
        let snapshot = if let Ok(mut p) = props.write() {
            match (name, data) {
                ("time-pos", Some(v)) if v.as_f64().is_some() => {
                    p.position = v.as_f64().unwrap_or(0.0).max(0.0);
                    changed = true;
                }
                ("duration", Some(v)) if v.as_f64().is_some() => {
                    p.duration = v.as_f64().unwrap_or(0.0).max(0.0);
                    changed = true;
                }
                ("pause", Some(Value::Bool(b))) => {
                    p.paused = *b;
                    changed = true;
                }
                ("volume", Some(v)) if v.as_f64().is_some() => {
                    let obs = v.as_f64().unwrap_or(80.0).round().clamp(0.0, 130.0) as u8;
                    let cur_target = target_vol.read().map(|t| *t).unwrap_or(80);
                    if !(cur_target > 100 && obs == 100) {
                        if let Ok(mut tg) = target_vol.write() {
                            *tg = obs;
                        }
                        p.volume = obs;
                        changed = true;
                    }
                }
                ("paused-for-cache", Some(Value::Bool(b))) => {
                    p.buffering = *b;
                    changed = true;
                }
                ("core-idle", Some(Value::Bool(b))) => {
                    p.core_idle = *b;
                    if !*b {
                        p.buffering = false;
                    }
                    changed = true;
                }
                _ => {}
            }
            *p
        } else {
            return;
        };

        if changed {
            let _ = events_tx.send(BackendEvent::Properties(snapshot));
        }
    } else if ev == "end-file" {
        let reason = msg.get("reason").and_then(Value::as_str).unwrap_or("");
        if reason == "eof" {
            let _ = events_tx.send(BackendEvent::Eof);
        } else if reason == "error" {
            let file_err = msg
                .get("file_error")
                .and_then(Value::as_str)
                .unwrap_or("Playback error")
                .to_string();
            let _ = events_tx.send(BackendEvent::FileError(file_err));
        }
    }
}
