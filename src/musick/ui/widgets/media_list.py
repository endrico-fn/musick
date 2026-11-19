"""Two-line media list widget: Title + Artist below Title, Time + Album below Time."""

from __future__ import annotations

import asyncio
import contextlib
import logging
from collections.abc import Sequence
from dataclasses import dataclass
from typing import Any

import httpx
from PIL import Image as PILImage
from rich.cells import cell_len
from rich.console import Console
from rich.segment import Segment
from rich.text import Text
from textual.events import Click, Key
from textual.geometry import Region
from textual.message import Message
from textual.strip import Strip
from textual.widget import Widget

from musick.domain import (
    AlbumSummary,
    ArtistSummary,
    PlaylistSummary,
    SearchItem,
    Track,
)
from musick.services.images import ImageService
from musick.ui.text_utils import align_cells, split_row_cells
from musick.ui.theme import (
    ACCENT,
    CURSOR_BG,
    CURSOR_BLUR_BG,
    DIM,
    SKEL_DARK,
    TEXT,
    TR_ACTIVE,
    TR_PAST,
    TR_UPCOMING,
    UNICODE_ICONS,
    IconSet,
    media_kind_for,
)

from .cover import renderable_for, square_crop
from .scroll import scroll_child_region

logger = logging.getLogger(__name__)
_FALLBACK_CONSOLE = Console(color_system="truecolor", legacy_windows=False)


@dataclass(frozen=True, slots=True)
class MediaListItem:
    top_left: str
    bottom_left: str
    top_right: str
    bottom_right: str
    payload: Any
    cover_url: str | None = None
    kind: str = ""
    role: str = ""


THUMB_W = 4
THUMB_H = 2
THUMB_GUTTER = THUMB_W + 1
THUMB_CACHE_MAX = 256


def media_item_from(item: SearchItem | Track, *, role: str = "") -> MediaListItem:
    """Convert a domain Track or SearchItem into a two-line MediaListItem."""
    kind = media_kind_for(item)
    if isinstance(item, Track):
        kind_badge = ""
        if not item.album:
            if kind == "video":
                kind_badge = "Video"
            elif item.video_type == "MUSIC_VIDEO_TYPE_ATV":
                kind_badge = "Song"
        base_right = item.album or kind_badge
        if item.plays:
            base_right = f"{base_right} · {item.plays}" if base_right else item.plays
        if role == "next":
            base_right = f"Next · {base_right}" if base_right else "Next"
        return MediaListItem(
            top_left=item.title,
            bottom_left=item.artist_names,
            top_right=item.duration_text if item.duration else "",
            bottom_right=base_right,
            payload=item,
            cover_url=item.cover_url,
            kind=kind,
            role=role,
        )
    if isinstance(item, AlbumSummary):
        art = ", ".join(a.name for a in item.artists) or "Unknown artist"
        return MediaListItem(
            top_left=item.title,
            bottom_left=art,
            top_right=item.year or "",
            bottom_right=item.kind,
            payload=item,
            cover_url=item.cover_url,
            kind=kind,
        )
    if isinstance(item, ArtistSummary):
        return MediaListItem(
            top_left=item.name,
            bottom_left="Artist",
            top_right="",
            bottom_right=item.subtitle or "",
            payload=item,
            cover_url=item.cover_url,
            kind=kind,
        )
    if isinstance(item, PlaylistSummary):
        cnt = f"{item.count} tracks" if item.count and item.count.isdigit() else (item.count or "")
        return MediaListItem(
            top_left=item.title,
            bottom_left=item.author or "YouTube Music",
            top_right=cnt,
            bottom_right="Playlist",
            payload=item,
            cover_url=item.cover_url,
            kind=kind,
        )
    return MediaListItem(
        str(item), "", "", "", item, cover_url=getattr(item, "cover_url", None), kind=kind
    )


