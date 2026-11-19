"""High-level playback coordinator uniting Queue, StreamResolver, MpvController, and Lyrics."""

from __future__ import annotations

import asyncio
import logging
import time
from collections.abc import Awaitable, Callable, Iterable, Sequence
from dataclasses import dataclass, field

import httpx

from musick.config import Settings
from musick.domain import Lyrics, Track
from musick.services.catalog import CATALOG_ERRORS
from musick.services.lyrics import LyricsService, active_line_index
from musick.services.resolver import StreamResolver
from musick.services.search import HistoryStore

from .mpv_ipc import MpvController, MpvProperties
from .queue import PlayQueue, RepeatMode

logger = logging.getLogger(__name__)

# mpv reports time-pos far faster than a terminal can use. Discrete changes
# (pause/volume/duration/buffering) bypass this; position-only updates are capped.
POSITION_EMIT_INTERVAL = 0.1


@dataclass
class PlaybackState:
    track: Track | None = None
    playing: bool = False
    paused: bool = False
    buffering: bool = False
    resolving: bool = False
    restored: bool = False
    position: float = 0.0
    duration: float = 0.0
    volume: int = 80
    shuffle: bool = False
    repeat: RepeatMode = "off"
    autoplay_active: bool = False
    normalize_audio: bool = False
    upcoming_count: int = 0
    lyrics: Lyrics = field(default_factory=Lyrics)
    active_lyric_idx: int | None = None
    show_translation: bool = False
    translating_lyrics: bool = False
    audio_info: str = ""
    error: str | None = None
    revision: int = 0
    track_rev: int = 0
    queue_rev: int = 0

    @property
    def active_lyric_text(self) -> str:
        if self.active_lyric_idx is not None and 0 <= self.active_lyric_idx < len(
            self.lyrics.lines
        ):
            return self.lyrics.lines[self.active_lyric_idx].text
        if self.lyrics.lines and not self.lyrics.synced:
            return self.lyrics.lines[0].text
        return ""


