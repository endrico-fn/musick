"""Responsive cover grid supporting S (8x4), M (16x8), and L (24x12) thumbnails."""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass
from typing import Any

from rich.text import Text
from textual.app import ComposeResult
from textual.containers import Horizontal, Vertical
from textual.events import Click, Key, Resize
from textual.message import Message
from textual.widget import Widget
from textual.widgets import Static

from musick.config import COVER_SIZES
from musick.services.images import ImageService
from musick.ui.text_utils import align_cells, trunc_cells
from musick.ui.theme import (
    ACCENT,
    CURSOR_BG,
    CURSOR_BLUR_BG,
    DIM,
    TEXT,
    UNICODE_ICONS,
    IconSet,
    media_icon_for,
)

from .cover import _BASE_HEIGHTS, CoverWidget, cell_aspect_ratio, cover_dims


@dataclass(frozen=True, slots=True)
class GridCardItem:
    title: str
    subtitle: str
    cover_url: str | None
    payload: Any


class _GridCard(Vertical):
    DEFAULT_CSS = """
    _GridCard {
        background: transparent;
        height: auto;
        margin-right: 3;
        margin-bottom: 1;
    }
    """

    def __init__(
        self,
        index: int,
        item: GridCardItem,
        cover_size: str,
        images: ImageService,
        protocol: str,
        icons: IconSet,
        selected: bool = False,
        compact: bool = False,
        card_dims: tuple[int, int] | None = None,
        gap_right: int = 3,
        **kw,
    ) -> None:
        super().__init__(**kw)
        self.index = index
        self.item = item
        self.cover_size = cover_size
        self.images = images
        self.protocol = protocol
        self.icons = icons
        self.selected = selected
        self._focused = True
        w, h = card_dims or cover_dims(cover_size)
        self._card_w = w
        self._card_h = h
        self.styles.width = w
        self.styles.margin = (0, gap_right, 0 if compact else 1, 0)

    def _title_renderable(self, focused: bool = True) -> Text:
        w = self._card_w
        if self.selected:
            bg = CURSOR_BG if focused else CURSOR_BLUR_BG
            fg = f"bold {ACCENT}" if focused else TEXT
            label = f"{self.icons.cursor} " + trunc_cells(self.item.title, max(2, w - 2))
            return Text(
                align_cells(label, w, "l"),
                style=f"{fg} on {bg}",
                no_wrap=True,
                overflow="ignore",
            )
        return Text(
            align_cells(self.item.title, w, "l"),
            style=TEXT,
            no_wrap=True,
            overflow="ignore",
        )

    def _sub_renderable(self) -> Text:
        w = self._card_w
        glyph = media_icon_for(self.item.payload, self.icons)
        prefix = f"{glyph} " if glyph and w >= 10 else ""
        sub_txt = self.item.subtitle or ""
        if w < 14 and " · " in sub_txt:
            # At S size only one metadata token fits; keep the first, drop the rest.
            sub_txt = sub_txt.split(" · ", 1)[0]
        out = Text(no_wrap=True, overflow="ignore")
        if prefix:
            out.append(prefix, style=ACCENT if self.selected else DIM)
        rem_w = max(2, w - len(prefix))
        out.append(align_cells(sub_txt, rem_w, "l"), style=DIM)
        return out

    def compose(self) -> ComposeResult:
        cov = CoverWidget(self.cover_size, images=self.images, protocol=self.protocol)
        cov.set_exact_size(self._card_w, self._card_h)
        cov.set_url(self.item.cover_url)
        yield cov
        yield Static(self._title_renderable(self._focused), classes="card-title")
        yield Static(self._sub_renderable(), classes="card-sub")

    def on_mount(self) -> None:
        self._render_labels(self._focused)

    def set_selected(self, selected: bool, focused: bool = True) -> None:
        if selected != self.selected or focused != self._focused:
            self.selected = selected
            self._focused = focused
            self._render_labels(focused)

    def set_icons(self, icons: IconSet) -> None:
        self.icons = icons
        self._render_labels(self._focused)

    def _render_labels(self, focused: bool = True) -> None:
        if not self.is_attached:
            return
        titles = self.query(".card-title")
        subs = self.query(".card-sub")
        if not titles or not subs:
            return
        titles.first(Static).update(self._title_renderable(focused))
        subs.first(Static).update(self._sub_renderable())


