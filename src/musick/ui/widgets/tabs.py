"""Shared sharp underlined section tab bar for detail and search views."""

from __future__ import annotations

from collections.abc import Sequence

from rich.text import Text
from textual.events import Click, Key
from textual.message import Message
from textual.widget import Widget

from musick.ui.theme import ACCENT, BORDER, DIM


class SectionTabs(Widget):
    """Two-row transparent tab header with `━━━` active underline and click hitboxes."""

    DEFAULT_CSS = """
    SectionTabs {
        background: transparent;
        height: 2;
        width: 1fr;
        margin-bottom: 0;
    }
    """

    class TabSelected(Message):
        def __init__(self, widget: SectionTabs, tab_id: str) -> None:
            super().__init__()
            self.widget = widget
            self.tab_id = tab_id

    def __init__(
        self,
        tabs: Sequence[tuple[str, str]],
        active: str | None = None,
        *,
        short_labels: dict[str, str] | None = None,
        **kw,
    ) -> None:
        super().__init__(**kw)
        self.tabs: tuple[tuple[str, str], ...] = tuple(tabs)
        self.active: str = active or (self.tabs[0][0] if self.tabs else "")
        self._short = short_labels or {}
        self._hits: list[tuple[int, int, str]] = []

    def set_active(self, tab_id: str) -> None:
        if tab_id != self.active:
            self.active = tab_id
            self.refresh()

    def set_tabs(
        self, tabs: Sequence[tuple[str, str]], *, short_labels: dict[str, str] | None = None
    ) -> None:
        self.tabs = tuple(tabs)
        if short_labels is not None:
            self._short = short_labels
        self.refresh()

    def _layout_labels(self, width: int) -> tuple[list[tuple[str, str]], int]:
        items = list(self.tabs)
        for sep_w in (3, 2, 1):
            total = sum(len(lbl) for _, lbl in items) + sep_w * max(0, len(items) - 1)
            if total <= width:
                return items, sep_w
        short_items = [(tid, self._short.get(tid, lbl)) for tid, lbl in items]
        for sep_w in (2, 1):
            total = sum(len(lbl) for _, lbl in short_items) + sep_w * max(0, len(short_items) - 1)
            if total <= width:
                return short_items, sep_w
        return short_items, 1

    def render(self) -> Text:
        w = max(24, self.size.width or 70)
        items, sep_w = self._layout_labels(w)
        sep = " " * sep_w
        self._hits.clear()

        l1 = Text()
        x = 0
        span: tuple[int, int] | None = None
        for idx, (tid, label) in enumerate(items):
            if idx > 0:
                l1.append(sep)
                x += sep_w
            s = x
            is_act = tid == self.active
            l1.append(label, style=f"bold {ACCENT}" if is_act else DIM)
            x += len(label)
            self._hits.append((s, x, tid))
            if is_act:
                span = (s, x)

        l2 = Text()
        if span is None:
            l2.append("─" * w, style=BORDER)
        else:
            s, e = span
            if s > 0:
                l2.append("─" * s, style=BORDER)
            l2.append("━" * (e - s), style=ACCENT)
            if e < w:
                l2.append("─" * (w - e), style=BORDER)

        out = Text(no_wrap=True, overflow="ignore")
        out.append_text(l1)
        out.append("\n")
        out.append_text(l2)
        return out

    def on_click(self, event: Click) -> None:
        if event.y != 0:
            return
        for s, e, tid in self._hits:
            if s <= event.x < e:
                self.set_active(tid)
                self.post_message(self.TabSelected(self, tid))
                return

    def on_key(self, event: Key) -> None:
        if not self.tabs:
            return
        ids = [t[0] for t in self.tabs]
        cur = ids.index(self.active) if self.active in ids else 0
        if event.key in ("right", "l"):
            nxt = ids[(cur + 1) % len(ids)]
            self.set_active(nxt)
            self.post_message(self.TabSelected(self, nxt))
            event.stop()
        elif event.key in ("left", "h"):
            prv = ids[(cur - 1) % len(ids)]
            self.set_active(prv)
            self.post_message(self.TabSelected(self, prv))
            event.stop()
