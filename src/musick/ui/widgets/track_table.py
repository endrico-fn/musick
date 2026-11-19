"""Focus-aware, zero-bg table widget with cursor, playing mark, mini icons, and skeleton tail."""

from __future__ import annotations

from collections.abc import Callable, Sequence
from dataclasses import dataclass
from typing import Any

from rich.cells import cell_len
from rich.text import Text
from textual.events import Click, Key
from textual.geometry import Region
from textual.message import Message
from textual.widget import Widget

from musick.ui.text_utils import align_cells
from musick.ui.theme import (
    ACCENT,
    BORDER_DIM,
    CURSOR_BG,
    CURSOR_BLUR_BG,
    DIM,
    TEXT,
    TR_ACTIVE,
    TR_PAST,
    UNICODE_ICONS,
    IconSet,
    media_icon_for,
)

from .loading import SHIMMER_CLOCK, shimmer_line
from .scroll import scroll_child_region


@dataclass(frozen=True, slots=True)
class ColumnDef:
    title: str
    width: int | None = None
    align: str = "l"  # "l" or "r"


class TrackTable(Widget):
    """Sharp, transparent-bg table with `▸` cursor, `▶` active mark, and optional mini icons."""

    can_focus = True

    DEFAULT_CSS = """
    TrackTable {
        background: transparent;
        height: auto;
        width: 1fr;
    }
    """

    class RowActivated(Message):
        def __init__(self, table: TrackTable, index: int, payload: Any) -> None:
            super().__init__()
            self.table = table
            self.index = index
            self.payload = payload

    class RowQueued(Message):
        def __init__(self, table: TrackTable, index: int, payload: Any) -> None:
            super().__init__()
            self.table = table
            self.index = index
            self.payload = payload

    class RowPlayNext(Message):
        def __init__(self, table: TrackTable, index: int, payload: Any) -> None:
            super().__init__()
            self.table = table
            self.index = index
            self.payload = payload

    class RowPlay(Message):
        def __init__(self, table: TrackTable, index: int, payload: Any) -> None:
            super().__init__()
            self.table = table
            self.index = index
            self.payload = payload

    class CursorChanged(Message):
        def __init__(self, table: TrackTable, index: int, payload: Any) -> None:
            super().__init__()
            self.table = table
            self.index = index
            self.payload = payload

    def __init__(
        self,
        columns: Sequence[ColumnDef],
        sort_col: str | None = None,
        icons: IconSet = UNICODE_ICONS,
        show_header: bool = True,
        allow_none_cursor: bool = False,
        allow_a_queue: bool = True,
        icon_col: str | None = None,
        icon_fn: Callable[[Any, IconSet], str] | None = None,
        bubble_boundary: bool = False,
        **kw,
    ) -> None:
        super().__init__(**kw)
        self.columns = list(columns)
        self.sort_col = sort_col
        self.icons = icons
        self.show_header = show_header
        self.allow_none_cursor = allow_none_cursor
        self.allow_a_queue = allow_a_queue
        self.icon_col = icon_col
        self._icon_fn = icon_fn or (media_icon_for if icon_col else None)
        self.bubble_boundary = bubble_boundary
        self.rows: list[tuple[list[str], Any]] = []
        self.row_roles: list[str] = []
        self.cursor: int = -1 if allow_none_cursor else 0
        self.playing_index: int | None = None
        self.skeleton_tail: int = 0
        self._unsub: Callable[[], None] | None = None
        self._last_click_idx: int | None = None

    def set_icon_fn(self, icon_fn: Callable[[Any, IconSet], str] | None) -> None:
        self._icon_fn = icon_fn or (media_icon_for if self.icon_col else None)
        self.refresh()

    def set_columns(self, columns: Sequence[ColumnDef], sort_col: str | None = None) -> None:
        self.columns = list(columns)
        if sort_col is not None:
            self.sort_col = sort_col
        self.refresh(layout=True)

    def set_icons(self, icons: IconSet) -> None:
        self.icons = icons
        self.refresh()

    def set_data(
        self,
        rows: Sequence[tuple[list[str], Any]],
        *,
        playing_index: int | None = None,
        sort_col: str | None = None,
        skeleton_tail: int = 0,
        row_roles: Sequence[str] | None = None,
    ) -> None:
        self.rows = list(rows)
        self.row_roles = list(row_roles) if row_roles is not None else []
        if sort_col is not None:
            self.sort_col = sort_col
        self.playing_index = playing_index
        self.skeleton_tail = max(0, skeleton_tail)
        if not self.rows:
            self.cursor = -1 if self.allow_none_cursor else 0
        elif self.allow_none_cursor:
            self.cursor = -1
        else:
            self.cursor = max(0, min(self.cursor, len(self.rows) - 1))
        self._sync_skeleton_sub()
        self.refresh(layout=True)

    def set_playing_index(self, idx: int | None) -> None:
        if idx != self.playing_index:
            self.playing_index = idx
            self.refresh()

    def reset_cursor(self) -> None:
        target = -1 if self.allow_none_cursor else 0
        if self.cursor != target:
            self.cursor = target
            self.refresh()

    def set_cursor(self, index: int, *, scroll: bool = True) -> None:
        if not self.rows:
            return
        min_idx = -1 if self.allow_none_cursor else 0
        nxt = max(min_idx, min(len(self.rows) - 1, index))
        if nxt != self.cursor:
            self.cursor = nxt
            self.refresh()
        if scroll and self.cursor >= 0:
            self._scroll_cursor_into_view()

    def set_skeleton_tail(self, count: int) -> None:
        self.skeleton_tail = max(0, count)
        self._sync_skeleton_sub()
        self.refresh(layout=True)

    def _sync_skeleton_sub(self) -> None:
        if self.skeleton_tail > 0 and self._unsub is None and self.is_mounted:
            self._unsub = SHIMMER_CLOCK.subscribe(lambda _: self.refresh())
        elif self.skeleton_tail == 0 and self._unsub is not None:
            self._unsub()
            self._unsub = None

    def on_mount(self) -> None:
        self._sync_skeleton_sub()

    def on_unmount(self) -> None:
        if self._unsub is not None:
            self._unsub()
            self._unsub = None

    def get_content_height(self, container, viewport, width: int) -> int:
        hdr = 2 if self.show_header else 0
        body = max(1, len(self.rows)) + self.skeleton_tail
        return hdr + body

    def _compute_widths(self, total_w: int) -> list[int]:
        n = len(self.columns)
        if n == 0:
            return []
        gap_total = 2 * max(0, n - 1)
        usable = max(8, total_w - 2 - gap_total)
        fixed_cols = [c.width for c in self.columns if c.width is not None]
        fixed_sum = sum(fixed_cols)
        flex_count = sum(1 for c in self.columns if c.width is None)

        compact_sum = sum(w for w in fixed_cols if w <= 10)
        wide_sum = sum(w for w in fixed_cols if w > 10)
        if flex_count > 0 and wide_sum > 0:
            desired_flex = min(22, max(8, (usable - compact_sum - 10) // flex_count))
            min_flex = desired_flex * flex_count
        else:
            min_flex = 8 * flex_count

        if fixed_sum + min_flex > usable and fixed_cols:
            # Preserve compact columns like "#" (3-4), "Plays", and "Time" before wide columns
            if wide_sum > 0 and usable > compact_sum + min_flex:
                wide_budget = max(6, usable - min_flex - compact_sum)
                scale = max(0.35, min(1.0, wide_budget / wide_sum))
                widths_fixed = [w if w <= 10 else max(6, int(w * scale)) for w in fixed_cols]
            else:
                scale = max(0.25, min(1.0, (usable - 4 * flex_count) / fixed_sum))
                widths_fixed = [max(3, int(w * scale)) for w in fixed_cols]
        else:
            widths_fixed = list(fixed_cols)

        avail_flex = max(4 * flex_count, usable - sum(widths_fixed))
        flex_w = max(4, avail_flex // max(1, flex_count)) if flex_count else 0

        out: list[int] = []
        fi = 0
        for c in self.columns:
            if c.width is not None:
                out.append(widths_fixed[fi])
                fi += 1
            else:
                out.append(flex_w)
        return out

    def _row_icon_for(self, payload: Any) -> str:
        if not self.icon_col or self._icon_fn is None:
            return ""
        return self._icon_fn(payload, self.icons)

    def render(self) -> Text:
        w = max(30, self.size.width or 94)
        if self.parent is not None and getattr(self.parent, "show_vertical_scrollbar", False):
            w = max(28, w - 1)
        ws = self._compute_widths(w)
        out = Text(no_wrap=True, overflow="ignore")

        if self.show_header:
            hdr_parts = []
            for col, cw in zip(self.columns, ws, strict=True):
                title = (
                    f"{col.title} {self.icons.sort_desc}"
                    if col.title == self.sort_col
                    else col.title
                )
                hdr_parts.append(align_cells(title, cw, col.align))
            out.append("  " + "  ".join(hdr_parts), style=DIM)
            out.append("\n")
            out.append("─" * w, style=BORDER_DIM)

        if not self.rows and self.skeleton_tail == 0:
            if self.show_header:
                out.append("\n")
            out.append("  No items to display", style=DIM)
            return out

        for idx, (cells, payload) in enumerate(self.rows):
            if self.show_header or idx > 0:
                out.append("\n")
            glyph = self._row_icon_for(payload)
            is_cur = idx == self.cursor
            is_play = idx == self.playing_index

            if is_cur or is_play:
                formatted_parts: list[str] = []
                for val, cw, col in zip(cells, ws, self.columns, strict=False):
                    txt_val = str(val)
                    if glyph and col.title == self.icon_col and cw > cell_len(glyph) + 2:
                        txt_val = f"{glyph} {txt_val}"
                    formatted_parts.append(align_cells(txt_val, cw, col.align))
                formatted = "  ".join(formatted_parts)
                if is_cur:
                    prefix = f"{self.icons.playing} " if is_play else f"{self.icons.cursor} "
                    line = align_cells(prefix + formatted, w, "l")
                    bg = CURSOR_BG if self.has_focus else CURSOR_BLUR_BG
                    st = f"bold {ACCENT} on {bg}" if is_play else f"{TEXT} on {bg}"
                    out.append(line, style=st)
                else:
                    out.append(f"{self.icons.playing} ", style=f"bold {ACCENT}")
                    out.append(formatted, style=ACCENT)
            else:
                role = self.row_roles[idx] if idx < len(self.row_roles) else ""
                is_played = role == "played"
                is_next = role == "next"
                row_style = TR_PAST if is_played else (TR_ACTIVE if is_next else TEXT)
                glyph_style = TR_PAST if is_played else ACCENT
                out.append("  ", style=row_style)
                for col_i, (val, cw, col) in enumerate(zip(cells, ws, self.columns, strict=False)):
                    if col_i > 0:
                        out.append("  ", style=row_style)
                    txt_val = str(val)
                    col_style = (
                        ACCENT
                        if (is_next and col.title in ("Status", "Role"))
                        else (DIM if col.title == "#" and not is_played else row_style)
                    )
                    if glyph and col.title == self.icon_col and cw > cell_len(glyph) + 2:
                        prefix_icon = f"{glyph} "
                        body = align_cells(txt_val, cw - cell_len(prefix_icon), col.align)
                        out.append(prefix_icon, style=glyph_style)
                        out.append(body, style=col_style)
                    else:
                        out.append(align_cells(txt_val, cw, col.align), style=col_style)

        if self.skeleton_tail > 0:
            tick = SHIMMER_CLOCK.tick
            rm = SHIMMER_CLOCK.reduced_motion
            for r in range(self.skeleton_tail):
                out.append("\n")
                mask_chars = [" "] * w
                x = 2
                for cw, col in zip(ws, self.columns, strict=True):
                    seg = max(2, min(cw - 2, cw if col.width else max(12, cw - 6 - (r * 5) % 10)))
                    s_x = x + (cw - seg) if col.align == "r" else x
                    for p in range(max(0, s_x), min(w, s_x + seg)):
                        mask_chars[p] = "x"
                    x += cw + 2
                out.append_text(
                    shimmer_line("".join(mask_chars), len(self.rows) + r, tick, reduced_motion=rm)
                )

        return out

    def move_cursor(self, delta: int) -> bool:
        if not self.rows:
            return False
        min_idx = -1 if self.allow_none_cursor else 0
        nxt = max(min_idx, min(len(self.rows) - 1, self.cursor + delta))
        if nxt != self.cursor:
            self.cursor = nxt
            self.refresh()
            if self.cursor >= 0:
                self.post_message(self.CursorChanged(self, self.cursor, self.rows[self.cursor][1]))
                self._scroll_cursor_into_view()
            return True
        return False

    def _scroll_cursor_into_view(self) -> None:
        if self.cursor < 0 or not self.is_mounted:
            return
        hdr = 2 if self.show_header else 0
        y_row = hdr + self.cursor
        scroll_child_region(self, Region(0, y_row, max(1, self.size.width), 1), animate=False)

    def on_key(self, event: Key) -> None:
        if not self.rows:
            return
        k = event.key
        if k in ("down", "j"):
            if self.move_cursor(1) or not self.bubble_boundary:
                event.stop()
        elif k in ("up", "k"):
            if self.move_cursor(-1) or not self.bubble_boundary:
                event.stop()
        elif k in ("home", "g"):
            self.move_cursor(-len(self.rows))
            event.stop()
        elif k in ("end", "G"):
            self.move_cursor(len(self.rows))
            event.stop()
        elif k == "pagedown":
            self.move_cursor(10)
            event.stop()
        elif k == "pageup":
            self.move_cursor(-10)
            event.stop()
        elif k == "enter" and self.cursor >= 0:
            self.post_message(self.RowActivated(self, self.cursor, self.rows[self.cursor][1]))
            event.stop()
        elif k == "p" and self.cursor >= 0:
            self.post_message(self.RowPlay(self, self.cursor, self.rows[self.cursor][1]))
            event.stop()
        elif (k == "plus" or (k == "a" and self.allow_a_queue)) and self.cursor >= 0:
            self.post_message(self.RowQueued(self, self.cursor, self.rows[self.cursor][1]))
            event.stop()
        elif k == "N" and self.cursor >= 0:
            self.post_message(self.RowPlayNext(self, self.cursor, self.rows[self.cursor][1]))
            event.stop()

    def on_click(self, event: Click) -> None:
        self.focus()
        hdr = 2 if self.show_header else 0
        idx = event.y - hdr
        if 0 <= idx < len(self.rows):
            was_same = idx == self.cursor and self._last_click_idx == idx
            self.cursor = idx
            self._last_click_idx = idx
            self.refresh()
            self.post_message(self.CursorChanged(self, idx, self.rows[idx][1]))
            if was_same or event.chain >= 2:
                self.post_message(self.RowActivated(self, idx, self.rows[idx][1]))
