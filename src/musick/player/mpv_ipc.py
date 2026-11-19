"""Persistent async mpv IPC controller with observe_property (no polling)."""

from __future__ import annotations

import asyncio
import contextlib
import json
import logging
import os
import stat
import tempfile
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from musick.config import Settings

logger = logging.getLogger(__name__)


def _default_socket_path() -> Path:
    """Place the IPC socket in a private runtime dir (never a predictable /tmp path)."""
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    if runtime:
        base = Path(runtime) / "musick"
        try:
            base.mkdir(parents=True, exist_ok=True, mode=0o700)
            base.chmod(0o700)
        except OSError:
            pass
        return base / f"mpv-{os.getpid()}.sock"
    base = Path(tempfile.mkdtemp(prefix="musick-mpv-"))
    return base / f"mpv-{os.getpid()}.sock"


@dataclass
class MpvProperties:
    position: float = 0.0
    duration: float = 0.0
    paused: bool = False
    volume: int = 80
    buffering: bool = False
    core_idle: bool = True


_OBSERVED = (
    (1, "time-pos"),
    (2, "duration"),
    (3, "pause"),
    (4, "volume"),
    (5, "paused-for-cache"),
    (6, "core-idle"),
)


def _overdrive_limiter_af(vol: int) -> str:
    """Build a labeled lavfi lookahead limiter carrying cubic gain above 100% with zero clipping."""
    gain = (max(100, min(130, vol)) / 100.0) ** 3
    return (
        f"@musicklim:lavfi=[alimiter=level_in={gain:.4f}:"
        "limit=0.98:attack=2:release=50:level=false]"
    )


def _normalize_af() -> str:
    """Build a labeled low-latency dynamic loudness normalizer filter."""
    return "@musicknorm:lavfi=[dynaudnorm=f=150:g=15:p=0.90]"


