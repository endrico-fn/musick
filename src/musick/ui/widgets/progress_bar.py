"""Clickable progress/seek bar widget."""

from __future__ import annotations

from rich.text import Text
from textual.events import Click
from textual.message import Message
from textual.widget import Widget

from musick.domain import format_duration
from musick.ui.theme import ACCENT, BORDER, DIM


class SeekBar(Widget):
    """Renders `01:47 ━━━━━━━━●────────── 03:20` and emits Seeked on click."""

    DEFAULT_CSS = """
    SeekBar {
        background: transparent;
        height: 1;
        width: 1fr;
    }
    """

    class Seeked(Message):
        def __init__(self, seconds: float) -> None:
            super().__init__()
            self.seconds = seconds

    def __init__(self, **kw) -> None:
        super().__init__(**kw)
        self.position: float = 0.0
        self.duration: float = 0.0
        self._bar_start: int = 0
        self._bar_width: int = 10

    def update_progress(self, position: float, duration: float) -> None:
        pos = max(0.0, position)
        dur = max(0.0, duration)
        if abs(pos - self.position) < 0.15 and abs(dur - self.duration) < 0.15:
            return
        self.position = pos
        self.duration = dur
        self.refresh()

    def render(self) -> Text:
        w = max(20, self.size.width or 60)
        left_str = (
            format_duration(int(self.position))
            if self.duration > 0 or self.position > 0
            else "0:00"
        )
        right_str = format_duration(int(self.duration)) if self.duration > 0 else "--:--"
        left_pad = left_str.rjust(5)
        right_pad = right_str.rjust(5)

        bar_w = max(8, w - len(left_pad) - len(right_pad) - 2)
        self._bar_start = len(left_pad) + 1
        self._bar_width = bar_w

        ratio = min(1.0, max(0.0, self.position / self.duration)) if self.duration > 0 else 0.0
        knob = min(bar_w - 1, round(ratio * (bar_w - 1)))

        txt = Text(no_wrap=True, overflow="ignore")
        txt.append(left_pad + " ", style=DIM)
        txt.append("━" * knob, style=ACCENT)
        txt.append("●", style=ACCENT)
        txt.append("─" * max(0, bar_w - knob - 1), style=BORDER)
        txt.append(" " + right_pad, style=DIM)
        return txt

    def on_click(self, event: Click) -> None:
        if self.duration <= 0 or self._bar_width <= 1:
            return
        rel = event.x - self._bar_start
        if 0 <= rel <= self._bar_width:
            ratio = min(1.0, max(0.0, rel / max(1, self._bar_width - 1)))
            self.post_message(self.Seeked(ratio * self.duration))
