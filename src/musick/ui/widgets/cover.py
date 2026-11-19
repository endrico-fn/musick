"""Multi-size square 1:1 album/artist cover widget with safe protocol fallback."""

from __future__ import annotations

import asyncio
import contextlib
import logging
import os
import sys
from typing import Any, Literal

import httpx
from PIL import Image as PILImage
from rich.text import Text
from textual.widget import Widget

from musick.services.images import ImageService
from musick.ui.theme import BORDER_DIM

from .loading import SHIMMER_CLOCK, shimmer_line

logger = logging.getLogger(__name__)

CoverSizeKey = Literal["S", "M", "L"]

_BASE_HEIGHTS: dict[str, int] = {
    "S": 4,
    "M": 8,
    "L": 11,
}

_TERMINAL_PROBED = False


def cell_aspect_ratio() -> float:
    """Return terminal cell height/width ratio so W = H * ratio forms a visual 1:1 square."""
    init_terminal_image_support()
    from textual_image._terminal import get_cell_size

    cs = get_cell_size()
    if cs.width > 0 and cs.height > 0:
        # VT340 default fallback is (10, 20); modern monospace fonts (e.g. JetBrains Mono) are ~2.25
        if (cs.width, cs.height) == (10, 20):
            return 2.25
        return max(1.5, min(3.0, cs.height / cs.width))
    return 2.25


def cover_dims(size: str) -> tuple[int, int]:
    """Return (cols, rows) sized to look 1:1 square on the active terminal."""
    h = _BASE_HEIGHTS.get(size, 8)
    w = max(4, round(h * cell_aspect_ratio()))
    return (w, h)


def fit_square_dims(max_w: int, max_h: int, *, min_w: int = 8, min_h: int = 4) -> tuple[int, int]:
    """Compute exact (cols, rows) forming a 1:1 visual square that fits inside (max_w, max_h)."""
    ratio = cell_aspect_ratio()
    mw = max(min_w, int(max_w))
    mh = max(min_h, int(max_h))
    h = mh
    while h > min_h and round(h * ratio) > mw:
        h -= 1
    w = min(mw, max(min_w, round(h * ratio)))
    return (w, h)


def init_terminal_image_support() -> None:
    """Safely seed or probe textual_image terminal capabilities before rendering images."""
    global _TERMINAL_PROBED
    if _TERMINAL_PROBED:
        return
    _TERMINAL_PROBED = True

    from textual_image import _terminal

    if not hasattr(_terminal.probe_terminal, "_result"):
        term = os.environ.get("TERM", "")
        in_pytest = "PYTEST_CURRENT_TEST" in os.environ
        is_tty = bool(
            sys.__stdout__ and sys.__stdout__.isatty() and sys.__stdin__ and sys.__stdin__.isatty()
        )
        if in_pytest or term in ("", "dumb") or not is_tty:
            _terminal.probe_terminal._result = _terminal.TerminalCapabilities(  # type: ignore[attr-defined]
                _terminal.CellSize(10, 20), sixel=False, tgp=False
            )
        else:
            try:
                _terminal.get_cell_size()
            except (
                _terminal.TerminalError,
                ZeroDivisionError,
                OSError,
                ValueError,
                RuntimeError,
                EOFError,
            ):
                _terminal.probe_terminal._result = _terminal.TerminalCapabilities(  # type: ignore[attr-defined]
                    _terminal.CellSize(10, 20), sixel=False, tgp=False
                )


def renderable_for(protocol: str):
    init_terminal_image_support()
    if protocol == "tgp":
        from textual_image.renderable.tgp import Image as TGPImage

        return TGPImage
    if protocol == "sixel":
        from textual_image.renderable.sixel import Image as SixelImage

        return SixelImage
    if protocol == "halfcell":
        from textual_image.renderable.halfcell import Image as HalfcellImage

        return HalfcellImage

    from textual_image import _terminal
    from textual_image.renderable.halfcell import Image as HalfcellImage
    from textual_image.renderable.sixel import Image as SixelImage
    from textual_image.renderable.tgp import Image as TGPImage

    caps = _terminal.probe_terminal()
    if caps.tgp:
        return TGPImage
    if caps.sixel:
        return SixelImage
    return HalfcellImage


def square_crop(img: PILImage.Image) -> PILImage.Image:
    w, h = img.size
    if w == h or w <= 0 or h <= 0:
        return img
    side = min(w, h)
    left = (w - side) // 2
    top = (h - side) // 2
    return img.crop((left, top, left + side, top + side))


