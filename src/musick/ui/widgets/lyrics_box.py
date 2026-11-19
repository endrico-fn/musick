"""Auto-centering lyrics widget with playback highlight only (no arrow or cursor bar)."""

from __future__ import annotations

from rich.text import Text
from textual.events import Click, Key, MouseScrollDown, MouseScrollUp
from textual.message import Message
from textual.widget import Widget

from musick.domain import Lyrics
from musick.ui.text_utils import (
    WrappedLyricRow,
    format_lyric_row,
    wrap_bilingual_lyric_lines,
)
from musick.ui.theme import DIM


class LyricsBox(Widget):
    """Auto-centering synced/plain lyrics panel with bold accent active line only."""

    can_focus = True

    DEFAULT_CSS = """
    LyricsBox {
        background: transparent;
        width: 1fr;
        height: 1fr;
    }
    """

    class LineSeek(Message):
        def __init__(self, seconds: float) -> None:
            super().__init__()
            self.seconds = seconds

    def __init__(
        self,
        *,
        blank_between: bool = False,
        pad_left: int = 0,
        **kw,
    ) -> None:
        super().__init__(**kw)
        self.blank_between = blank_between
        self.pad_left = max(0, pad_left)
        self.lyrics = Lyrics()
        self.active_idx: int | None = None
        self.show_translation: bool = False
        self._has_track: bool = False
        self._scroll_offset: int = 0
        self._rendered_orig_indices: list[int] = []
        self._wrap_key: tuple[object, ...] | None = None
        self._wrap_rows: list[WrappedLyricRow] = []

    def set_lyrics(
        self,
        lyrics: Lyrics,
        active_idx: int | None,
        *,
        has_track: bool = True,
        show_translation: bool = False,
    ) -> None:
        changed = (
            lyrics != self.lyrics
            or active_idx != self.active_idx
            or has_track != self._has_track
            or show_translation != self.show_translation
        )
        if lyrics != self.lyrics or active_idx != self.active_idx:
            self._scroll_offset = 0
        self.lyrics = lyrics
        self.active_idx = active_idx
        self._has_track = has_track
        self.show_translation = show_translation
        if changed:
            self.refresh()

    def _wrapped_rows(self, width: int) -> list[WrappedLyricRow]:
        key = (width, self.lyrics, self.show_translation, self.blank_between, self.pad_left)
        if key != self._wrap_key:
            self._wrap_key = key
            self._wrap_rows = wrap_bilingual_lyric_lines(
                self.lyrics.lines,
                width,
                show_translation=self.show_translation,
                blank_between=self.blank_between,
                pad_left=self.pad_left,
            )
        return self._wrap_rows

    def _synced_window_start(self, rows: list[WrappedLyricRow], height: int) -> int:
        """Compute signed start index placing the active stanza vertically dead-center."""
        total = len(rows)
        target_idx = self.active_idx if self.active_idx is not None else 0
        target_rows = [
            r_i for r_i, r in enumerate(rows) if r.orig_idx == target_idx and not r.is_spacer
        ]
        first_r = target_rows[0] if target_rows else 0
        last_r = target_rows[-1] if target_rows else first_r
        span = last_r - first_r + 1
        top_pad = max(0, (height - span) // 2)

        # Keep manual scroll within bounds where at least the lyrics edge stays centered.
        min_off = -first_r
        max_off = max(0, total - 1 - first_r)
        self._scroll_offset = max(min_off, min(max_off, self._scroll_offset))
        return first_r - top_pad + self._scroll_offset

    def _window_start(self, rows: list[WrappedLyricRow], height: int) -> int:
        if self.lyrics.synced:
            return self._synced_window_start(rows, height)
        max_scroll = max(0, len(rows) - height)
        self._scroll_offset = max(0, min(max_scroll, self._scroll_offset))
        return self._scroll_offset

    def render(self) -> Text:
        w = max(16, self.size.width or 28)
        h = max(3, self.size.height or 8)
        out = Text(no_wrap=True, overflow="ignore")
        self._rendered_orig_indices.clear()

        if not self._has_track:
            out.append("  Play a song for synced lyrics", style=DIM)
            return out

        if not self.lyrics.lines:
            out.append("  No lyrics available", style=DIM)
            return out

        rows = self._wrapped_rows(w)
        total = len(rows)
        start = self._window_start(rows, h)

        for v_i in range(h):
            if v_i > 0:
                out.append("\n")
            r_i = start + v_i
            if r_i < 0 or r_i >= total:
                self._rendered_orig_indices.append(-1)
                continue
            r = rows[r_i]
            self._rendered_orig_indices.append(r.orig_idx)
            is_act = r.orig_idx == self.active_idx
            is_past = self.active_idx is not None and r.orig_idx < self.active_idx
            txt, style = format_lyric_row(
                r,
                pad_left=self.pad_left,
                is_act=is_act,
                is_past=is_past,
            )
            if txt:
                out.append(txt, style=style)

        return out

    def _scroll_by(self, delta: int) -> None:
        if not self.lyrics.lines:
            return
        self._scroll_offset += delta
        self.refresh()

    def on_key(self, event: Key) -> None:
        if not self.lyrics.lines:
            return
        k = event.key
        if k in ("down", "j"):
            self._scroll_by(1)
            event.stop()
        elif k in ("up", "k"):
            self._scroll_by(-1)
            event.stop()
        elif k in ("c", "home"):
            self._scroll_offset = 0
            self.refresh()
            event.stop()

    def on_mouse_scroll_up(self, event: MouseScrollUp) -> None:
        if self.lyrics.lines:
            self._scroll_by(-2)
            event.stop()

    def on_mouse_scroll_down(self, event: MouseScrollDown) -> None:
        if self.lyrics.lines:
            self._scroll_by(2)
            event.stop()

    def on_click(self, event: Click) -> None:
        self.focus()
        if 0 <= event.y < len(self._rendered_orig_indices):
            orig = self._rendered_orig_indices[event.y]
            if self.lyrics.synced and 0 <= orig < len(self.lyrics.lines):
                self._scroll_offset = 0
                self.post_message(self.LineSeek(self.lyrics.lines[orig].time))
