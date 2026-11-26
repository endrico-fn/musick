use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::debug;

use musick_core::{CavaAudioSource, CavaSmoothing};

#[derive(Debug, Clone, PartialEq)]
pub struct CavaFrame {
    pub bars: Vec<f32>,
    pub mode_label: &'static str,
    pub live: bool,
}

impl Default for CavaFrame {
    fn default() -> Self {
        Self {
            bars: Vec::new(),
            mode_label: "idle",
            live: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CavaControl {
    pub enabled: bool,
    pub playing: bool,
    pub paused: bool,
    pub visible: bool,
    pub reduced_motion: bool,
    pub bar_count: usize,
    pub smoothing: CavaSmoothing,
    pub audio_source: CavaAudioSource,
}

impl Default for CavaControl {
    fn default() -> Self {
        Self {
            enabled: true,
            playing: false,
            paused: false,
            visible: true,
            reduced_motion: false,
            bar_count: 32,
            smoothing: CavaSmoothing::Smooth,
            audio_source: CavaAudioSource::Auto,
        }
    }
}

pub fn smoothing_params(s: CavaSmoothing) -> (u8, u8, f32, f32, f32) {
    // (monstercat, noise_reduction, attack, release, spatial_feather)
    match s {
        CavaSmoothing::Smooth => (1, 62, 0.68, 0.24, 0.14),
        CavaSmoothing::Balanced => (1, 52, 0.80, 0.34, 0.08),
        CavaSmoothing::Crisp => (0, 42, 1.00, 0.42, 0.00),
    }
}

pub fn detect_cava_methods() -> Vec<&'static str> {
    let rt = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/user/1000"));
    let mut methods = Vec::new();
    if rt.join("pipewire-0").exists() {
        methods.push("pipewire");
    }
    if rt.join("pulse").join("native").exists() || std::env::var_os("PULSE_SERVER").is_some() {
        methods.push("pulse");
    }
    for fb in ["pipewire", "pulse"] {
        if !methods.contains(&fb) {
            methods.push(fb);
        }
    }
    methods.push("");
    methods
}

fn default_pulse_monitor() -> String {
    let output = std::process::Command::new("pactl")
        .arg("get-default-sink")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    if let Ok(out) = output
        && out.status.success()
    {
        let sink = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !sink.is_empty() && sink != "@DEFAULT_SINK@" {
            return format!("{sink}.monitor");
        }
    }
    "auto".to_string()
}

pub fn resolve_cava_source(method: &str, audio_source: CavaAudioSource) -> String {
    if audio_source == CavaAudioSource::Monitor && method == "pulse" {
        return default_pulse_monitor();
    }
    "auto".to_string()
}

pub fn build_cava_config(
    bars: usize,
    method: &str,
    max_range: u32,
    framerate: u32,
    smoothing: CavaSmoothing,
    audio_source: CavaAudioSource,
) -> String {
    let mut even_bars = bars.max(4);
    if !even_bars.is_multiple_of(2) {
        even_bars += 1;
    }
    let (monstercat, noise_reduction, _, _, _) = smoothing_params(smoothing);
    let input_block = if method.is_empty() {
        String::new()
    } else {
        let src = resolve_cava_source(method, audio_source);
        format!("method = {method}\nsource = {src}\n")
    };
    format!(
        "[general]\n\
bars = {even_bars}\n\
framerate = {}\n\
autosens = 1\n\
lower_cutoff_freq = 40\n\
higher_cutoff_freq = 16000\n\
sleep_timer = 0\n\n\
[input]\n\
{input_block}\n\
[output]\n\
method = raw\n\
raw_target = /dev/stdout\n\
data_format = ascii\n\
ascii_max_range = {max_range}\n\
bar_delimiter = 59\n\
frame_delimiter = 10\n\
channels = mono\n\
mono_option = average\n\n\
[smoothing]\n\
monstercat = {monstercat}\n\
waves = 0\n\
noise_reduction = {noise_reduction}\n",
        framerate.max(10)
    )
}

pub fn parse_cava_frame(line: &str, max_range: u32) -> Option<Vec<f32>> {
    let cleaned = strip_ansi(line);
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }
    let scale = max_range.max(1) as f32;
    let mut vals = Vec::new();
    for part in trimmed.split(';').filter(|p| !p.is_empty()) {
        let n: u32 = part.parse().ok()?;
        vals.push((n as f32 / scale).clamp(0.0, 1.0));
    }
    if vals.is_empty() { None } else { Some(vals) }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    let _ = chars.next();
                    for ch in chars.by_ref() {
                        if ch.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    let _ = chars.next();
                    while let Some(ch) = chars.next() {
                        if ch == '\x07' || (ch == '\x1b' && chars.peek() == Some(&'\\')) {
                            if ch == '\x1b' {
                                let _ = chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn resample_bars(bars: &[f32], target_count: usize) -> Vec<f32> {
    if target_count == 0 {
        return Vec::new();
    }
    if bars.is_empty() {
        return vec![0.0; target_count];
    }
    if bars.len() == target_count {
        return bars.to_vec();
    }
    let n = bars.len();
    let mut out = Vec::with_capacity(target_count);
    for i in 0..target_count {
        let pos = if target_count > 1 {
            (i * (n - 1)) as f32 / (target_count - 1) as f32
        } else {
            0.0
        };
        let lo = (pos as usize).min(n - 1);
        let hi = (lo + 1).min(n - 1);
        let frac = pos - lo as f32;
        out.push(bars[lo] * (1.0 - frac) + bars[hi] * frac);
    }
    out
}

pub fn feather_bars(vals: &[f32], weight: f32) -> Vec<f32> {
    if weight <= 0.0 || vals.len() < 3 {
        return vals.to_vec();
    }
    let center = 1.0 - 2.0 * weight;
    let edge = 1.0 - weight;
    let mut out = Vec::with_capacity(vals.len());
    out.push(edge * vals[0] + weight * vals[1]);
    for i in 1..vals.len() - 1 {
        out.push(weight * vals[i - 1] + center * vals[i] + weight * vals[i + 1]);
    }
    out.push(weight * vals[vals.len() - 2] + edge * vals[vals.len() - 1]);
    out
}

#[derive(Clone)]
pub struct CavaRunnerHandle {
    ctrl_tx: mpsc::UnboundedSender<CavaMsg>,
    pub frame_rx: watch::Receiver<CavaFrame>,
}

impl CavaRunnerHandle {
    pub fn update(&self, ctrl: CavaControl) {
        let _ = self.ctrl_tx.send(CavaMsg::Control(ctrl));
    }

    pub fn shutdown(&self) {
        let _ = self.ctrl_tx.send(CavaMsg::Shutdown);
    }

    pub fn frame(&self) -> CavaFrame {
        self.frame_rx.borrow().clone()
    }
}

enum CavaMsg {
    Control(CavaControl),
    Shutdown,
}

pub struct CavaRunner;

impl CavaRunner {
    pub fn spawn(initial: CavaControl) -> (CavaRunnerHandle, JoinHandle<()>) {
        let (ctrl_tx, ctrl_rx) = mpsc::unbounded_channel();
        let (frame_tx, frame_rx) = watch::channel(CavaFrame::default());
        let handle = CavaRunnerHandle { ctrl_tx, frame_rx };
        let join = tokio::spawn(run_cava_task(initial, ctrl_rx, frame_tx));
        (handle, join)
    }
}

fn has_cava_binary() -> bool {
    std::process::Command::new("cava")
        .arg("-v")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

struct ActiveCava {
    child: Child,
    conf_path: PathBuf,
    reader: BufReader<tokio::process::ChildStdout>,
    started_at: Instant,
    got_frame: bool,
}

impl ActiveCava {
    async fn stop(mut self) {
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_millis(300), self.child.wait()).await;
        let _ = std::fs::remove_file(&self.conf_path);
    }
}

async fn run_cava_task(
    mut ctrl: CavaControl,
    mut ctrl_rx: mpsc::UnboundedReceiver<CavaMsg>,
    frame_tx: watch::Sender<CavaFrame>,
) {
    let cava_present = has_cava_binary();
    let methods = detect_cava_methods();
    let mut method_idx = 0usize;
    let mut active_proc: Option<ActiveCava> = None;
    let mut smooth_bars: Vec<f32> = Vec::new();
    let mut line_buf = String::new();

    loop {
        let disabled = !ctrl.enabled || ctrl.audio_source == CavaAudioSource::Off;
        let want_live =
            !disabled && ctrl.visible && ctrl.playing && !ctrl.paused && !ctrl.reduced_motion;

        if !want_live {
            if let Some(proc) = active_proc.take() {
                proc.stop().await;
            }
            let label = if disabled {
                "off"
            } else if !ctrl.playing || ctrl.paused {
                "idle"
            } else if ctrl.reduced_motion {
                "static"
            } else {
                "idle"
            };
            let bars = if ctrl.reduced_motion && ctrl.playing && !ctrl.paused && !disabled {
                vec![0.22; ctrl.bar_count.max(8)]
            } else {
                Vec::new()
            };
            smooth_bars.clear();
            let _ = frame_tx.send(CavaFrame {
                bars,
                mode_label: label,
                live: false,
            });

            // Sleep completely idle (zero CPU!) until control state changes
            match ctrl_rx.recv().await {
                Some(CavaMsg::Control(next)) => {
                    if next.audio_source != ctrl.audio_source {
                        method_idx = 0;
                    }
                    ctrl = next;
                }
                Some(CavaMsg::Shutdown) | None => break,
            }
            continue;
        }

        if !cava_present {
            let _ = frame_tx.send(CavaFrame {
                bars: Vec::new(),
                mode_label: "cava missing",
                live: false,
            });
            match ctrl_rx.recv().await {
                Some(CavaMsg::Control(next)) => ctrl = next,
                Some(CavaMsg::Shutdown) | None => break,
            }
            continue;
        }

        if active_proc.is_none() && method_idx < methods.len() {
            let method = methods[method_idx];
            match spawn_cava_process(
                ctrl.bar_count.max(8),
                method,
                ctrl.smoothing,
                ctrl.audio_source,
            ) {
                Ok(proc) => {
                    active_proc = Some(proc);
                    let _ = frame_tx.send(CavaFrame {
                        bars: smooth_bars.clone(),
                        mode_label: "connecting",
                        live: false,
                    });
                }
                Err(err) => {
                    debug!("Failed spawning cava with method '{method}': {err}");
                    method_idx += 1;
                    continue;
                }
            }
        }

        let Some(ref mut proc) = active_proc else {
            let _ = frame_tx.send(CavaFrame {
                bars: Vec::new(),
                mode_label: "no signal",
                live: false,
            });
            match ctrl_rx.recv().await {
                Some(CavaMsg::Control(next)) => {
                    if next.audio_source != ctrl.audio_source
                        || (next.playing && !ctrl.playing)
                    {
                        method_idx = 0;
                    }
                    ctrl = next;
                }
                Some(CavaMsg::Shutdown) | None => break,
            }
            continue;
        };

        line_buf.clear();
        tokio::select! {
            msg = ctrl_rx.recv() => {
                match msg {
                    Some(CavaMsg::Control(next)) => {
                        let restart = next.smoothing != ctrl.smoothing
                            || next.audio_source != ctrl.audio_source
                            || next.bar_count.abs_diff(ctrl.bar_count) >= 4;
                        if next.audio_source != ctrl.audio_source {
                            method_idx = 0;
                        }
                        ctrl = next;
                        if restart && let Some(p) = active_proc.take() {
                            p.stop().await;
                        }
                    }
                    Some(CavaMsg::Shutdown) | None => {
                        if let Some(p) = active_proc.take() {
                            p.stop().await;
                        }
                        break;
                    }
                }
            }
            read_res = tokio::time::timeout(Duration::from_millis(500), proc.reader.read_line(&mut line_buf)) => {
                match read_res {
                    Ok(Ok(0)) => {
                        // EOF -> try next input method
                        if let Some(p) = active_proc.take() {
                            p.stop().await;
                        }
                        method_idx += 1;
                    }
                    Ok(Ok(_)) => {
                        if let Some(raw_vals) = parse_cava_frame(&line_buf, 1000) {
                            proc.got_frame = true;
                            let target_n = ctrl.bar_count.max(8);
                            let (_, _, attack, release, feather) = smoothing_params(ctrl.smoothing);
                            let target = feather_bars(&resample_bars(&raw_vals, target_n), feather);
                            if smooth_bars.len() != target_n {
                                smooth_bars = vec![0.0; target_n];
                            }
                            for (cur, tgt) in smooth_bars.iter_mut().zip(target.iter()) {
                                let rate = if *tgt >= *cur { attack } else { release };
                                *cur += (*tgt - *cur) * rate;
                            }
                            let _ = frame_tx.send(CavaFrame {
                                bars: smooth_bars.clone(),
                                mode_label: "live",
                                live: true,
                            });
                        }
                    }
                    Ok(Err(_)) => {
                        if let Some(p) = active_proc.take() {
                            p.stop().await;
                        }
                        method_idx += 1;
                    }
                    Err(_) => {
                        // Timeout without frames: switch method if > 2.2s since start with no frame
                        if !proc.got_frame && proc.started_at.elapsed() >= Duration::from_millis(2200) {
                            if let Some(p) = active_proc.take() {
                                p.stop().await;
                            }
                            method_idx += 1;
                        }
                    }
                }
            }
        }
    }
}

fn spawn_cava_process(
    bars: usize,
    method: &str,
    smoothing: CavaSmoothing,
    audio_source: CavaAudioSource,
) -> std::io::Result<ActiveCava> {
    let cfg = build_cava_config(bars, method, 1000, 30, smoothing, audio_source);
    let tmp = tempfile::Builder::new()
        .prefix("musick_cava_")
        .suffix(".conf")
        .tempfile()?;
    let (_, conf_path) = tmp.keep().map_err(|e| e.error)?;
    std::fs::write(&conf_path, cfg)?;

    let mut child = Command::new("cava")
        .arg("-p")
        .arg(&conf_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;

    let Some(stdout) = child.stdout.take() else {
        let _ = std::fs::remove_file(&conf_path);
        return Err(std::io::Error::other("cava stdout unavailable"));
    };

    Ok(ActiveCava {
        child,
        conf_path,
        reader: BufReader::new(stdout),
        started_at: Instant::now(),
        got_frame: false,
    })
}
