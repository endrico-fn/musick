"""Cava audio spectrum visualizer widget using clean async subprocess pipe streaming."""

from __future__ import annotations

import asyncio
import contextlib
import logging
import os
import re
import shutil
import subprocess
import tempfile
import time
from collections.abc import Sequence
from pathlib import Path

from rich.text import Text
from textual.events import Resize
from textual.message import Message
from textual.timer import Timer
from textual.widget import Widget

from musick.ui.theme import DIM

logger = logging.getLogger(__name__)

_BLOCKS = (" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█")
_OSC_RE = re.compile(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")
_ANSI_RE = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")

_GRADIENT_STOPS: tuple[tuple[float, tuple[int, int, int]], ...] = (
    (0.00, (13, 82, 71)),  # deep ocean teal base
    (0.20, (18, 118, 102)),  # dark jade
    (0.42, (25, 158, 134)),  # rich emerald
    (0.65, (38, 217, 180)),  # accent teal
    (0.84, (74, 236, 202)),  # luminous mint
    (1.00, (148, 250, 228)),  # silky aqua crest
)

_PALETTES: dict[str, tuple[tuple[float, tuple[int, int, int]], ...]] = {
    "teal": _GRADIENT_STOPS,
    "emerald": (
        (0.00, (14, 76, 48)),
        (0.25, (20, 118, 70)),
        (0.55, (29, 185, 84)),
        (0.82, (74, 224, 132)),
        (1.00, (152, 248, 186)),
    ),
    "aurora": (
        (0.00, (34, 62, 136)),
        (0.28, (30, 114, 178)),
        (0.56, (28, 174, 196)),
        (0.82, (58, 224, 192)),
        (1.00, (166, 252, 236)),
    ),
    "sunset": (
        (0.00, (98, 34, 86)),
        (0.26, (164, 54, 96)),
        (0.54, (224, 92, 82)),
        (0.80, (246, 156, 76)),
        (1.00, (255, 222, 148)),
    ),
    "mono": (
        (0.00, (48, 54, 66)),
        (0.30, (82, 92, 108)),
        (0.62, (128, 142, 162)),
        (0.85, (176, 190, 212)),
        (1.00, (224, 234, 248)),
    ),
}

_SMOOTHING_PROFILES: dict[str, tuple[int, int, float, float, float]] = {
    # name: (monstercat, noise_reduction, attack, release, spatial_feather)
    "smooth": (1, 62, 0.68, 0.24, 0.14),
    "balanced": (1, 52, 0.80, 0.34, 0.08),
    "crisp": (0, 42, 1.00, 0.42, 0.00),
}


def _smoothstep(t: float) -> float:
    t = max(0.0, min(1.0, t))
    return t * t * (3.0 - 2.0 * t)


def _lerp_rgb(c0: tuple[int, int, int], c1: tuple[int, int, int], t: float) -> tuple[int, int, int]:
    s = _smoothstep(t)
    return (
        round(c0[0] + (c1[0] - c0[0]) * s),
        round(c0[1] + (c1[1] - c0[1]) * s),
        round(c0[2] + (c1[2] - c0[2]) * s),
    )


def _gradient_hex(ratio: float, *, tip_boost: float = 0.0, palette: str = "teal") -> str:
    stops = _PALETTES.get(palette, _GRADIENT_STOPS)
    r = max(0.0, min(1.0, ratio))
    rgb = stops[-1][1]
    for idx in range(len(stops) - 1):
        p0, c0 = stops[idx]
        p1, c1 = stops[idx + 1]
        if r <= p1:
            local = (r - p0) / max(1e-6, p1 - p0)
            rgb = _lerp_rgb(c0, c1, local)
            break
    if tip_boost > 0.0:
        rgb = _lerp_rgb(rgb, (236, 255, 250), tip_boost)
    return f"#{rgb[0]:02x}{rgb[1]:02x}{rgb[2]:02x}"


def detect_cava_methods() -> list[str]:
    """Return preferred cava input methods based on available audio sockets."""
    rt = os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}"
    rt_path = Path(rt)
    methods: list[str] = []
    if (rt_path / "pipewire-0").exists():
        methods.append("pipewire")
    if (rt_path / "pulse" / "native").exists() or os.environ.get("PULSE_SERVER"):
        methods.append("pulse")
    for fallback in ("pipewire", "pulse"):
        if fallback not in methods:
            methods.append(fallback)
    return methods