_QUADRANTS = {0: " ", 1: "▘", 2: "▝", 3: "▀", 4: "▖", 5: "▌", 6: "▞", 7: "▛"}


def _mean(pixels: list[tuple[int, int, int]]) -> tuple[int, int, int]:
    n = max(1, len(pixels))
    return (
        sum(p[0] for p in pixels) // n,
        sum(p[1] for p in pixels) // n,
        sum(p[2] for p in pixels) // n,
    )


def _quadrant_cell(
    block: list[tuple[int, int, int]],
) -> tuple[str, tuple[int, int, int], tuple[int, int, int]]:
    """Pick the best 2-colour quadrant glyph for a 2x2 pixel block [UL, UR, LL, LR]."""
    best: tuple[int, int, tuple[int, int, int], tuple[int, int, int]] | None = None
    for mask in range(8):  # masks 0..7; the complement is covered by swapping fg/bg
        fg_px = [block[i] for i in range(4) if mask & (1 << i)]
        bg_px = [block[i] for i in range(4) if not mask & (1 << i)]
        fg = _mean(fg_px) if fg_px else _mean(bg_px)
        bg = _mean(bg_px) if bg_px else fg
        err = 0
        for i in range(4):
            c = fg if mask & (1 << i) else bg
            err += sum((block[i][k] - c[k]) ** 2 for k in range(3))
        if best is None or err < best[0]:
            best = (err, mask, fg, bg)
    assert best is not None
    _, mask, fg, bg = best
    return _QUADRANTS[mask], fg, bg