class CoverWidget(Widget):
    """Displays a square 1:1 album/artist/playlist cover at S, M, or L size."""

    DEFAULT_CSS = """
    CoverWidget {
        background: transparent;
    }
    """

    def __init__(
        self,
        cover_size: str = "M",
        images: ImageService | None = None,
        protocol: str = "auto",
        **kw,
    ) -> None:
        super().__init__(**kw)
        self._cover_size = cover_size if cover_size in _BASE_HEIGHTS else "M"
        self._exact_dims: tuple[int, int] | None = None
        self._images = images
        self._protocol = protocol
        self._url: str | None = None
        self._pil: PILImage.Image | None = None
        self._loading: bool = False
        self._fetch_task: asyncio.Task[None] | None = None
        self._unsub: Any = None
        self._renderable: Any = None
        self._apply_dims()

    def dims(self) -> tuple[int, int]:
        return self._exact_dims if self._exact_dims is not None else cover_dims(self._cover_size)

    def _apply_dims(self) -> None:
        w, h = self.dims()
        self.styles.width = w
        self.styles.height = h

    def set_protocol(self, protocol: str) -> None:
        if protocol != self._protocol:
            self._protocol = protocol
            self._rebuild_renderable()
            self.refresh()

    def set_exact_size(self, cols: int, rows: int) -> None:
        target = (max(4, int(cols)), max(2, int(rows)))
        if target == self._exact_dims:
            return
        self._exact_dims = target
        self._apply_dims()
        self._rebuild_renderable()
        self.refresh(layout=True)

    def set_url(self, url: str | None) -> None:
        if url == self._url and (self._pil is not None or self._loading):
            return
        self._url = url
        self._pil = None
        self._cleanup_renderable()
        if self._fetch_task and not self._fetch_task.done():
            self._fetch_task.cancel()

        if not url or self._images is None:
            self._set_loading(False)
            self.refresh()
            return

        cached = self._images.get_cached(url)
        if cached is not None:
            self._pil = square_crop(cached)
            self._set_loading(False)
            self._rebuild_renderable()
            self.refresh()
            return

        try:
            asyncio.get_running_loop()
            self._set_loading(True)
            self._fetch_task = asyncio.create_task(self._load(url))
        except RuntimeError:
            self._set_loading(False)
        self.refresh()

    def _set_loading(self, active: bool) -> None:
        self._loading = active
        if active and self._unsub is None and self.is_mounted:
            self._unsub = SHIMMER_CLOCK.subscribe(self._on_tick)
        elif not active and self._unsub is not None:
            self._unsub()
            self._unsub = None

    def _on_tick(self, _tick: int) -> None:
        if self.display:
            self.refresh()

    def on_mount(self) -> None:
        if self._loading and self._unsub is None:
            self._unsub = SHIMMER_CLOCK.subscribe(self._on_tick)

    def on_unmount(self) -> None:
        if self._unsub is not None:
            self._unsub()
            self._unsub = None
        if self._fetch_task and not self._fetch_task.done():
            self._fetch_task.cancel()
        self._cleanup_renderable()

    async def _load(self, url: str) -> None:
        assert self._images is not None
        try:
            img = await self._images.fetch(url)
            if self._url != url:
                return
            self._pil = square_crop(img) if img is not None else None
            self._set_loading(False)
            self._rebuild_renderable()
            self.refresh()
        except asyncio.CancelledError:
            raise
        except (
            OSError,
            httpx.HTTPError,
            ValueError,
            RuntimeError,
            PILImage.DecompressionBombError,
        ):
            logger.debug("Cover image fetch failed for %s", url, exc_info=True)
            if self._url == url:
                self._set_loading(False)
                self.refresh()
        finally:
            if self._url == url and self._loading:
                self._set_loading(False)
                self.refresh()

    def _cleanup_renderable(self) -> None:
        if self._renderable is not None:
            with contextlib.suppress(OSError, RuntimeError, AttributeError):
                self._renderable.cleanup()
            self._renderable = None

    def _rebuild_renderable(self) -> None:
        self._cleanup_renderable()
        if self._pil is None:
            return
        w, h = self.dims()
        cls = renderable_for(self._protocol)
        try:
            self._renderable = cls(self._pil, width=w, height=h)
        except (OSError, ValueError, RuntimeError):
            logger.debug("Failed building %s cover renderable", self._protocol, exc_info=True)
            self._renderable = None

    def render(self):
        if self._renderable is not None:
            return self._renderable

        w, h = self.dims()
        if self._loading:
            out = Text()
            mask = "x" * w
            tick = SHIMMER_CLOCK.tick
            rm = SHIMMER_CLOCK.reduced_motion
            for y in range(h):
                out.append_text(shimmer_line(mask, y, tick, reduced_motion=rm))
                if y + 1 < h:
                    out.append("\n")
            return out

        out = Text()
        for y in range(h):
            if y == 0:
                out.append("┌" + "─" * (w - 2) + "┐", style=BORDER_DIM)
            elif y == h - 1:
                out.append("└" + "─" * (w - 2) + "┘", style=BORDER_DIM)
            else:
                inner = " " * (w - 2)
                if y == h // 2 and w >= 6:
                    mid = "◈"
                    pad_l = (w - 2 - len(mid)) // 2
                    inner = " " * pad_l + mid + " " * (w - 2 - pad_l - len(mid))
                out.append("│" + inner + "│", style=BORDER_DIM)
            if y + 1 < h:
                out.append("\n")
        return out