def _default_pulse_monitor() -> str:
    """Resolve Pulse/PipeWire default output sink monitor name, falling back to 'auto'."""
    if shutil.which("pactl") is None:
        return "auto"
    with contextlib.suppress(OSError, subprocess.SubprocessError):
        res = subprocess.run(
            ["pactl", "get-default-sink"],
            capture_output=True,
            text=True,
            timeout=0.6,
        )
        sink = res.stdout.strip()
        if res.returncode == 0 and sink and sink != "@DEFAULT_SINK@":
            return f"{sink}.monitor"
    return "auto"


def resolve_cava_source(method: str, audio_source: str) -> str:
    """Map user-facing audio_source ('auto'|'monitor'|'off') to a valid cava [input] source."""
    if audio_source == "monitor" and method == "pulse":
        return _default_pulse_monitor()
    # PipeWire's cava backend uses 'auto' to monitor the active output sink;
    # passing literal 'monitor' creates target.object="monitor" which routes to the mic.
    return "auto"


def build_cava_config(
    bars: int,
    method: str = "pipewire",
    max_range: int = 100,
    framerate: int = 60,
    *,
    smoothing: str = "crisp",
    audio_source: str = "auto",
) -> str:
    """Build a raw ASCII stdout cava config with mono spectrum and calibrated smoothing."""
    even_bars = max(4, int(bars))
    if even_bars % 2 == 1:
        even_bars += 1
    monstercat, noise_reduction, _, _, _ = _SMOOTHING_PROFILES.get(
        smoothing, _SMOOTHING_PROFILES["crisp"]
    )
    resolved_src = resolve_cava_source(method, audio_source)
    input_block = f"method = {method}\nsource = {resolved_src}\n" if method else ""
    return (
        "[general]\n"
        f"bars = {even_bars}\n"
        f"framerate = {max(10, framerate)}\n"
        "autosens = 1\n"
        "lower_cutoff_freq = 40\n"
        "higher_cutoff_freq = 16000\n"
        "sleep_timer = 0\n\n"
        "[input]\n"
        f"{input_block}\n"
        "[output]\n"
        "method = raw\n"
        "raw_target = /dev/stdout\n"
        "data_format = ascii\n"
        f"ascii_max_range = {max_range}\n"
        "bar_delimiter = 59\n"
        "frame_delimiter = 10\n"
        "channels = mono\n"
        "mono_option = average\n\n"
        "[smoothing]\n"
        f"monstercat = {monstercat}\n"
        "waves = 0\n"
        f"noise_reduction = {noise_reduction}\n"
    )


def parse_cava_frame(line: str, max_range: int = 100) -> list[float] | None:
    """Parse one raw ASCII line from cava ('12;45;80;...') into normalized [0.0..1.0] floats."""
    cleaned = _OSC_RE.sub("", line)
    cleaned = _ANSI_RE.sub("", cleaned).strip()
    if not cleaned:
        return None
    parts = [p for p in cleaned.split(";") if p]
    if not parts:
        return None
    scale = float(max(1, max_range))
    vals: list[float] = []
    for p in parts:
        if not p.isdigit():
            return None
        vals.append(max(0.0, min(1.0, int(p) / scale)))
    return vals


def resample_bars(bars: Sequence[float], target_count: int) -> list[float]:
    """Resample a bar vector cleanly to target_count."""
    if target_count <= 0:
        return []
    if not bars:
        return [0.0] * target_count
    n = len(bars)
    if n == target_count:
        return list(bars)
    out: list[float] = []
    for i in range(target_count):
        pos = (i * (n - 1)) / max(1, target_count - 1) if target_count > 1 else 0.0
        lo = int(pos)
        hi = min(n - 1, lo + 1)
        frac = pos - lo
        out.append(bars[lo] * (1.0 - frac) + bars[hi] * frac)
    return out


_GRADIENT_LUT_SIZE = 128


def _gradient_lut(palette: str, *, tip: bool = False) -> tuple[str, ...]:
    boost = 0.16 if tip else 0.0
    return tuple(
        _gradient_hex(
            i / _GRADIENT_LUT_SIZE,
            tip_boost=boost * (i / _GRADIENT_LUT_SIZE),
            palette=palette,
        )
        for i in range(_GRADIENT_LUT_SIZE + 1)
    )