class MediaList(Widget):
    """Focus-aware, zero-bg 2-line list with `▸` cursor, `▶` active track, and mini icons."""

    can_focus = True

    DEFAULT_CSS = """
    MediaList {
        background: transparent;
        height: auto;
        width: 1fr;
    }
    """

    class ItemActivated(Message):
        def __init__(self, widget: MediaList, index: int, payload: Any) -> None:
            super().__init__()
            self.widget = widget
            self.index = index
            self.payload = payload

    class ItemPlay(Message):
        def __init__(self, widget: MediaList, index: int, payload: Any) -> None:
            super().__init__()
            self.widget = widget
            self.index = index
            self.payload = payload

    class ItemQueued(Message):
        def __init__(self, widget: MediaList, index: int, payload: Any) -> None:
            super().__init__()
            self.widget = widget
            self.index = index
            self.payload = payload

    class ItemPlayNext(Message):
        def __init__(self, widget: MediaList, index: int, payload: Any) -> None:
            super().__init__()
            self.widget = widget
            self.index = index
            self.payload = payload

    class ItemOpenAlbum(Message):
        def __init__(self, widget: MediaList, index: int, payload: Any) -> None:
            super().__init__()
            self.widget = widget
            self.index = index
            self.payload = payload

    class ItemOpenArtist(Message):
        def __init__(self, widget: MediaList, index: int, payload: Any) -> None:
            super().__init__()
            self.widget = widget
            self.index = index
            self.payload = payload

    class ItemRemoved(Message):
        def __init__(self, widget: MediaList, index: int, payload: Any) -> None:
            super().__init__()
            self.widget = widget
            self.index = index
            self.payload = payload

    class ItemSaved(Message):
        def __init__(self, widget: MediaList, index: int, payload: Any) -> None:
            super().__init__()
            self.widget = widget
            self.index = index
            self.payload = payload

    class CursorChanged(Message):
        def __init__(self, widget: MediaList, index: int, payload: Any) -> None:
            super().__init__()
            self.widget = widget
            self.index = index
            self.payload = payload

    def __init__(
        self,
        *,
        icons: IconSet = UNICODE_ICONS,
        empty_text: str = "No items to display",
        images: ImageService | None = None,
        protocol: str = "auto",
        allow_a_queue: bool = False,
        **kw,
    ) -> None:
        super().__init__(**kw)
        self._images = images
        self._protocol = protocol
        self._thumb_cache: dict[str, tuple[Text, ...]] = {}
        self._tgp_cache: dict[str, Any] = {}
        self._thumb_requested: set[str] = set()
        self._thumb_tasks: set[asyncio.Task[None]] = set()
        self.icons = icons
        self.empty_text = empty_text
        self.allow_a_queue = allow_a_queue
        self.items: list[MediaListItem] = []
        self.cursor: int = 0
        self.playing_index: int | None = None
        self._last_click_idx: int | None = None
        self._thumb_ver: int = 0
        self._items_ver: int = 0
        self._strip_sig: tuple[object, ...] | None = None
        self._strip_cache: list[Strip] = []

    @property
    def show_thumbs(self) -> bool:
        return self._images is not None

    @property
    def row_h(self) -> int:
        """Rows per item including the 1-line gap."""
        return THUMB_H + 1

    def _clear_thumb_cache(self) -> None:
        self._thumb_cache.clear()
        for tgp in self._tgp_cache.values():
            with contextlib.suppress(OSError, RuntimeError, AttributeError):
                tgp.cleanup()
        self._tgp_cache.clear()
        self._thumb_ver += 1

    def _remember_thumb(self, url: str, result: tuple[Text, ...]) -> None:
        self._thumb_cache[url] = result
        while len(self._thumb_cache) > THUMB_CACHE_MAX:
            oldest = next(iter(self._thumb_cache))
            del self._thumb_cache[oldest]
            tgp = self._tgp_cache.pop(oldest, None)
            if tgp is not None:
                with contextlib.suppress(OSError, RuntimeError, AttributeError):
                    tgp.cleanup()

    def set_protocol(self, protocol: str) -> None:
        if protocol != self._protocol:
            self._protocol = protocol
            self._clear_thumb_cache()
            self.refresh()

    def _request_thumbs(self, center: int | None = None) -> None:
        if self._images is None or not self.is_mounted or not self.items:
            return
        try:
            asyncio.get_running_loop()
        except RuntimeError:
            return
        c = self.cursor if center is None else center
        lo = max(0, c - 15)
        hi = min(len(self.items), c + 45)
        for it in self.items[lo:hi]:
            url = it.cover_url
            if not url:
                continue
            if self._images.get_cached(url) is not None:
                continue
            if url in self._thumb_requested:
                continue
            self._thumb_requested.add(url)
            task = asyncio.create_task(self._fetch_thumb(url))
            self._thumb_tasks.add(task)
            task.add_done_callback(self._thumb_tasks.discard)

    async def _fetch_thumb(self, url: str) -> None:
        assert self._images is not None
        try:
            await self._images.fetch(url)
        except asyncio.CancelledError:
            raise
        except (OSError, httpx.HTTPError, ValueError, RuntimeError):
            logger.debug("Thumbnail fetch failed for %s", url, exc_info=True)
            return
        finally:
            self._thumb_requested.discard(url)
        self._thumb_ver += 1
        if self.is_mounted:
            self.refresh()

    def _thumb_lines(self, url: str | None) -> tuple[Text, ...]:
        """THUMB_H terminal rows using TGP pixel graphics or quadrant-block fallback."""
        placeholder = tuple(Text("▒" * THUMB_W, style=SKEL_DARK) for _ in range(THUMB_H))
        if not url or self._images is None:
            return placeholder
        hit = self._thumb_cache.get(url)
        if hit is not None:
            return hit
        img = self._images.get_cached(url)
        if img is None:
            return placeholder
        try:
            from textual_image._terminal import get_cell_size
            from textual_image.renderable.tgp import Image as TGPImage

            rgb = square_crop(img.convert("RGB"))
            cls = renderable_for(self._protocol)
            if cls is TGPImage:
                cs = get_cell_size()
                pw = max(16, THUMB_W * max(8, cs.width))
                ph = max(16, THUMB_H * max(16, cs.height))
                scaled = rgb.resize((pw, ph), PILImage.Resampling.LANCZOS, reducing_gap=3.0)
                tgp = TGPImage(scaled, width=THUMB_W, height=THUMB_H)
                cw, ch = tgp._render_size.get_cell_size(THUMB_W, THUMB_H, cs)
                tgp._send_image_to_terminal(pw, ph)
                tgp._create_virtual_placement(cw, ch)
                result = tuple(
                    Text(seg.text.rstrip("\n"), style=seg.style)
                    for seg in tgp._render_diacritics(cw, ch)
                )
                self._tgp_cache[url] = tgp
            else:
                rgb = rgb.resize(
                    (THUMB_W * 2, THUMB_H * 2), PILImage.Resampling.LANCZOS, reducing_gap=3.0
                )
                px = rgb.load()
                rows = []
                for line in range(THUMB_H):
                    t = Text()
                    for x in range(THUMB_W):
                        block = [
                            px[x * 2, line * 2],
                            px[x * 2 + 1, line * 2],
                            px[x * 2, line * 2 + 1],
                            px[x * 2 + 1, line * 2 + 1],
                        ]
                        glyph, fg, bg = _quadrant_cell(block)
                        fg_hex = f"#{fg[0]:02x}{fg[1]:02x}{fg[2]:02x}"
                        bg_hex = f"#{bg[0]:02x}{bg[1]:02x}{bg[2]:02x}"
                        t.append(glyph, style=f"{fg_hex} on {bg_hex}")
                    rows.append(t)
                result = tuple(rows)
        except (OSError, ValueError, RuntimeError, ImportError, PILImage.DecompressionBombError):
            logger.debug("Thumbnail render failed for %s", url, exc_info=True)
            return placeholder
        self._remember_thumb(url, result)
        return result

    def set_icons(self, icons: IconSet) -> None:
        self.icons = icons
        self.refresh()

    def set_empty_text(self, text: str) -> None:
        self.empty_text = text
        if not self.items:
            self.refresh()

    def set_items(
        self,
        items: Sequence[MediaListItem],
        *,
        playing_index: int | None = None,
        reset_cursor: bool = False,
        emit_cursor: bool = False,
    ) -> None:
        self.items = list(items)
        self._items_ver += 1
        self.playing_index = playing_index
        if not self.items or reset_cursor:
            self.cursor = 0
        else:
            self.cursor = max(0, min(self.cursor, len(self.items) - 1))
        self._request_thumbs()
        self.refresh(layout=True)
        if emit_cursor and self.items:
            self.post_message(
                self.CursorChanged(self, self.cursor, self.items[self.cursor].payload)
            )

    def set_playing_index(self, idx: int | None) -> None:
        if idx != self.playing_index:
            self.playing_index = idx
            self.refresh()

    def on_mount(self) -> None:
        self._request_thumbs()

    def on_unmount(self) -> None:
        for task in list(self._thumb_tasks):
            if not task.done():
                task.cancel()
        self._thumb_tasks.clear()
        self._thumb_requested.clear()
        self._clear_thumb_cache()

    def get_content_height(self, container, viewport, width: int) -> int:
        return max(1, len(self.items) * self.row_h - 1)

    def _effective_width(self) -> int:
        w = max(24, self.size.width or 60)
        if self.parent is not None and getattr(self.parent, "show_vertical_scrollbar", False):
            w = max(24, w - 1)
        return w

    def render(self) -> Text:
        w = self._effective_width()
        out = Text(no_wrap=True, overflow="ignore")

        if not self.items:
            out.append(f"  {self.empty_text}", style=DIM)
            return out

        thumbs = self.show_thumbs
        if thumbs:
            w = max(12, w - THUMB_GUTTER)
        for idx, item in enumerate(self.items):
            if idx > 0:
                out.append("\n\n")
            row = Text(no_wrap=True, overflow="ignore")
            is_cur = idx == self.cursor
            is_play = idx == self.playing_index

            kind_glyph = self.icons.icon_for_kind(item.kind)
            icon_prefix = f"{kind_glyph} " if kind_glyph and w >= 18 else ""
            icon_w = cell_len(icon_prefix)

            l1, r1, gap1 = split_row_cells(
                item.top_left,
                item.top_right,
                w,
                gutter=2 + icon_w,
                right_cap_ratio=0.30,
            )
            l2, r2, gap2 = split_row_cells(
                item.bottom_left,
                item.bottom_right,
                w,
                gutter=2,
                right_cap_ratio=0.42,
            )

            if is_cur:
                bg = CURSOR_BG if self.has_focus else CURSOR_BLUR_BG
                prefix = f"{self.icons.playing} " if is_play else f"{self.icons.cursor} "
                raw1 = prefix + icon_prefix + l1 + (" " * gap1) + r1
                raw2 = "  " + l2 + (" " * gap2) + r2
                line1 = align_cells(raw1, w, "l")
                line2 = align_cells(raw2, w, "l")
                st1 = f"bold {ACCENT if is_play else TEXT} on {bg}"
                st2 = f"#b8c9c5 on {bg}"
                row.append(line1, style=st1)
                row.append("\n")
                row.append(line2, style=st2)
            elif is_play:
                row.append(f"{self.icons.playing} ", style=f"bold {ACCENT}")
                if icon_prefix:
                    row.append(icon_prefix, style=ACCENT)
                row.append(l1, style=f"bold {ACCENT}")
                if r1:
                    row.append(" " * gap1)
                    row.append(r1, style=ACCENT)
                row.append("\n")
                row.append("  " + l2, style=TEXT)
                if r2:
                    row.append(" " * gap2)
                    row.append(r2, style=DIM)
            else:
                is_played = item.role == "played"
                is_next = item.role == "next"
                t_style = TR_PAST if is_played else (TR_ACTIVE if is_next else TEXT)
                sub_style = TR_PAST if is_played else (TR_UPCOMING if is_next else DIM)
                tag_style = TR_PAST if is_played else (ACCENT if is_next else DIM)
                glyph_style = TR_PAST if is_played else ACCENT

                row.append("  ", style=t_style)
                if icon_prefix:
                    row.append(icon_prefix, style=glyph_style)
                row.append(l1, style=t_style)
                if r1:
                    row.append(" " * gap1)
                    row.append(r1, style=sub_style)
                row.append("\n")
                row.append("  " + l2, style=sub_style)
                if r2:
                    row.append(" " * gap2)
                    row.append(r2, style=tag_style)

            if thumbs:
                tl = self._thumb_lines(item.cover_url)
                l1_txt, l2_txt = row.split("\n", allow_blank=True)[:2]
                texts = [l1_txt, l2_txt] + [Text("") for _ in range(THUMB_H - 2)]
                for i, (t, body) in enumerate(zip(tl, texts, strict=False)):
                    if i:
                        out.append("\n")
                    out.append_text(t)
                    out.append(" ")
                    out.append_text(body)
            else:
                out.append_text(row)

        return out

    def _render_strips(self) -> list[Strip]:
        eff_w = self._effective_width()
        sig = (
            eff_w,
            self.has_focus,
            self.cursor,
            self.playing_index,
            self._thumb_ver,
            self._items_ver,
            self.empty_text,
            id(self.icons),
            self._protocol,
            self.show_thumbs,
        )
        if sig == self._strip_sig:
            return self._strip_cache
        console = self.app.console if self.is_attached else _FALLBACK_CONSOLE
        segs = list(self.render().render(console))
        self._strip_cache = [
            Strip(line)
            for line in Segment.split_and_crop_lines(
                segs, eff_w, include_new_lines=False, pad=False
            )
        ]
        self._strip_sig = sig
        return self._strip_cache

    def render_line(self, y: int) -> Strip:
        strips = self._render_strips()
        if 0 <= y < len(strips):
            return strips[y]
        return Strip.blank(max(24, self.size.width or 60), self.visual_style.rich_style)

    def set_cursor(self, index: int, *, scroll: bool = True) -> None:
        if not self.items:
            return
        nxt = max(0, min(len(self.items) - 1, index))
        if nxt != self.cursor:
            self.cursor = nxt
            self._request_thumbs(nxt)
            self.refresh()
        if scroll:
            self._scroll_cursor_into_view()

    def move_cursor(self, delta: int) -> None:
        if not self.items:
            return
        nxt = max(0, min(len(self.items) - 1, self.cursor + delta))
        if nxt != self.cursor:
            self.cursor = nxt
            self._request_thumbs(nxt)
            self.refresh()
            self.post_message(
                self.CursorChanged(self, self.cursor, self.items[self.cursor].payload)
            )
            self._scroll_cursor_into_view()

    def _scroll_cursor_into_view(self) -> None:
        if not self.items or not self.is_mounted:
            return
        scroll_child_region(
            self,
            Region(
                0,
                self.cursor * self.row_h,
                max(1, self.size.width),
                THUMB_H if self.show_thumbs else 2,
            ),
            animate=False,
        )

    @property
    def current_payload(self) -> Any | None:
        if 0 <= self.cursor < len(self.items):
            return self.items[self.cursor].payload
        return None

    def on_key(self, event: Key) -> None:
        if not self.items:
            return
        k = event.key
        if k in ("down", "j"):
            self.move_cursor(1)
            event.stop()
        elif k in ("up", "k"):
            self.move_cursor(-1)
            event.stop()
        elif k in ("home", "g"):
            self.move_cursor(-len(self.items))
            event.stop()
        elif k in ("end", "G"):
            self.move_cursor(len(self.items))
            event.stop()
        elif k == "pagedown":
            self.move_cursor(6)
            event.stop()
        elif k == "pageup":
            self.move_cursor(-6)
            event.stop()
        elif k == "enter":
            self.post_message(
                self.ItemActivated(self, self.cursor, self.items[self.cursor].payload)
            )
            event.stop()
        elif k == "p":
            self.post_message(self.ItemPlay(self, self.cursor, self.items[self.cursor].payload))
            event.stop()
        elif k == "plus":
            self.post_message(self.ItemQueued(self, self.cursor, self.items[self.cursor].payload))
            event.stop()
        elif k == "N":
            self.post_message(self.ItemPlayNext(self, self.cursor, self.items[self.cursor].payload))
            event.stop()
        elif k == "a":
            if self.allow_a_queue:
                self.post_message(
                    self.ItemQueued(self, self.cursor, self.items[self.cursor].payload)
                )
            else:
                self.post_message(
                    self.ItemOpenAlbum(self, self.cursor, self.items[self.cursor].payload)
                )
            event.stop()
        elif k == "o" and self.allow_a_queue:
            self.post_message(
                self.ItemOpenAlbum(self, self.cursor, self.items[self.cursor].payload)
            )
            event.stop()
        elif k == "A":
            self.post_message(
                self.ItemOpenArtist(self, self.cursor, self.items[self.cursor].payload)
            )
            event.stop()
        elif k in ("d", "delete"):
            self.post_message(self.ItemRemoved(self, self.cursor, self.items[self.cursor].payload))
            event.stop()
        elif k == "w":
            self.post_message(self.ItemSaved(self, self.cursor, self.items[self.cursor].payload))
            event.stop()

    def on_click(self, event: Click) -> None:
        self.focus()
        idx = event.y // self.row_h
        if 0 <= idx < len(self.items):
            was_same = idx == self.cursor and self._last_click_idx == idx
            self.cursor = idx
            self._last_click_idx = idx
            self.refresh()
            self.post_message(self.CursorChanged(self, idx, self.items[idx].payload))
            if was_same or event.chain >= 2:
                self.post_message(self.ItemActivated(self, idx, self.items[idx].payload))
