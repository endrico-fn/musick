"""Fast async yt-dlp stream URL resolver with expire-aware TTL cache and prefetch."""

from __future__ import annotations

import asyncio
import logging
import shutil
import sys
import time
import urllib.parse
from collections import OrderedDict
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path

from musick.config import AUDIO_QUALITIES, Settings
from musick.domain import Track

from .process import communicate_capped

logger = logging.getLogger(__name__)

MAX_RESOLVED_STREAMS = 200

_QUALITY_PROFILES: dict[str, tuple[str, str]] = {
    "best": (
        "bestaudio[format_id!*=drc][acodec^=opus]/bestaudio[format_id!*=drc]/bestaudio/best",
        "lang,quality,acodec:opus,abr,asr",
    ),
    "balanced": (
        "bestaudio[format_id!*=drc][abr<=145]/bestaudio[format_id!*=drc]/bestaudio/best",
        "lang,+abr:128,acodec:opus",
    ),
    "saver": (
        "worstaudio[abr>=48]/bestaudio[abr<=80]/bestaudio/best",
        "lang,+abr:64",
    ),
}


def ytdlp_command() -> list[str]:
    """Return executable prefix for yt-dlp (PATH binary, sibling venv, or `python -m yt_dlp`)."""
    exe = shutil.which("yt-dlp")
    if exe:
        return ["yt-dlp"]
    sibling = Path(sys.executable).with_name("yt-dlp")
    if sibling.is_file():
        return [str(sibling)]
    return [sys.executable, "-m", "yt_dlp"]


@dataclass(frozen=True, slots=True)
class ResolvedStream:
    video_id: str
    url: str
    expires_at: float
    codec: str = ""
    bitrate: int = 0
    sample_rate: int = 0
    quality: str = "best"

    @property
    def quality_label(self) -> str:
        parts: list[str] = []
        if self.codec and self.codec.lower() not in ("none", "na"):
            c = "aac" if self.codec.startswith("mp4a") else self.codec.split(".")[0]
            parts.append(c)
        if self.bitrate > 0:
            parts.append(f"{self.bitrate}k")
        if self.quality == "saver":
            parts.append("saver")
        return " ".join(parts)


def parse_stream_expiry(url: str, now: float | None = None, default_ttl: float = 3600.0) -> float:
    """Extract googlevideo 'expire=' unix timestamp minus safety margin, else now + default_ttl."""
    now_ts = time.time() if now is None else now
    try:
        qs = urllib.parse.parse_qs(urllib.parse.urlparse(url).query)
        exp_raw = qs.get("expire", [None])[0]
        if exp_raw and exp_raw.isdigit():
            exp = float(int(exp_raw))
            if exp > now_ts:
                return max(now_ts + 30.0, exp - 120.0)
    except ValueError:
        logger.debug("Unable to parse expiry from stream URL", exc_info=True)
    return now_ts + default_ttl