def _feather_bars(vals: list[float], weight: float) -> list[float]:
    """Apply subtle 3-tap spatial smoothing so adjacent frequency bars flow fluidly."""
    if weight <= 0.0 or len(vals) < 3:
        return vals
    center = 1.0 - 2.0 * weight
    edge_center = 1.0 - weight
    out = [edge_center * vals[0] + weight * vals[1]]
    out.extend(
        weight * vals[i - 1] + center * vals[i] + weight * vals[i + 1]
        for i in range(1, len(vals) - 1)
    )
    out.append(weight * vals[-2] + edge_center * vals[-1])
    return out


def render_spectrum(
    bars: Sequence[float],
    width: int,
    height: int,
    *,
    bar_width: int = 1,
    gap: int = 1,
    idle: bool = False,
    palette: str = "teal",
) -> Text:
    """Render vertical 8-sublevel block spectrum with continuous 2D TrueColor gradient."""
    w = max(8, width)
    h = max(2, height)
    step = bar_width + gap
    count = max(1, (w + gap) // step)
    vals = [max(0.0, min(1.0, v)) for v in resample_bars(bars, count)]

    total_used = count * bar_width + max(0, count - 1) * gap
    pad_left = max(0, (w - total_used) // 2)

    out = Text(no_wrap=True, overflow="ignore")
    max_units = h * 8
    lut = _gradient_lut(palette, tip=False)
    tip_lut = _gradient_lut(palette, tip=True)
    base_floor = lut[0]
    bar_units = [round(v * max_units) for v in vals]

    for row_from_top in range(h):
        if row_from_top > 0:
            out.append("\n")
        row_from_bottom = h - 1 - row_from_top
        cell_mid = (row_from_bottom + 0.5) / h

        if pad_left:
            out.append(" " * pad_left)

        for idx, (amp, units) in enumerate(zip(vals, bar_units, strict=True)):
            if idx > 0 and gap:
                out.append(" " * gap)
            cell_units = units - row_from_bottom * 8
            if row_from_bottom == 0 and cell_units <= 0:
                glyph = _BLOCKS[1]  # crisp ▁ baseline
                st = DIM if idle else base_floor
            elif cell_units <= 0:
                glyph = " "
                st = ""
            elif cell_units >= 8:
                glyph = _BLOCKS[8]
                is_top = (row_from_bottom + 1) * 8 >= units >= 6
                t_val = max(0.0, min(1.0, 0.78 * cell_mid + 0.22 * amp))
                b_idx = round(t_val * _GRADIENT_LUT_SIZE)
                st = tip_lut[b_idx] if is_top else lut[b_idx]
            else:
                glyph = _BLOCKS[cell_units]
                pos_t = (row_from_bottom + cell_units / 8.0) / h
                exact_t = max(0.0, min(1.0, 0.72 * pos_t + 0.28 * amp))
                b_idx = round(exact_t * _GRADIENT_LUT_SIZE)
                st = tip_lut[b_idx] if units >= 4 else lut[b_idx]
            out.append(glyph * bar_width, style=st)
    return out


class CavaVisualizer(Widget):
    """Real-time Cava audio visualizer with pipewire/pulse pipe capture & clean fallback."""

    class ModeChanged(Message):
        def __init__(self, mode: str) -> None:
            super().__init__()
            self.mode = mode

    DEFAULT_CSS = """
    CavaVisualizer {
        background: transparent;
        width: 1fr;
        height: 1fr;
    }
    """

    def __init__(
        self,
        *,
        enable_subprocess: bool | None = None,
        palette: str = "teal",
        smoothing: str = "smooth",
        enabled: bool = True,
        audio_source: str = "auto",
        **kw,
    ) -> None:
        super().__init__(**kw)
        self._has_cava = shutil.which("cava") is not None
        if enable_subprocess is None:
            enable_subprocess = "PYTEST_CURRENT_TEST" not in os.environ and self._has_cava
        self._enable_subprocess = enable_subprocess
        self._enabled = enabled
        self._palette = palette if palette in _PALETTES else "teal"
        self._smoothing = smoothing if smoothing in _SMOOTHING_PROFILES else "smooth"
        valid_src = ("auto", "monitor", "off")
        self._audio_source = audio_source if audio_source in valid_src else "auto"

        self._playing: bool = False
        self._paused: bool = False
        self._reduced_motion: bool = False
        self._active_visible: bool = True

        self._bars: list[float] = []
        self._cava_bars: list[float] = []
        self._cava_max_range: int = 100
        self._methods: list[str] = [*detect_cava_methods(), ""]
        self._last_frame_ts: float = 0.0
        self._last_tick_ts: float = 0.0
        self._cava_start_ts: float = 0.0
        self._method_idx: int = 0
        self._cava_failed: bool = False

        self._proc: asyncio.subprocess.Process | None = None
        self._conf_path: Path | None = None
        self._proc_bars: int = 0
        self._watch_task: asyncio.Task[None] | None = None
        self._anim_timer: Timer | None = None
        self._live_mode: bool = False
        self._last_mode_label: str = "idle"
        self._last_render_sig: tuple[object, ...] | None = None
        self._proc_has_frame: bool = False
        self._bg_tasks: set[asyncio.Task[None]] = set()

    @property
    def mode_label(self) -> str:
        if not self._enabled or self._audio_source == "off":
            return "off"
        if not self._playing or self._paused:
            return "idle"
        if self._reduced_motion:
            return "static"
        if self._live_mode:
            return "live"
        if not self._has_cava and self._enable_subprocess:
            return "cava missing"
        if (
            not self._cava_failed
            and self._cava_start_ts
            and (time.monotonic() - self._cava_start_ts) < 2.5
        ):
            return "connecting"
        return "no signal"

    def set_appearance(
        self,
        *,
        palette: str | None = None,
        smoothing: str | None = None,
        enabled: bool | None = None,
        audio_source: str | None = None,
    ) -> None:
        """Update Cava palette, smoothing curve, enabled toggle, or audio source live."""
        needs_restart = False
        changed = False
        if palette is not None and palette in _PALETTES and palette != self._palette:
            self._palette = palette
            changed = True
        if (
            smoothing is not None
            and smoothing in _SMOOTHING_PROFILES
            and smoothing != self._smoothing
        ):
            self._smoothing = smoothing
            needs_restart = True
            changed = True
        if enabled is not None and enabled != self._enabled:
            self._enabled = enabled
            changed = True
        if (
            audio_source is not None
            and audio_source in ("auto", "monitor", "off")
            and audio_source != self._audio_source
        ):
            self._audio_source = audio_source
            self._method_idx = 0
            self._cava_failed = False
            needs_restart = True
            changed = True
        if not changed:
            return
        self._last_render_sig = None
        if needs_restart and self._proc is not None:
            self._restart_cava()
        self._sync_activity()
        self.refresh()

    def _bar_geometry(self) -> tuple[int, int, int]:
        if self.size.width <= 0 and len(self._bars) >= 4:
            return len(self._bars), 1, 1
        w = max(12, self.size.width or 48)
        bar_w = 2 if w >= 96 else 1
        gap = 1
        count = max(8, (w + gap) // (bar_w + gap))
        if count > 8 and count % 2 == 1:
            count -= 1
        return count, bar_w, gap

    def set_playback_state(
        self,
        *,
        playing: bool,
        paused: bool,
        track_key: str = "",
        reduced_motion: bool = False,
    ) -> None:
        del track_key
        was_active = self._playing and not self._paused
        changed = (
            playing != self._playing
            or paused != self._paused
            or reduced_motion != self._reduced_motion
        )
        self._playing = playing
        self._paused = paused
        self._reduced_motion = reduced_motion
        if playing and not paused and not was_active and self._cava_failed:
            self._cava_failed = False
            self._method_idx = 0
        self._sync_activity()
        if changed:
            self._last_render_sig = None
            self.refresh()

    def set_active_visible(self, visible: bool) -> None:
        self._active_visible = visible
        if visible:
            if self._cava_failed:
                self._cava_failed = False
                self._method_idx = 0
            self._sync_activity()
        else:
            self._stop_timer()
            self._stop_cava()

    def on_mount(self) -> None:
        self._sync_activity()

    def on_unmount(self) -> None:
        self._stop_timer()
        self._stop_cava()

    def on_resize(self, event: Resize) -> None:
        del event
        count, _, _ = self._bar_geometry()
        if count != len(self._bars):
            self._bars = resample_bars(self._bars, count)
        if self._proc is not None and abs(count - self._proc_bars) >= 4:
            self._restart_cava()

    def _sync_activity(self) -> None:
        if not (self.is_mounted or self.is_attached):
            return
        disabled = not self._enabled or self._audio_source == "off"
        if not self._active_visible or disabled:
            self._stop_timer()
            self._stop_cava()
            if disabled:
                self._bars.clear()
                self.refresh()
            return
        active = self._playing and not self._paused
        if active and not self._reduced_motion:
            self._ensure_timer()
            if self._enable_subprocess and not self._cava_failed and self._proc is None:
                self._start_cava()
        else:
            self._stop_cava()
            if self._reduced_motion:
                self._stop_timer()
                count, _, _ = self._bar_geometry()
                self._bars = [0.22 for _ in range(count)] if active else [0.0] * count
                self.refresh()
            elif not self._bars or all(v < 0.01 for v in self._bars):
                self._stop_timer()
            else:
                self._ensure_timer()

    def _ensure_timer(self) -> None:
        if self._anim_timer is None and (self.is_mounted or self.is_attached):
            self._last_tick_ts = 0.0
            self._anim_timer = self.set_interval(1 / 60, self._on_anim_tick)

    def _stop_timer(self) -> None:
        if self._anim_timer is not None:
            self._anim_timer.stop()
            self._anim_timer = None
        self._last_tick_ts = 0.0

    def _on_anim_tick(self) -> None:
        if not self.display:
            return
        count, _, _ = self._bar_geometry()
        if len(self._bars) != count:
            self._bars = [0.0] * count

        active = self._playing and not self._paused
        now = time.monotonic()
        dt = (
            max(1 / 120, min(0.1, now - self._last_tick_ts))
            if self._last_tick_ts
            else (1 / 60)
        )
        self._last_tick_ts = now
        frame_scale = dt * 60.0

        if (
            active
            and not self._reduced_motion
            and self._proc is not None
            and self._proc.returncode is None
            and not self._proc_has_frame
            and self._cava_start_ts
            and (now - self._cava_start_ts) >= 2.2
            and (self._method_idx + 1) < len(self._methods)
        ):
            self._method_idx += 1
            self._restart_cava()
            return

        if active and self._cava_bars and (now - self._last_frame_ts) < 1.0:
            self._live_mode = True
            _, _, attack, release, feather = _SMOOTHING_PROFILES.get(
                self._smoothing, _SMOOTHING_PROFILES["smooth"]
            )
            target = _feather_bars(resample_bars(self._cava_bars, count), feather)
            for i in range(count):
                cur = self._bars[i]
                tgt = target[i]
                base = attack if tgt >= cur else release
                rate = 1.0 - (1.0 - min(1.0, base)) ** frame_scale
                self._bars[i] = cur + (tgt - cur) * rate
        else:
            self._live_mode = False
            decay = 0.72**frame_scale
            settled = True
            for i in range(count):
                self._bars[i] *= decay
                if self._bars[i] > 0.012:
                    settled = False
                else:
                    self._bars[i] = 0.0
            if settled and not active:
                self._stop_timer()

        cur_label = self.mode_label
        if cur_label != self._last_mode_label:
            self._last_mode_label = cur_label
            self.post_message(self.ModeChanged(cur_label))

        max_units = max(16, (self.size.height or 8) * 8)
        sig = (
            not active,
            self._enabled and self._audio_source != "off",
            self._palette,
            self.size.width,
            self.size.height,
            tuple(round(max(0.0, min(1.0, v)) * max_units) for v in self._bars),
        )
        if sig != self._last_render_sig:
            self._last_render_sig = sig
            self.refresh()

    def feed_raw_line(self, line: str, max_range: int | None = None) -> bool:
        """Parse one cava raw output line and update live bars."""
        vals = parse_cava_frame(line, max_range=max_range or self._cava_max_range)
        if vals is None:
            return False
        self._cava_bars = vals
        self._proc_has_frame = True
        self._last_frame_ts = time.monotonic()
        return True

    def _start_cava(self) -> None:
        if (
            self._proc is not None
            or (self._watch_task is not None and not self._watch_task.done())
            or self._cava_failed
            or self._method_idx >= len(self._methods)
        ):
            return
        self._has_cava = shutil.which("cava") is not None
        if not self._has_cava:
            self._cava_failed = True
            return
        try:
            loop = asyncio.get_running_loop()
        except RuntimeError:
            return

        count, _, _ = self._bar_geometry()
        self._proc_bars = count
        self._cava_max_range = 1000
        method = self._methods[self._method_idx]
        cfg = build_cava_config(
            count, method=method, max_range=1000,
            smoothing=self._smoothing, audio_source=self._audio_source,
        )

        try:
            fd, path_str = tempfile.mkstemp(prefix="musick_cava_", suffix=".conf")
            os.close(fd)
            conf_path = Path(path_str)
            conf_path.write_text(cfg, encoding="utf-8")
            self._conf_path = conf_path
        except OSError:
            self._cava_failed = True
            return

        self._proc_has_frame = False
        self._cava_start_ts = time.monotonic()
        self._watch_task = loop.create_task(self._run_cava(conf_path))

    def _restart_cava(self) -> None:
        self._stop_cava()
        if self._playing and not self._paused and self._enable_subprocess and not self._cava_failed:
            self._start_cava()

    async def _run_cava(self, conf_path: Path) -> None:
        # Yield once so caller finishes assigning `self._watch_task` under eager_task_factory.
        await asyncio.sleep(0)
        proc: asyncio.subprocess.Process | None = None
        my_task = asyncio.current_task()
        started_at = time.monotonic()
        try:
            proc = await asyncio.create_subprocess_exec(
                "cava",
                "-p",
                str(conf_path),
                stdin=asyncio.subprocess.DEVNULL,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.DEVNULL,
                start_new_session=True,
            )
            if self._watch_task is not my_task:
                with contextlib.suppress(ProcessLookupError):
                    proc.terminate()
                return
            self._proc = proc

            stdout = getattr(proc, "stdout", None)
            if stdout is not None:
                while self._watch_task is my_task:
                    raw = await stdout.readline()
                    if not raw:
                        break
                    self.feed_raw_line(raw.decode("utf-8", errors="ignore"))

            rc = await proc.wait()
            if rc != 0 and self._watch_task is my_task:
                if (
                    time.monotonic() - started_at < 2.0
                    and (self._method_idx + 1) < len(self._methods)
                    and self._playing
                    and not self._paused
                ):
                    self._method_idx += 1
                    self._watch_task = None
                    self._cleanup_cava_handles()
                    self._start_cava()
                    return
                self._cava_failed = True
        except asyncio.CancelledError:
            raise
        except (OSError, ValueError, RuntimeError):
            logger.debug("Cava subprocess loop exited with error", exc_info=True)
            if self._watch_task is my_task:
                self._cava_failed = True
        finally:
            if self._watch_task is my_task:
                self._cleanup_cava_handles()
            else:
                self._safe_unlink(conf_path)
            if proc is not None and proc.returncode is None:
                with contextlib.suppress(ProcessLookupError):
                    proc.terminate()

    @staticmethod
    def _safe_unlink(path: Path) -> None:
        with contextlib.suppress(OSError):
            path.unlink(missing_ok=True)

    def _cleanup_cava_handles(self) -> None:
        self._proc = None
        if self._conf_path is not None:
            with contextlib.suppress(OSError):
                self._conf_path.unlink(missing_ok=True)
            self._conf_path = None

    def _stop_cava(self) -> None:
        if self._watch_task and not self._watch_task.done():
            self._watch_task.cancel()
        self._watch_task = None
        proc = self._proc
        self._cleanup_cava_handles()
        if proc is not None and proc.returncode is None:
            with contextlib.suppress(ProcessLookupError):
                proc.terminate()
            with contextlib.suppress(RuntimeError):
                loop = asyncio.get_running_loop()
                task = loop.create_task(self._reap_proc(proc))
                self._bg_tasks.add(task)
                task.add_done_callback(self._bg_tasks.discard)
        self._cava_bars.clear()
        self._proc_has_frame = False
        self._live_mode = False

    @staticmethod
    async def _reap_proc(proc: asyncio.subprocess.Process) -> None:
        try:
            await asyncio.wait_for(proc.wait(), timeout=0.25)
        except TimeoutError:
            with contextlib.suppress(ProcessLookupError, AttributeError):
                proc.kill()
            with contextlib.suppress(OSError, RuntimeError):
                await proc.wait()
        except (OSError, RuntimeError):
            logger.debug("Cava reap error", exc_info=True)

    def render(self) -> Text:
        w = max(8, self.size.width or 48)
        h = max(2, self.size.height or 8)
        count, bar_w, gap = self._bar_geometry()
        vals = self._bars if len(self._bars) == count else resample_bars(self._bars, count)
        idle = not (
            self._playing
            and not self._paused
            and self._enabled
            and self._audio_source != "off"
        )
        return render_spectrum(
            vals, w, h, bar_width=bar_w, gap=gap, idle=idle, palette=self._palette
        )