class PlayerEngine:
    def __init__(
        self,
        settings: Settings,
        resolver: StreamResolver,
        lyrics: LyricsService,
        history: HistoryStore,
        mpv: MpvController | None = None,
        queue: PlayQueue | None = None,
        radio_fn: Callable[[str], Awaitable[Sequence[Track]]] | None = None,
    ) -> None:
        self._settings = settings
        self._resolver = resolver
        self._lyrics = lyrics
        self._history = history
        self._radio_fn = radio_fn
        self.queue = queue or PlayQueue()
        self.mpv = mpv or MpvController(
            settings,
            on_update=self._on_mpv_update,
            on_eof=self._on_eof,
            on_error=self._on_mpv_error,
        )
        # Ensure callbacks are attached even if a custom MpvController was injected without them
        self.mpv.bind_callbacks(
            on_update=self._on_mpv_update,
            on_eof=self._on_eof,
            on_error=self._on_mpv_error,
        )

        self.state = PlaybackState(
            volume=settings.player.volume,
            normalize_audio=settings.player.normalize_audio,
            show_translation=settings.lyrics.translate_to_id,
        )
        self._listeners: list[Callable[[PlaybackState], None]] = []
        self._play_gen = 0
        self._loaded_vid: str | None = None
        self._lyric_task: asyncio.Task[None] | None = None
        self._autoplay_task: asyncio.Task[None] | None = None
        self._autoplay_seed_vid: str | None = None
        self._last_emit_ts: float = 0.0
        self._queue_sig: tuple[object, ...] = ()

    def subscribe(self, fn: Callable[[PlaybackState], None]) -> Callable[[], None]:
        self._listeners.append(fn)
        return lambda: self._listeners.remove(fn) if fn in self._listeners else None

    def _emit(self) -> None:
        self.state.revision += 1
        for fn in list(self._listeners):
            fn(self.state)

    def _sync_queue_flags(self) -> None:
        self.state.shuffle = self.queue.shuffle
        self.state.repeat = self.queue.repeat
        self.state.upcoming_count = len(self.queue.upcoming_tracks)
        self.state.normalize_audio = self._settings.player.normalize_audio
        sig = (
            len(self.queue.tracks),
            self.queue.index,
            self.queue.shuffle,
            self.queue.repeat,
            self.state.upcoming_count,
        )
        if sig != self._queue_sig:
            self._queue_sig = sig
            self.state.queue_rev += 1

    def notify_queue_changed(self) -> None:
        self._sync_queue_flags()
        self.state.queue_rev += 1
        self._prefetch_next()
        self._emit()

    def refresh_audio_settings(self) -> None:
        """Sync audio settings (quality, normalization, autoplay) after SettingsUpdated."""
        self._resolver.set_quality(self._settings.player.audio_quality)
        self._sync_queue_flags()
        self._prefetch_next()
        self._emit()

    def refresh_lyrics(self) -> None:
        """Re-fetch lyrics for the active track (e.g. after changing lyrics settings)."""
        self.state.show_translation = self._settings.lyrics.translate_to_id
        if self.state.track is None:
            self._emit()
            return
        if not self._settings.lyrics.enabled:
            if self._lyric_task and not self._lyric_task.done():
                self._lyric_task.cancel()
            self.state.lyrics = Lyrics()
            self.state.active_lyric_idx = None
            self.state.translating_lyrics = False
            self._emit()
            return
        self._launch_lyrics_fetch(self.state.track, self._play_gen)

    async def toggle_lyrics_translation(self) -> bool:
        """Toggle Indonesian lyrics translation on/off and translate active lyrics on demand."""
        self._settings.lyrics.translate_to_id = not self._settings.lyrics.translate_to_id
        enabled = self._settings.lyrics.translate_to_id
        self.state.show_translation = enabled

        if (
            not enabled
            or self.state.track is None
            or not self.state.lyrics
            or self.state.lyrics.translation_lang == "id"
        ):
            self.state.translating_lyrics = False
            self._emit()
            return enabled

        gen = self._play_gen
        vid = self.state.track.video_id
        lyr = self.state.lyrics
        self.state.translating_lyrics = True
        self._emit()
        try:
            translated = await self._lyrics.translate_lyrics(lyr, video_id=vid, target_lang="id")
            if gen == self._play_gen:
                self.state.lyrics = translated
        finally:
            if gen == self._play_gen:
                self.state.translating_lyrics = False
                self._emit()
        return enabled

    async def play_tracks(self, tracks: Iterable[Track], start: int = 0) -> None:
        self._autoplay_seed_vid = None
        self.state.autoplay_active = False
        chosen = self.queue.set_tracks(tracks, start=start)
        self._sync_queue_flags()
        if chosen:
            await self._start_track(chosen)

    async def play_track(self, track: Track) -> None:
        await self.play_tracks([track], start=0)

    def enqueue(self, tracks: Iterable[Track]) -> None:
        self.queue.append(tracks)
        self._sync_queue_flags()
        self._prefetch_next()
        self._emit()

    def play_next_in_queue(self, track: Track | Iterable[Track]) -> None:
        if isinstance(track, Track):
            self.queue.play_next(track)
        else:
            self.queue.play_next_many(track)
        self._sync_queue_flags()
        self._prefetch_next()
        self._emit()

    async def jump_to(self, index: int) -> None:
        track = self.queue.jump(index)
        self._sync_queue_flags()
        if track:
            await self._start_track(track)

    async def next_track(self) -> None:
        track = self.queue.advance(manual=True)
        self._sync_queue_flags()
        if track:
            await self._start_track(track)

    async def prev_track(self) -> None:
        if self.state.position > 4.0 and self.state.track is not None:
            await self.seek_absolute(0.0)
            return
        track = self.queue.retreat()
        self._sync_queue_flags()
        if track:
            await self._start_track(track)

    def restore_last_track(self, track: Track, *, fetch_lyrics: bool = True) -> None:
        """Display the last-played track in idle/ready state without starting mpv."""
        self._play_gen += 1
        gen = self._play_gen
        self._loaded_vid = None
        self.queue.set_tracks([track], start=0)
        self._sync_queue_flags()
        self.state.track = track
        self.state.track_rev += 1
        self.state.playing = False
        self.state.paused = False
        self.state.buffering = False
        self.state.resolving = False
        self.state.restored = True
        self.state.position = 0.0
        self.state.duration = float(track.duration or 0)
        self.state.lyrics = Lyrics()
        self.state.active_lyric_idx = None
        self.state.translating_lyrics = False
        self.state.audio_info = ""
        self.state.error = None
        self._emit()
        if fetch_lyrics and self._settings.lyrics.enabled:
            try:
                asyncio.get_running_loop()
                self._launch_lyrics_fetch(track, gen)
            except RuntimeError:
                pass

    def clear_restored(self) -> None:
        """Clear a restored (not actively playing) last track, e.g. after wiping history."""
        if not self.state.restored or self.state.playing or self.state.resolving:
            return
        self._play_gen += 1
        self._loaded_vid = None
        self.queue.clear()
        self._sync_queue_flags()
        self.state.track = None
        self.state.track_rev += 1
        self.state.restored = False
        self.state.position = 0.0
        self.state.duration = 0.0
        self.state.lyrics = Lyrics()
        self.state.active_lyric_idx = None
        self.state.translating_lyrics = False
        self.state.audio_info = ""
        self.state.error = None
        self._emit()

    async def toggle_pause(self) -> None:
        if self.state.track is None or self.state.resolving:
            return
        if self._loaded_vid != self.state.track.video_id:
            await self._start_track(self.state.track)
            return
        paused = await self.mpv.toggle_pause()
        self.state.paused = paused
        self.state.playing = not paused
        self._emit()

    async def stop(self) -> None:
        self._play_gen += 1
        self._loaded_vid = None
        await self.mpv.stop_track()
        self.state.playing = False
        self.state.paused = False
        self.state.buffering = False
        self.state.resolving = False
        self.state.restored = False
        self.state.position = 0.0
        self.state.active_lyric_idx = None
        self.state.translating_lyrics = False
        self.state.audio_info = ""
        self._emit()

    async def shutdown(self) -> None:
        if self._lyric_task and not self._lyric_task.done():
            self._lyric_task.cancel()
        if self._autoplay_task and not self._autoplay_task.done():
            self._autoplay_task.cancel()
        await self.mpv.stop()

    async def seek(self, delta: float) -> None:
        await self.mpv.seek(delta, relative=True)

    async def seek_absolute(self, seconds: float) -> None:
        if self.state.track is None or self.state.restored:
            return
        target = max(0.0, seconds)
        await self.mpv.seek(target, relative=False)
        self.state.position = target
        self._update_active_lyric()
        self._emit()

    async def change_volume(self, delta: int) -> int:
        return await self.set_volume(self.state.volume + delta)

    async def set_volume(self, vol: int) -> int:
        actual = await self.mpv.set_volume(vol)
        self.state.volume = actual
        self._emit()
        return actual

    def toggle_shuffle(self) -> bool:
        res = self.queue.toggle_shuffle()
        self._sync_queue_flags()
        self._prefetch_next()
        self._emit()
        return res

    def cycle_repeat(self) -> RepeatMode:
        res = self.queue.cycle_repeat()
        self._sync_queue_flags()
        self._prefetch_next()
        self._emit()
        return res

    def _prefetch_next(self) -> None:
        if self._settings.player.prefetch:
            self._resolver.prefetch(self.queue.peek_next())

    async def _start_track(self, track: Track) -> None:
        self._play_gen += 1
        gen = self._play_gen
        self._loaded_vid = None

        self.state.track = track
        self.state.track_rev += 1
        self.state.playing = True
        self.state.paused = False
        self.state.resolving = True
        self.state.buffering = True
        self.state.restored = False
        self.state.position = 0.0
        self.state.duration = float(track.duration or 0)
        self.state.lyrics = Lyrics()
        self.state.active_lyric_idx = None
        self.state.translating_lyrics = False
        self.state.audio_info = ""
        self.state.error = None
        self._emit()

        self._history.record_track(track)
        self._launch_lyrics_fetch(track, gen)

        try:
            resolved = await self._resolver.resolve(track)
            if gen != self._play_gen:
                return
            await self.mpv.load_url(resolved.url)
            if gen != self._play_gen:
                return
            self._loaded_vid = track.video_id
            self.state.resolving = False
            self.state.audio_info = resolved.quality_label
            self._prefetch_next()
            self._emit()
        except (OSError, TimeoutError, ValueError, RuntimeError) as exc:
            if gen != self._play_gen:
                return
            self._loaded_vid = None
            self.state.resolving = False
            self.state.buffering = False
            self.state.playing = False
            self.state.error = str(exc)
            self._emit()

    def _launch_lyrics_fetch(self, track: Track, gen: int) -> None:
        if self._lyric_task and not self._lyric_task.done():
            self._lyric_task.cancel()
        self.state.translating_lyrics = bool(self._settings.lyrics.translate_to_id)

        async def _job() -> None:
            try:
                lyr = await self._lyrics.fetch(track)
                if gen != self._play_gen:
                    return
                self.state.lyrics = lyr
                self.state.translating_lyrics = False
                self._update_active_lyric()
                self._emit()
            except asyncio.CancelledError:
                raise
            except (*CATALOG_ERRORS, httpx.HTTPError):
                logger.debug("Lyrics fetch failed for %s", track.video_id, exc_info=True)
                if gen == self._play_gen:
                    self.state.translating_lyrics = False
                    self._emit()

        self._lyric_task = asyncio.create_task(_job())

    def _update_active_lyric(self) -> None:
        self.state.active_lyric_idx = active_line_index(self.state.lyrics, self.state.position)

    def _on_mpv_update(self, props: MpvProperties) -> None:
        before = (
            self.state.paused,
            self.state.buffering,
            self.state.volume,
            round(self.state.duration, 1),
            self.state.playing,
        )
        self.state.position = props.position
        if props.duration > 0:
            self.state.duration = props.duration
        self.state.paused = props.paused
        self.state.volume = props.volume
        self.state.buffering = props.buffering or self.state.resolving
        if self.state.track is not None and self._loaded_vid == self.state.track.video_id:
            self.state.playing = not props.paused
        self._update_active_lyric()
        self._maybe_prequeue_autoplay()

        discrete = (
            self.state.paused,
            self.state.buffering,
            self.state.volume,
            round(self.state.duration, 1),
            self.state.playing,
        ) != before
        now = time.monotonic()
        if discrete or (now - self._last_emit_ts) >= POSITION_EMIT_INTERVAL:
            self._last_emit_ts = now
            self._emit()

    def _maybe_prequeue_autoplay(self) -> None:
        """Silently stage radio continuation shortly before the final track finishes."""
        trk = self.state.track
        if (
            trk is None
            or not self._settings.player.autoplay
            or self._radio_fn is None
            or self.queue.repeat != "off"
            or self.queue.peek_next() is not None
            or self.state.duration < 20.0
            or self.state.position < self.state.duration - 12.0
            or self._autoplay_seed_vid == trk.video_id
        ):
            return
        self._autoplay_seed_vid = trk.video_id
        gen = self._play_gen
        if self._autoplay_task and not self._autoplay_task.done():
            self._autoplay_task.cancel()
        self._autoplay_task = asyncio.create_task(self._fill_autoplay(trk.video_id, gen))

    async def _fill_autoplay(self, seed_vid: str, gen: int) -> bool:
        if self._radio_fn is None:
            return False
        try:
            candidates = await self._radio_fn(seed_vid)
        except asyncio.CancelledError:
            raise
        except (*CATALOG_ERRORS, httpx.HTTPError):
            logger.debug("Autoplay radio fetch failed for %s", seed_vid, exc_info=True)
            return False
        if gen != self._play_gen:
            return False
        existing = {t.video_id for t in self.queue.tracks}
        recent = {t.video_id for t in self._history.recent_tracks[:5]}
        blocked = existing | recent | {seed_vid}
        fresh = [t for t in candidates if t.video_id and t.video_id not in blocked][:15]
        if not fresh:
            return False
        self.queue.append(fresh)
        self.state.autoplay_active = True
        self._sync_queue_flags()
        self._prefetch_next()
        self._emit()
        return True

    async def _on_eof(self) -> None:
        nxt = self.queue.advance(manual=False)
        self._sync_queue_flags()
        if nxt is not None:
            await self._start_track(nxt)
            return

        trk = self.state.track
        gen = self._play_gen
        if (
            trk is not None
            and self._settings.player.autoplay
            and self._radio_fn is not None
            and self.queue.repeat == "off"
        ):
            if (
                self._autoplay_task
                and not self._autoplay_task.done()
                and self._autoplay_seed_vid == trk.video_id
            ):
                await self._autoplay_task
            else:
                await self._fill_autoplay(trk.video_id, gen)
            if gen != self._play_gen:
                return
            nxt = self.queue.advance(manual=False)
            self._sync_queue_flags()
            if nxt is not None:
                await self._start_track(nxt)
                return

        self._loaded_vid = None
        self.state.playing = False
        self.state.paused = False
        self.state.buffering = False
        self.state.position = self.state.duration
        self._emit()

    async def _on_mpv_error(self, message: str) -> None:
        if self.state.track is not None:
            self._resolver.invalidate(self.state.track.video_id)
        self._loaded_vid = None
        self.state.playing = False
        self.state.paused = False
        self.state.buffering = False
        self.state.resolving = False
        self.state.error = message or "Stream playback failed"
        self._emit()
