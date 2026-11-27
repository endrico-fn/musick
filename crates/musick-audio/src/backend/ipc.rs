use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::process::Child;
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio::task::JoinHandle;
use tracing::debug;

use crate::backend::{AudioProperties, BackendEvent};
use crate::error::{AudioError, AudioResult};

pub(super) const OBSERVED_PROPERTIES: &[(u64, &str)] = &[
    (1, "time-pos"),
    (2, "duration"),
    (3, "pause"),
    (4, "volume"),
    (5, "paused-for-cache"),
    (6, "core-idle"),
];

pub(super) type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<AudioResult<Value>>>>>;

pub(super) struct RunningState {
    pub child: Option<Child>,
    pub writer: OwnedWriteHalf,
    pub read_task: JoinHandle<()>,
}

pub(super) fn spawn_ipc_reader(
    read_half: OwnedReadHalf,
    pending: PendingMap,
    props: Arc<RwLock<AudioProperties>>,
    target_vol: Arc<RwLock<u8>>,
    stopping: Arc<AtomicBool>,
    events_tx: broadcast::Sender<BackendEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
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
                        handle_ipc_message(msg, &pending, &props, &target_vol, &events_tx).await;
                    }
                }
                Err(err) => {
                    debug!("mpv IPC read loop terminated: {err}");
                    break;
                }
            }
        }

        let mut map = pending.lock().await;
        for (_, tx) in map.drain() {
            let _ = tx.send(Err(AudioError::Backend("mpv IPC disconnected".into())));
        }
        if !stopping.load(Ordering::SeqCst) {
            let _ = events_tx.send(BackendEvent::Disconnected(
                "mpv IPC disconnected unexpectedly".into(),
            ));
        }
    })
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
        if reason == "eof" || reason == "error" {
            if let Ok(mut p) = props.write() {
                p.buffering = false;
                p.core_idle = true;
                let snap = *p;
                drop(p);
                let _ = events_tx.send(BackendEvent::Properties(snap));
            }
            if reason == "eof" {
                let _ = events_tx.send(BackendEvent::Eof);
            } else {
                let file_err = msg
                    .get("file_error")
                    .and_then(Value::as_str)
                    .unwrap_or("Playback error")
                    .to_string();
                let _ = events_tx.send(BackendEvent::FileError(file_err));
            }
        }
    }
}
