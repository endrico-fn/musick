"""Async album/artist cover downloader with memory LRU and opt-in disk cache."""

from __future__ import annotations

import asyncio
import hashlib
import io
import logging
from collections import OrderedDict
from pathlib import Path

import httpx
from PIL import Image as PILImage

from musick.config import Settings
from musick.paths import IMAGE_CACHE_LIMITS, image_cache_dir, prune_cache_dir

logger = logging.getLogger(__name__)
_IMAGE_ERRORS = (OSError, ValueError, SyntaxError, PILImage.DecompressionBombError)


def _disk_path(url: str) -> Path:
    digest = hashlib.sha256(url.encode("utf-8")).hexdigest()[:32]
    return image_cache_dir() / f"{digest}.png"


class ImageService:
    """Fetches cover art without cookies, decodes via Pillow, and caches in memory/disk."""

    def __init__(
        self,
        settings: Settings,
        http: httpx.AsyncClient | None = None,
        max_memory_items: int = 96,
        max_concurrency: int = 6,
    ) -> None:
        self._settings = settings
        self._http = http
        self._owns_http = http is None
        self._max_mem = max_memory_items
        self._sem = asyncio.Semaphore(max_concurrency)
        self._mem: OrderedDict[str, PILImage.Image] = OrderedDict()
        self._inflight: dict[str, asyncio.Task[PILImage.Image | None]] = {}
        self._disk_writes = 0

    def _client(self) -> httpx.AsyncClient:
        if self._http is None:
            self._http = httpx.AsyncClient(timeout=8.0, follow_redirects=True, http2=True)
        return self._http

    async def aclose(self) -> None:
        if self._owns_http and self._http is not None:
            await self._http.aclose()
            self._http = None

    def clear_cache(self) -> None:
        self._mem.clear()

    def get_cached(self, url: str) -> PILImage.Image | None:
        img = self._mem.get(url)
        if img is not None:
            self._mem.move_to_end(url)
        return img

    async def fetch(self, url: str | None) -> PILImage.Image | None:
        if not url:
            return None
        cached = self.get_cached(url)
        if cached is not None:
            return cached

        task = self._inflight.get(url)
        if task is None:
            task = asyncio.create_task(self._load(url))
            self._inflight[url] = task
            task.add_done_callback(
                lambda t, u=url: self._inflight.pop(u, None) if self._inflight.get(u) is t else None
            )
        return await asyncio.shield(task)

    def _remember(self, url: str, img: PILImage.Image) -> None:
        self._mem[url] = img
        self._mem.move_to_end(url)
        while len(self._mem) > self._max_mem:
            self._mem.popitem(last=False)

    async def _load(self, url: str) -> PILImage.Image | None:
        if self._settings.privacy.cache_images:
            path = _disk_path(url)
            if path.exists():
                img = await asyncio.to_thread(self._open_file, path)
                if img is not None:
                    self._remember(url, img)
                    return img

        async with self._sem:
            try:
                client = self._client()
                client.cookies.clear()
                resp = await client.get(url)
                if resp.status_code != 200 or not resp.content:
                    return None
                raw = resp.content
            except httpx.HTTPError:
                return None

        img = await asyncio.to_thread(self._decode_bytes, raw)
        if img is None:
            return None
        self._remember(url, img)
        if self._settings.privacy.cache_images:
            await asyncio.to_thread(self._write_disk, _disk_path(url), img)
            self._disk_writes += 1
            if self._disk_writes % 32 == 0:
                max_bytes, max_files = IMAGE_CACHE_LIMITS
                await asyncio.to_thread(
                    prune_cache_dir,
                    image_cache_dir(),
                    max_bytes=max_bytes,
                    max_files=max_files,
                )
        return img

    @staticmethod
    def _decode_bytes(data: bytes) -> PILImage.Image | None:
        try:
            im = PILImage.open(io.BytesIO(data))
            im.load()
            if im.mode not in ("RGB", "RGBA"):
                im = im.convert("RGB")
        except _IMAGE_ERRORS:
            logger.debug("Failed decoding image bytes", exc_info=True)
            return None
        return im

    @staticmethod
    def _open_file(path: Path) -> PILImage.Image | None:
        try:
            with PILImage.open(path) as im:
                im.load()
                return im.copy()
        except _IMAGE_ERRORS:
            logger.debug("Failed reading cached image %s", path, exc_info=True)
            return None

    @staticmethod
    def _write_disk(path: Path, img: PILImage.Image) -> None:
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            img.save(path, format="PNG")
        except (OSError, ValueError):
            logger.debug("Failed writing cached image %s", path, exc_info=True)