class StreamResolver:
    """Resolve direct audio stream URLs via yt-dlp --no-cookies with quality-aware TTL cache."""

    def __init__(
        self,
        clock: Callable[[], float] = time.time,
        runner: Callable[[str], Awaitable[str | tuple[str, str, int, int]]] | None = None,
        settings: Settings | None = None,
    ) -> None:
        self._clock = clock
        self._runner = runner
        self._settings = settings
        self._quality = (
            settings.player.audio_quality
            if settings and settings.player.audio_quality in AUDIO_QUALITIES
            else "best"
        )
        self._cache: OrderedDict[str, ResolvedStream] = OrderedDict()
        self._inflight: dict[str, asyncio.Task[ResolvedStream]] = {}
        self._bg_tasks: set[asyncio.Task[None]] = set()

    @property
    def audio_quality(self) -> str:
        if self._settings is not None:
            q = self._settings.player.audio_quality
            if q in AUDIO_QUALITIES:
                return q
        return self._quality

    def set_quality(self, quality: str) -> None:
        norm = quality if quality in AUDIO_QUALITIES else "best"
        if norm != self._quality:
            self._quality = norm
            self._cache.clear()

    def get_cached(self, video_id: str) -> ResolvedStream | None:
        hit = self._cache.get(video_id)
        if not hit:
            return None
        if hit.quality != self.audio_quality or self._clock() >= hit.expires_at:
            del self._cache[video_id]
            return None
        self._cache.move_to_end(video_id)
        return hit

    def invalidate(self, video_id: str) -> None:
        self._cache.pop(video_id, None)

    def clear_cache(self) -> None:
        self._cache.clear()

    async def resolve(self, track: Track | str) -> ResolvedStream:
        vid = track.video_id if isinstance(track, Track) else track
        cached = self.get_cached(vid)
        if cached is not None:
            return cached

        task = self._inflight.get(vid)
        if task is None:
            task = asyncio.create_task(self._resolve_uncached(vid))
            self._inflight[vid] = task
            task.add_done_callback(
                lambda t, k=vid: self._inflight.pop(k, None) if self._inflight.get(k) is t else None
            )
        return await asyncio.shield(task)

    def prefetch(self, track: Track | str | None) -> None:
        if not track:
            return
        vid = track.video_id if isinstance(track, Track) else track
        if self.get_cached(vid) or vid in self._inflight:
            return

        async def _bg() -> None:
            try:
                await self.resolve(vid)
            except (OSError, TimeoutError, ValueError, RuntimeError):
                logger.debug("Prefetch failed for %s", vid, exc_info=True)

        task = asyncio.create_task(_bg())
        self._bg_tasks.add(task)
        task.add_done_callback(self._bg_tasks.discard)

    async def _resolve_uncached(self, video_id: str) -> ResolvedStream:
        quality = self.audio_quality
        codec = ""
        bitrate = 0
        sample_rate = 0
        if self._runner is not None:
            res = await self._runner(video_id)
            if isinstance(res, tuple):
                url, codec, bitrate, sample_rate = res
            else:
                url = str(res)
        else:
            url, codec, bitrate, sample_rate = await self._run_ytdlp_rich(video_id, quality=quality)
        stream = ResolvedStream(
            video_id=video_id,
            url=url,
            expires_at=parse_stream_expiry(url, now=self._clock()),
            codec=codec,
            bitrate=bitrate,
            sample_rate=sample_rate,
            quality=quality,
        )
        self._remember(video_id, stream)
        return stream

    def _remember(self, video_id: str, stream: ResolvedStream) -> None:
        self._cache[video_id] = stream
        self._cache.move_to_end(video_id)
        while len(self._cache) > MAX_RESOLVED_STREAMS:
            self._cache.popitem(last=False)

    @staticmethod
    async def _run_ytdlp_rich(video_id: str, *, quality: str = "best") -> tuple[str, str, int, int]:
        target = f"https://music.youtube.com/watch?v={video_id}"
        fmt, sort_spec = _QUALITY_PROFILES.get(quality, _QUALITY_PROFILES["best"])
        cmd = [
            *ytdlp_command(),
            "--no-cookies",
            "--no-playlist",
            "--no-warnings",
            "--quiet",
            "-f",
            fmt,
            "--format-sort",
            sort_spec,
            "--print",
            "%(acodec)s\t%(abr)s\t%(asr)s",
            "-g",
            target,
        ]
        try:
            proc = await asyncio.create_subprocess_exec(
                *cmd,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
        except FileNotFoundError as exc:
            raise RuntimeError("yt-dlp executable not found in PATH") from exc

        stdout, stderr = await communicate_capped(proc, timeout=20.0)
        if proc.returncode != 0:
            err = (
                stderr.decode("utf-8", errors="ignore").strip() or f"yt-dlp exit {proc.returncode}"
            )
            raise RuntimeError(err)
        lines = [
            ln.strip() for ln in stdout.decode("utf-8", errors="ignore").splitlines() if ln.strip()
        ]
        if not lines:
            raise RuntimeError("yt-dlp returned empty stream URL")

        codec = ""
        bitrate = 0
        sample_rate = 0
        url = lines[0]
        if "\t" in lines[0] and len(lines) >= 2:
            parts = lines[0].split("\t")
            codec = "" if parts[0] in ("NA", "none", "None") else parts[0]
            try:
                bitrate = (
                    round(float(parts[1]))
                    if len(parts) > 1 and parts[1] not in ("NA", "none", "None")
                    else 0
                )
            except ValueError:
                bitrate = 0
            try:
                sample_rate = (
                    int(float(parts[2]))
                    if len(parts) > 2 and parts[2] not in ("NA", "none", "None")
                    else 0
                )
            except ValueError:
                sample_rate = 0
            url = lines[1]
        return url, codec, bitrate, sample_rate