class MpvController:
    """Manages one persistent mpv instance via Unix socket JSON IPC."""

    def __init__(
        self,
        settings: Settings,
        socket_path: str | Path | None = None,
        spawn_process: bool = True,
        on_update: Callable[[MpvProperties], Any] | None = None,
        on_eof: Callable[[], Awaitable[Any] | Any] | None = None,
        on_error: Callable[[str], Awaitable[Any] | Any] | None = None,
    ) -> None:
        self._settings = settings
        self._socket_path = str(socket_path or _default_socket_path())
        self._spawn_process = spawn_process
        self._on_update = on_update
        self._on_eof = on_eof
        self._on_error = on_error

        init_vol = max(0, min(130, int(settings.player.volume)))
        self._target_volume = init_vol
        self._normalize_enabled = bool(settings.player.normalize_audio)
        self.props = MpvProperties(volume=init_vol)
        self._proc: asyncio.subprocess.Process | None = None
        self._reader: asyncio.StreamReader | None = None
        self._writer: asyncio.StreamWriter | None = None
        self._read_task: asyncio.Task[None] | None = None
        self._req_id = 10
        self._pending: dict[int, asyncio.Future[Any]] = {}
        self._bg_tasks: set[asyncio.Task[Any]] = set()
        self._write_lock = asyncio.Lock()
        self._started = False
        self._stopping = False

    @property
    def is_running(self) -> bool:
        if not self._started:
            return False
        if self._spawn_process and (self._proc is None or self._proc.returncode is not None):
            return False
        return self._writer is not None and not self._writer.is_closing()

    def bind_callbacks(
        self,
        *,
        on_update: Callable[[MpvProperties], Any] | None = None,
        on_eof: Callable[[], Awaitable[Any] | Any] | None = None,
        on_error: Callable[[str], Awaitable[Any] | Any] | None = None,
    ) -> None:
        """Attach engine callbacks if not already provided at construction."""
        if self._on_update is None and on_update is not None:
            self._on_update = on_update
        if self._on_eof is None and on_eof is not None:
            self._on_eof = on_eof
        if self._on_error is None and on_error is not None:
            self._on_error = on_error

    @staticmethod
    def _unlink_socket(path_str: str) -> None:
        """Remove a stale IPC socket, but never an unrelated file or symlink."""
        path = Path(path_str)
        try:
            if path.is_symlink() or not path.exists():
                return
            if not stat.S_ISSOCK(path.stat().st_mode):
                return
            path.unlink()
        except OSError:
            pass

    @staticmethod
    def _socket_exists(path_str: str) -> bool:
        return Path(path_str).exists()

    async def start(self) -> None:
        if self.is_running:
            return
        await self.stop()
        self._stopping = False

        if self._spawn_process:
            await asyncio.to_thread(self._unlink_socket, self._socket_path)
            mpv_vol = min(self._target_volume, 100)
            args = [
                "mpv",
                "--idle=yes",
                "--no-video",
                "--audio-display=no",
                "--audio-client-name=musick",
                "--really-quiet",
                "--ytdl=no",
                "--cache=yes",
                "--demuxer-max-bytes=128MiB",
                f"--demuxer-readahead-secs={self._settings.player.cache_secs}",
                "--network-timeout=15",
                "--audio-channels=stereo",
                "--audio-pitch-correction=yes",
                "--volume-max=130",
                f"--volume={mpv_vol}",
                f"--gapless-audio={'yes' if self._settings.player.gapless else 'no'}",
                f"--input-ipc-server={self._socket_path}",
            ]
            af_filters: list[str] = []
            if self._settings.player.normalize_audio:
                af_filters.append(_normalize_af())
            if self._target_volume > 100:
                af_filters.append(_overdrive_limiter_af(self._target_volume))
            if af_filters:
                args.append(f"--af={','.join(af_filters)}")
            if not self._settings.player.use_mpv_config:
                args.append("--no-config")
            try:
                self._proc = await asyncio.create_subprocess_exec(
                    *args,
                    stdout=asyncio.subprocess.DEVNULL,
                    stderr=asyncio.subprocess.DEVNULL,
                )
            except FileNotFoundError as exc:
                raise RuntimeError("mpv executable not found in PATH") from exc

            for _ in range(80):
                if self._proc.returncode is not None:
                    raise RuntimeError(f"mpv exited early with code {self._proc.returncode}")
                if await asyncio.to_thread(self._socket_exists, self._socket_path):
                    with contextlib.suppress(OSError):
                        await asyncio.to_thread(os.chmod, self._socket_path, 0o600)
                    break
                await asyncio.sleep(0.03)
            else:
                await self.stop()
                raise RuntimeError("Timed out waiting for mpv IPC socket")

        self._reader, self._writer = await asyncio.open_unix_connection(self._socket_path)
        self._started = True
        self._read_task = asyncio.create_task(self._read_loop())

        for obs_id, prop in _OBSERVED:
            await self.command(["observe_property", obs_id, prop], wait=False)

    async def stop(self) -> None:
        self._stopping = True
        self._started = False
        if self._read_task:
            self._read_task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await self._read_task
            self._read_task = None

        for fut in self._pending.values():
            if not fut.done():
                fut.cancel()
        self._pending.clear()

        for t in list(self._bg_tasks):
            if not t.done():
                t.cancel()
        self._bg_tasks.clear()

        if self._writer:
            self._writer.close()
            with contextlib.suppress(OSError, RuntimeError):
                await self._writer.wait_closed()
        self._reader = None
        self._writer = None

        if self._proc and self._proc.returncode is None:
            self._proc.terminate()
            try:
                await asyncio.wait_for(self._proc.wait(), timeout=1.0)
            except TimeoutError:
                self._proc.kill()
                with contextlib.suppress(OSError, RuntimeError):
                    await self._proc.wait()
        self._proc = None

        if self._spawn_process:
            await asyncio.to_thread(self._unlink_socket, self._socket_path)
        self._stopping = False

    async def command(self, cmd: list[Any], wait: bool = True, timeout: float = 2.0) -> Any:
        if not self.is_running or self._writer is None:
            return None
        self._req_id += 1
        rid = self._req_id
        fut: asyncio.Future[Any] | None = None
        if wait:
            fut = asyncio.get_running_loop().create_future()
            self._pending[rid] = fut
        payload = (json.dumps({"command": cmd, "request_id": rid}) + "\n").encode("utf-8")
        try:
            async with self._write_lock:
                self._writer.write(payload)
                await self._writer.drain()
            if fut is not None:
                return await asyncio.wait_for(fut, timeout=timeout)
        except Exception:
            self._pending.pop(rid, None)
            raise
        return None

    async def load_url(self, url: str) -> None:
        await self.start()
        self.props.position = 0.0
        self.props.duration = 0.0
        self.props.paused = False
        self.props.buffering = True
        await self.command(["set_property", "pause", False], wait=False)
        await self.command(["loadfile", url, "replace"])

    async def stop_track(self) -> None:
        if self.is_running:
            await self.command(["stop"], wait=False)
        self.props.position = 0.0
        self.props.duration = 0.0
        self.props.buffering = False
        self.props.core_idle = True

    async def toggle_pause(self) -> bool:
        if not self.is_running:
            return self.props.paused
        self.props.paused = not self.props.paused
        await self.command(["set_property", "pause", self.props.paused], wait=False)
        self._notify()
        return self.props.paused

    async def seek(self, seconds: float, relative: bool = True) -> None:
        if not self.is_running:
            return
        mode = "relative" if relative else "absolute"
        await self.command(["seek", seconds, mode], wait=False)

    async def set_volume(self, volume: int) -> int:
        prev = self._target_volume
        vol = max(0, min(130, int(volume)))
        self._target_volume = vol
        self.props.volume = vol
        if self.is_running:
            if vol > 100:
                if prev > 100:
                    with contextlib.suppress(OSError, RuntimeError, TimeoutError):
                        await self.command(["af", "remove", "@musicklim"], wait=False)
                with contextlib.suppress(OSError, RuntimeError, TimeoutError):
                    await self.command(["set_property", "volume", 100], wait=False)
                    await self.command(["af", "add", _overdrive_limiter_af(vol)], wait=False)
            else:
                if prev > 100:
                    with contextlib.suppress(OSError, RuntimeError, TimeoutError):
                        await self.command(["af", "remove", "@musicklim"], wait=False)
                with contextlib.suppress(OSError, RuntimeError, TimeoutError):
                    await self.command(["set_property", "volume", vol], wait=False)
        self._notify()
        return vol

    def apply_settings(self, settings: Settings) -> None:
        self._settings = settings
        if self.is_running:
            with contextlib.suppress(RuntimeError):
                task = asyncio.create_task(self._sync_live_settings())
                self._bg_tasks.add(task)
                task.add_done_callback(self._bg_tasks.discard)

    async def _sync_live_settings(self) -> None:
        if not self.is_running:
            return
        with contextlib.suppress(OSError, RuntimeError, TimeoutError):
            await self.command(
                ["set_property", "gapless-audio", "yes" if self._settings.player.gapless else "no"],
                wait=False,
            )
            await self.command(
                ["set_property", "demuxer-readahead-secs", self._settings.player.cache_secs],
                wait=False,
            )
            want_norm = bool(self._settings.player.normalize_audio)
            if want_norm != self._normalize_enabled:
                self._normalize_enabled = want_norm
                if want_norm:
                    await self.command(["af", "add", _normalize_af()], wait=False)
                else:
                    await self.command(["af", "remove", "@musicknorm"], wait=False)

    def _notify(self) -> None:
        if self._on_update:
            self._on_update(self.props)

    async def _read_loop(self) -> None:
        assert self._reader is not None
        try:
            while True:
                raw = await self._reader.readline()
                if not raw:
                    break
                line = raw.decode("utf-8", errors="ignore").strip()
                if not line:
                    continue
                try:
                    msg = json.loads(line)
                except json.JSONDecodeError:
                    continue
                self._handle_message(msg)
        except asyncio.CancelledError:
            raise
        except (OSError, RuntimeError, ValueError):
            logger.debug("mpv IPC read loop ended", exc_info=True)
        finally:
            was_running = self._started
            self._started = False
            for fut in self._pending.values():
                if not fut.done():
                    fut.set_exception(ConnectionError("mpv IPC disconnected"))
            self._pending.clear()
            if was_running and not self._stopping and self._on_error is not None:
                try:
                    res = self._on_error("mpv process exited unexpectedly")
                except Exception:
                    logger.debug("mpv exit callback failed", exc_info=True)
                    res = None
                if asyncio.iscoroutine(res):
                    task = asyncio.create_task(res)
                    self._bg_tasks.add(task)
                    task.add_done_callback(self._bg_tasks.discard)

    def _handle_message(self, msg: dict[str, Any]) -> None:
        rid = msg.get("request_id")
        if rid is not None and rid in self._pending:
            fut = self._pending.pop(rid)
            if not fut.done():
                if msg.get("error") == "success":
                    fut.set_result(msg.get("data"))
                else:
                    fut.set_exception(RuntimeError(str(msg.get("error", "mpv error"))))

        ev = msg.get("event")
        if ev == "property-change":
            name = msg.get("name")
            data = msg.get("data")
            changed = False
            if name == "time-pos" and isinstance(data, (int, float)):
                self.props.position = max(0.0, float(data))
                changed = True
            elif name == "duration" and isinstance(data, (int, float)):
                self.props.duration = max(0.0, float(data))
                changed = True
            elif name == "pause" and isinstance(data, bool):
                self.props.paused = data
                changed = True
            elif name == "volume" and isinstance(data, (int, float)):
                observed = round(data)
                if not (self._target_volume > 100 and observed == 100):
                    self._target_volume = max(0, min(130, observed))
                    self.props.volume = self._target_volume
                    changed = True
            elif name == "paused-for-cache" and isinstance(data, bool):
                self.props.buffering = data
                changed = True
            elif name == "core-idle" and isinstance(data, bool):
                self.props.core_idle = data
                if not data:
                    self.props.buffering = False
                changed = True
            if changed:
                self._notify()
        elif ev == "end-file":
            reason = msg.get("reason")
            if reason == "eof" and self._on_eof:
                res = self._on_eof()
                if asyncio.iscoroutine(res):
                    task = asyncio.create_task(res)
                    self._bg_tasks.add(task)
                    task.add_done_callback(self._bg_tasks.discard)
            elif reason == "error" and self._on_error:
                err_txt = str(msg.get("file_error") or "Playback error")
                res = self._on_error(err_txt)
                if asyncio.iscoroutine(res):
                    task = asyncio.create_task(res)
                    self._bg_tasks.add(task)
                    task.add_done_callback(self._bg_tasks.discard)