class CoverGrid(Widget):
    """Multi-column responsive cover grid with vim/arrow navigation & S/M/L sizing."""

    can_focus = True

    DEFAULT_CSS = """
    CoverGrid {
        layout: vertical;
        background: transparent;
        height: auto;
        width: 1fr;
    }
    CoverGrid .grid-row {
        layout: horizontal;
        height: auto;
        width: 1fr;
    }
    """

    class ItemActivated(Message):
        def __init__(self, grid: CoverGrid, index: int, payload: Any) -> None:
            super().__init__()
            self.grid = grid
            self.index = index
            self.payload = payload

    class ItemPlay(Message):
        def __init__(self, grid: CoverGrid, index: int, payload: Any) -> None:
            super().__init__()
            self.grid = grid
            self.index = index
            self.payload = payload

    class ItemQueued(Message):
        def __init__(self, grid: CoverGrid, index: int, payload: Any) -> None:
            super().__init__()
            self.grid = grid
            self.index = index
            self.payload = payload

    class ItemPlayNext(Message):
        def __init__(self, grid: CoverGrid, index: int, payload: Any) -> None:
            super().__init__()
            self.grid = grid
            self.index = index
            self.payload = payload

    class ItemSaved(Message):
        def __init__(self, grid: CoverGrid, index: int, payload: Any) -> None:
            super().__init__()
            self.grid = grid
            self.index = index
            self.payload = payload

    class SizeChanged(Message):
        def __init__(self, grid: CoverGrid, size: str) -> None:
            super().__init__()
            self.grid = grid
            self.size = size

    def __init__(
        self,
        images: ImageService,
        cover_size: str = "M",
        protocol: str = "auto",
        icons: IconSet = UNICODE_ICONS,
        single_row: bool = False,
        fit_width: bool = False,
        scale: float = 1.0,
        **kw,
    ) -> None:
        super().__init__(**kw)
        self.images = images
        self.cover_size = cover_size if cover_size in COVER_SIZES else "M"
        self.protocol = protocol
        self.icons = icons
        self.single_row = single_row
        self.fit_width = fit_width
        self.scale = max(0.5, float(scale))
        self.items: list[GridCardItem] = []
        self.cursor: int = 0
        self._cols: int = 0
        self._layout_sig: tuple[int, int, int, int, int] | None = None
        self._needs_rebuild: bool = False

    def set_items(self, items: Sequence[GridCardItem], *, cover_size: str | None = None) -> None:
        self.items = list(items)
        if cover_size in COVER_SIZES:
            self.cover_size = cover_size
        self.cursor = max(0, min(self.cursor, len(self.items) - 1)) if self.items else 0
        self._rebuild()

    def set_cover_size(self, cover_size: str) -> None:
        if cover_size in COVER_SIZES and cover_size != self.cover_size:
            self.cover_size = cover_size
            self._rebuild()
            self.post_message(self.SizeChanged(self, cover_size))

    def set_protocol(self, protocol: str) -> None:
        self.protocol = protocol
        for cov in self.query(CoverWidget):
            cov.set_protocol(protocol)

    def set_icons(self, icons: IconSet) -> None:
        self.icons = icons
        for card in self.query(_GridCard):
            card.set_icons(icons)

    def _compute_layout(self) -> tuple[int, int, int, int, int]:
        """Compute (cols, card_w, card_h, gap, pad_left) fitted to available container width."""
        avail = max(12, self.size.width or 92)
        if self.parent is not None and getattr(self.parent, "show_vertical_scrollbar", False):
            avail = max(12, avail - 1)
        ratio = cell_aspect_ratio()
        base_h = max(3, round(_BASE_HEIGHTS.get(self.cover_size, 8) * self.scale))
        base_w = max(6, round(base_h * ratio))
        if not self.fit_width:
            cols = max(1, (avail + 3) // (base_w + 3))
            return cols, base_w, base_h, 3, 0

        min_gap = 2
        max_gap = 4
        cols = max(1, (avail + min_gap) // (base_w + min_gap))
        if cols == 1:
            card_w = min(avail, base_w)
            card_h = max(3, round(card_w / ratio))
            gap = 0
            pad_left = max(0, (avail - card_w) // 2)
        else:
            slack = avail - (cols * base_w + (cols - 1) * min_gap)
            extra_w = min(2, max(0, slack // cols))
            card_w = base_w + extra_w
            card_h = max(base_h, round(card_w / ratio))
            rem = avail - cols * card_w
            gap = max(min_gap, min(max_gap, rem // (cols - 1)))
            used = cols * card_w + (cols - 1) * gap
            pad_left = max(0, (avail - used) // 2)
        return cols, card_w, card_h, gap, pad_left

    def on_resize(self, event: Resize) -> None:
        del event
        if self.size.width <= 0:
            return
        sig = self._compute_layout()
        if (self._needs_rebuild or sig != self._layout_sig) and self.items:
            self._rebuild()

    def _rebuild(self) -> None:
        if not self.is_mounted:
            return
        if self.size.width <= 0:
            self._needs_rebuild = True
            return
        self._needs_rebuild = False
        if self.children:
            self.remove_children()

        if not self.items:
            self._layout_sig = None
            self.mount(Static(Text("No items", style=DIM)))
            return

        cols, card_w, card_h, gap, pad_left = self._compute_layout()
        self._cols = cols
        self._layout_sig = (cols, card_w, card_h, gap, pad_left)
        items = self.items[: cols] if self.single_row else self.items
        if self.cursor >= len(items):
            self.cursor = max(0, len(items) - 1)

        rows_to_mount: list[Horizontal] = []
        for r0 in range(0, len(items), cols):
            chunk = items[r0 : r0 + cols]
            cards = [
                _GridCard(
                    r0 + i,
                    card,
                    self.cover_size,
                    self.images,
                    self.protocol,
                    self.icons,
                    selected=(r0 + i == self.cursor),
                    compact=self.single_row,
                    card_dims=(card_w, card_h),
                    gap_right=gap if (i + 1 < len(chunk)) else 0,
                )
                for i, card in enumerate(chunk)
            ]
            row = Horizontal(*cards, classes="grid-row")
            if pad_left:
                row.styles.padding = (0, 0, 0, pad_left)
            rows_to_mount.append(row)
        self.mount(*rows_to_mount)
        self.refresh(layout=True)

    def on_mount(self) -> None:
        if self.items:
            self._rebuild()

    def on_focus(self) -> None:
        self._refresh_selection()

    def on_blur(self) -> None:
        self._refresh_selection()

    def _refresh_selection(self, *, scroll: bool = False) -> None:
        selected_card: _GridCard | None = None
        for card in self.query(_GridCard):
            is_sel = card.index == self.cursor
            card.set_selected(is_sel, focused=self.has_focus)
            if is_sel:
                selected_card = card
        if scroll and selected_card is not None and selected_card.is_mounted:
            node = self.parent
            while node is not None:
                if hasattr(node, "scroll_to_widget") and getattr(
                    node, "allow_vertical_scroll", False
                ):
                    node.scroll_to_widget(selected_card, animate=False)
                    break
                node = node.parent

    def _move(self, delta: int) -> None:
        visible_count = min(len(self.items), self._cols) if self.single_row else len(self.items)
        if visible_count <= 0:
            return
        nxt = max(0, min(visible_count - 1, self.cursor + delta))
        if nxt != self.cursor:
            self.cursor = nxt
            self._refresh_selection(scroll=True)

    def on_key(self, event: Key) -> None:
        k = event.key
        if k in ("right", "l"):
            self._move(1)
            event.stop()
        elif k in ("left", "h"):
            self._move(-1)
            event.stop()
        elif k in ("down", "j") and not self.single_row:
            self._move(self._cols)
            event.stop()
        elif k in ("up", "k") and not self.single_row:
            self._move(-self._cols)
            event.stop()
        elif k == "enter" and self.items:
            self.post_message(
                self.ItemActivated(self, self.cursor, self.items[self.cursor].payload)
            )
            event.stop()
        elif k == "p" and self.items:
            self.post_message(self.ItemPlay(self, self.cursor, self.items[self.cursor].payload))
            event.stop()
        elif k in ("a", "plus") and self.items:
            self.post_message(self.ItemQueued(self, self.cursor, self.items[self.cursor].payload))
            event.stop()
        elif k == "N" and self.items:
            self.post_message(self.ItemPlayNext(self, self.cursor, self.items[self.cursor].payload))
            event.stop()
        elif k == "w" and self.items:
            self.post_message(self.ItemSaved(self, self.cursor, self.items[self.cursor].payload))
            event.stop()
        elif k == "c":
            order = ("S", "M", "L")
            nxt = order[(order.index(self.cover_size) + 1) % 3]
            self.set_cover_size(nxt)
            event.stop()
        elif k in ("S", "M", "L"):
            self.set_cover_size(k)
            event.stop()

    def on_click(self, event: Click) -> None:
        self.focus()
        node = event.widget
        while node is not None and not isinstance(node, _GridCard):
            node = node.parent
        if isinstance(node, _GridCard):
            was_sel = node.index == self.cursor
            self.cursor = node.index
            self._refresh_selection()
            if was_sel or event.chain >= 2:
                self.post_message(self.ItemActivated(self, node.index, node.item.payload))
