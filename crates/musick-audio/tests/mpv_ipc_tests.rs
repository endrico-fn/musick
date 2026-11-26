#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

use musick_audio::{
    AudioBackend, BackendEvent, MpvIpcBackend, normalize_af, overdrive_limiter_af,
    safe_unlink_socket,
};
use musick_core::Settings;

#[test]
fn safe_unlink_socket_refuses_regular_files_and_symlinks_but_removes_real_sockets() {
    let dir = tempdir().unwrap();

    // 1. Regular file MUST NOT be deleted
    let regular = dir.path().join("important.txt");
    fs::write(&regular, "do not delete").unwrap();
    assert!(!safe_unlink_socket(&regular));
    assert!(regular.exists());

    // 2. Real Unix socket IS deleted
    let sock_path = dir.path().join("real.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    drop(listener);
    assert!(sock_path.exists());

    // 3. Symlink pointing to a socket MUST NOT be deleted
    let link_path = dir.path().join("link.sock");
    std::os::unix::fs::symlink(&sock_path, &link_path).unwrap();
    assert!(!safe_unlink_socket(&link_path));
    assert!(link_path.symlink_metadata().is_ok());
    assert!(sock_path.exists());

    // 4. Actual socket path IS unlinked cleanly
    assert!(safe_unlink_socket(&sock_path));
    assert!(!sock_path.exists());
}

#[test]
fn filter_strings_match_cubic_overdrive_and_dynaudnorm_spec() {
    let lim = overdrive_limiter_af(120);
    assert!(lim.starts_with("@musicklim:lavfi=[alimiter=level_in=1.7280:"));
    assert!(lim.contains("limit=0.98:attack=2:release=50:level=false"));

    assert_eq!(
        normalize_af(),
        "@musicknorm:lavfi=[dynaudnorm=f=150:g=15:p=0.90]"
    );
}

#[tokio::test]
async fn mpv_ipc_framing_observe_property_overdrive_normalize_and_gapless() {
    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("mock-mpv.sock");
    let listener = UnixListener::bind(&sock_path).unwrap();

    let received: Arc<Mutex<Vec<Vec<Value>>>> = Arc::new(Mutex::new(Vec::new()));
    let received_srv = received.clone();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        while reader.read_line(&mut line).await.unwrap() > 0 {
            let msg: Value = serde_json::from_str(line.trim()).unwrap();
            let rid = msg.get("request_id").and_then(Value::as_u64);
            if let Some(cmd) = msg.get("command").and_then(Value::as_array) {
                received_srv.lock().unwrap().push(cmd.clone());
                let cmd_name = cmd.first().and_then(Value::as_str).unwrap_or("");
                if let Some(id) = rid {
                    let reply = format!("{}\n", json!({"error": "success", "request_id": id}));
                    if write_half.write_all(reply.as_bytes()).await.is_err() {
                        break;
                    }
                }
                if cmd_name == "loadfile" && cmd.get(2).and_then(Value::as_str) == Some("replace") {
                    let ev1 = format!(
                        "{}\n",
                        json!({"event": "property-change", "name": "duration", "data": 215.5})
                    );
                    let ev2 = format!(
                        "{}\n",
                        json!({"event": "property-change", "name": "time-pos", "data": 12.25})
                    );
                    if write_half.write_all(ev1.as_bytes()).await.is_err()
                        || write_half.write_all(ev2.as_bytes()).await.is_err()
                    {
                        break;
                    }
                }
            }
            line.clear();
        }
    });

    let mut settings = Settings::default();
    settings.player.volume = 80;
    settings.player.normalize_audio = false;

    let backend = MpvIpcBackend::with_socket(sock_path, settings.clone(), false);
    let mut events = backend.subscribe_events();
    backend.start().await.unwrap();
    backend.load_url("https://example.invalid/audio1.opus").await.unwrap();
    backend.append_gapless("https://example.invalid/audio2.opus").await.unwrap();

    // Test overdrive > 100% attaches @musicklim + clamps mpv volume to 100
    assert_eq!(backend.set_volume(120).await.unwrap(), 120);
    // Drop back below 100% removes @musicklim
    assert_eq!(backend.set_volume(75).await.unwrap(), 75);

    // Hot-reload normalize_audio on -> adds @musicknorm live
    settings.player.volume = 75;
    settings.player.normalize_audio = true;
    backend.apply_settings(&settings).await.unwrap();

    // Synchronize on a round-trip command so all prior fire-and-forget commands are processed
    backend
        .send_command(
            vec![json!("get_property"), json!("volume")],
            true,
            Duration::from_secs(2),
        )
        .await
        .unwrap();

    // Wait briefly for property-change events from mock server
    let mut saw_time_pos = false;
    for _ in 0..10 {
        if let Ok(Ok(BackendEvent::Properties(p))) =
            tokio::time::timeout(Duration::from_millis(100), events.recv()).await
            && (p.position - 12.25).abs() < 0.01
        {
            saw_time_pos = true;
            break;
        }
    }
    assert!(saw_time_pos);
    assert!((backend.properties().duration - 215.5).abs() < 0.01);

    backend.stop().await.unwrap();
    let _ = tokio::time::timeout(Duration::from_millis(300), server).await;

    let cmds = received.lock().unwrap();
    assert!(cmds.iter().any(|c| c[0] == "observe_property" && c[2] == "time-pos"));
    assert!(cmds.iter().any(|c| c[0] == "loadfile" && c[2] == "replace"));
    assert!(cmds.iter().any(|c| c[0] == "loadfile" && c[2] == "append"));
    assert!(
        cmds.iter()
            .any(|c| c[0] == "af" && c[1] == "add" && c[2].as_str().unwrap_or("").contains("@musicklim"))
    );
    assert!(
        cmds.iter()
            .any(|c| c[0] == "af" && c[1] == "remove" && c[2] == "@musicklim")
    );
    assert!(
        cmds.iter()
            .any(|c| c[0] == "af" && c[1] == "add" && c[2].as_str().unwrap_or("").contains("@musicknorm"))
    );
}
