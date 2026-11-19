"""Synchronized dot-loading and diagonal-shimmer skeleton animations."""

from __future__ import annotations

from collections.abc import Callable
from typing import Literal

from rich.text import Text
from textual.timer import Timer
from textual.widget import Widget

from musick.ui.theme import SKEL_DARK, SKEL_LIGHT, SKEL_MID

DOT_FRAMES: tuple[str, ...] = ("●··", "·●·", "··●")

SkeletonKind = Literal["cover", "table_rows"]


class ShimmerClock:
    """Single shared 120ms ticker for all active skeleton/dot widgets (zero CPU when idle)."""

    def __init__(self) -> None:
        self.tick: int = 0
        self.reduced_motion: bool = False
        self._listeners: set[Callable[[int], None]] = set()
        self._timer: Timer | None = None
        self._owner: Widget | None = None

    def bind_app(self, app_widget: Widget) -> None:
        if self._timer is not None:
            self._timer.stop()
            self._timer = None
        self._listeners.clear()
        self._owner = app_widget

    def unbind_app(self, app_widget: Widget) -> None:
        if self._owner is app_widget:
            if self._timer is not None:
                self._timer.stop()
                self._timer = None
            self._listeners.clear()
            self._owner = None

    def set_reduced_motion(self, value: bool) -> None:
        self.reduced_motion = value
        if value and self._timer is not None:
            self._timer.stop()
            self._timer = None
        elif not value and self._listeners:
            self._ensure_timer()
        for fn in list(self._listeners):
            fn(self.tick)

    def subscribe(self, fn: Callable[[int], None]) -> Callable[[], None]:
        self._listeners.add(fn)
        self._ensure_timer()

        def unsub() -> None:
            self._listeners.discard(fn)
            if not self._listeners and self._timer is not None:
                self._timer.stop()
                self._timer = None

        return unsub

    def _ensure_timer(self) -> None:
        if self.reduced_motion or self._timer is not None or self._owner is None:
            return
        self._timer = self._owner.set_interval(0.12, self._advance)

    def _advance(self) -> None:
        if self.reduced_motion or not self._listeners:
            return
        self.tick = (self.tick + 1) % 10000
        for fn in list(self._listeners):
            fn(self.tick)


SHIMMER_CLOCK = ShimmerClock()


def dot_frame(tick: int, reduced_motion: bool = False) -> str:
    if reduced_motion:
        return "●●●"
    # 120ms base * 2 -> ~240ms per dot step
    return DOT_FRAMES[(tick // 2) % len(DOT_FRAMES)]


def shimmer_line(
    mask: str,
    row: int,
    tick: int,
    x_offset: int = 0,
    reduced_motion: bool = False,
) -> Text:
    """Render a skeleton mask ('x' = block, space = blank) with diagonal shimmer."""
    txt = Text()
    if reduced_motion:
        run = ""
        is_x = False
        for ch in mask:
            cur = ch == "x"
            if run and cur != is_x:
                txt.append("░" * len(run) if is_x else run, style=SKEL_DARK if is_x else "")
                run = ""
            run += ch
            is_x = cur
        if run:
            txt.append("░" * len(run) if is_x else run, style=SKEL_DARK if is_x else "")
        return txt

    span = max(80, len(mask) + 32)
    wave = (tick * 3 + row * 4) % span - 6
    cur_style: str | None = None
    buf: list[str] = []

    for x, ch in enumerate(mask):
        if ch != "x":
            st = ""
            glyph = " "
        else:
            dist = abs((x + x_offset) - wave)
            if dist <= 1:
                st = SKEL_LIGHT
                glyph = "▓"
            elif dist <= 4:
                st = SKEL_MID
                glyph = "▒"
            else:
                st = SKEL_DARK
                glyph = "░"
        if st != cur_style:
            if buf:
                txt.append("".join(buf), style=cur_style or "")
                buf.clear()
            cur_style = st
        buf.append(glyph)

    if buf:
        txt.append("".join(buf), style=cur_style or "")
    return txt


def _row_mask(width: int, segs: list[tuple[int, int]]) -> str:
    chars = [" "] * max(1, width)
    for start, length in segs:
        for i in range(max(0, start), min(width, start + length)):
            chars[i] = "x"
    return "".join(chars)


class Skeleton(Widget):
    """Animated layout-matched skeleton placeholder (zero CPU when hidden)."""

    DEFAULT_CSS = """
    Skeleton {
        background: transparent;
        height: auto;
    }
    """

    def __init__(
        self,
        kind: SkeletonKind = "table_rows",
        rows: int = 6,
        cover_w: int | None = None,
        cover_h: int | None = None,
        **kw,
    ) -> None:
        super().__init__(**kw)
        self.kind = kind
        self.rows = rows
        from .cover import cover_dims

        lw, lh = cover_dims("L")
        self.cover_w = cover_w or lw
        self.cover_h = cover_h or lh
        self._unsub: Callable[[], None] | None = None
        if kind == "cover":
            self.styles.width = self.cover_w
            self.styles.height = self.cover_h

    def set_cover_dims(self, w: int, h: int) -> None:
        self.cover_w = max(4, int(w))
        self.cover_h = max(2, int(h))
        if self.kind == "cover":
            self.styles.width = self.cover_w
            self.styles.height = self.cover_h
            self.refresh(layout=True)

    def _sync_clock(self) -> None:
        if self.is_mounted and self.display and self._unsub is None:
            self._unsub = SHIMMER_CLOCK.subscribe(self._on_tick)
        elif (not self.is_mounted or not self.display) and self._unsub is not None:
            self._unsub()
            self._unsub = None

    def on_mount(self) -> None:
        self._sync_clock()

    def on_unmount(self) -> None:
        if self._unsub:
            self._unsub()
            self._unsub = None

    def watch_display(self, _visible: bool) -> None:
        self._sync_clock()

    def _on_tick(self, _tick: int) -> None:
        if self.display:
            self.refresh()

    def get_content_height(self, container, viewport, width: int) -> int:
        if self.kind == "cover":
            return self.cover_h
        return max(1, self.rows)

    def render(self) -> Text:
        w = max(20, self.size.width or 80)
        tick = SHIMMER_CLOCK.tick
        rm = SHIMMER_CLOCK.reduced_motion
        out = Text()

        if self.kind == "cover":
            cw = max(4, self.size.width or self.cover_w)
            ch = max(2, self.size.height or self.cover_h)
            mask = "x" * cw
            for y in range(ch):
                out.append_text(shimmer_line(mask, y, tick, reduced_motion=rm))
                if y + 1 < ch:
                    out.append("\n")
            return out

        # table_rows
        title_w = max(16, w - 48)
        for i in range(self.rows):
            t_len = max(12, min(title_w - 4, 18 + (i * 7) % 15))
            a_len = 14 + (i * 5) % 8
            segs = [
                (2, 2),
                (7, t_len),
                (7 + title_w + 2, a_len),
                (max(7 + title_w + 22, w - 16), 5),
                (max(w - 7, 20), 5),
            ]
            mask = _row_mask(w, segs)
            out.append_text(shimmer_line(mask, i, tick, reduced_motion=rm))
            if i + 1 < self.rows:
                out.append("\n")
        return out
