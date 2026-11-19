"""Top navigation bar with tab underline and privacy status badge."""

from __future__ import annotations

from rich.cells import cell_len
from rich.text import Text
from textual.events import Click
from textual.message import Message
from textual.widget import Widget

from musick.ui.theme import ACCENT, BORDER, DIM, TEXT, UNICODE_ICONS, IconSet

NAV_TABS: tuple[tuple[str, str], ...] = (
    ("home", "Home"),
    ("library", "Library"),
    ("queue", "Queue"),
    ("settings", "Settings"),
)


class TopBar(Widget):
    """Two-line top navigation bar matching prototype."""

    DEFAULT_CSS = """
    TopBar {
        background: transparent;
        height: 2;
        width: 1fr;
    }
    """

    class TabSelected(Message):
        def __init__(self, tab_id: str) -> None:
            super().__init__()
            self.tab_id = tab_id

    def __init__(self, active_tab: str = "home", icons: IconSet = UNICODE_ICONS, **kw) -> None:
        super().__init__(**kw)
        self.active_tab = active_tab
        self.icons = icons
        self._hitboxes: list[tuple[int, int, str]] = []

    def set_active(self, tab_id: str) -> None:
        if tab_id != self.active_tab:
            self.active_tab = tab_id
            self.refresh()

    def set_icons(self, icons: IconSet) -> None:
        self.icons = icons
        self.refresh()

    def render(self) -> Text:
        w = max(40, self.size.width or 98)
        self._hitboxes.clear()

        gap_w = 4 if w >= 76 else 2
        gap = " " * gap_w

        # Line 1 keeps a 1-cell inset on both sides so the text never touches the
        # outer frame; line 2 (the divider) spans the full width and joins that frame.
        line1 = Text()
        line1.append(" ")
        x = 1

        line1.append(self.icons.brand + " ", style=ACCENT)
        line1.append("musick", style=f"bold {TEXT}")
        x += cell_len(self.icons.brand) + 1 + cell_len("musick")
        line1.append(gap)
        x += gap_w

        underline_span: tuple[int, int] | None = None
        for idx, (tid, label) in enumerate(NAV_TABS):
            if idx > 0:
                line1.append(gap)
                x += gap_w
            start = x
            is_act = tid == self.active_tab
            line1.append(label, style=f"bold {ACCENT}" if is_act else DIM)
            x += cell_len(label)
            self._hitboxes.append((start, x, tid))
            if is_act:
                underline_span = (start, x)

        inner_right = w - 1  # one blank cell before the right frame border
        badge_text = "no cookies " if inner_right - x >= 14 else ""
        right_len = cell_len(badge_text) + cell_len(self.icons.dot)
        pad = max(1, inner_right - x - right_len)
        line1.append(" " * pad)
        if badge_text:
            line1.append(badge_text, style=DIM)
        line1.append(self.icons.dot, style=ACCENT)

        line2 = Text()
        if underline_span is None:
            line2.append("─" * w, style=BORDER)
        else:
            s, e = underline_span
            if s > 0:
                line2.append("─" * s, style=BORDER)
            line2.append("━" * (e - s), style=ACCENT)
            if e < w:
                line2.append("─" * (w - e), style=BORDER)

        out = Text(no_wrap=True, overflow="ignore")
        out.append_text(line1)
        out.append("\n")
        out.append_text(line2)
        return out

    def on_click(self, event: Click) -> None:
        if event.y != 0:
            return
        for start, end, tid in self._hitboxes:
            if start <= event.x < end:
                self.post_message(self.TabSelected(tid))
                return
