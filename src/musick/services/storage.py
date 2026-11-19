"""Explicitly user-saved local bookmark persistence (privacy-safe)."""

from __future__ import annotations

import contextlib
import json
from pathlib import Path
from typing import Any

from musick.domain import AlbumSummary, ArtistSummary, PlaylistSummary
from musick.paths import atomic_write_text, bookmarks_file


class BookmarkStore:
    """Explicitly user-saved local bookmarks (written only when the user clicks Save)."""

    def __init__(self, path: Path | None = None) -> None:
        self._path = path or bookmarks_file()
        self.items: list[dict[str, str]] = self._load()

    def _load(self) -> list[dict[str, str]]:
        try:
            raw = json.loads(self._path.read_text(encoding="utf-8"))
            if isinstance(raw, list):
                return [x for x in raw if isinstance(x, dict) and x.get("browse_id")]
        except (OSError, ValueError):
            pass
        return []

    def _save(self) -> None:
        with contextlib.suppress(OSError):
            atomic_write_text(self._path, json.dumps(self.items, indent=2))

    def add(self, item: Any) -> bool:
        entry: dict[str, str] | None = None
        if isinstance(item, AlbumSummary):
            art = ", ".join(a.name for a in item.artists)
            entry = {
                "kind": item.kind,
                "browse_id": item.browse_id,
                "title": item.title,
                "info": art or (item.year or ""),
            }
        elif isinstance(item, PlaylistSummary):
            entry = {
                "kind": "Playlist",
                "browse_id": item.browse_id,
                "title": item.title,
                "info": item.author or "",
            }
        elif isinstance(item, ArtistSummary):
            entry = {
                "kind": "Artist",
                "browse_id": item.browse_id,
                "title": item.name,
                "info": item.subtitle or "",
            }
        if not entry:
            return False
        if any(x["browse_id"] == entry["browse_id"] for x in self.items):
            return False
        self.items.insert(0, entry)
        self._save()
        return True

    def remove(self, idx: int) -> None:
        if 0 <= idx < len(self.items):
            self.items.pop(idx)
            self._save()

    def clear(self) -> None:
        self.items.clear()
        if self._path.exists():
            with contextlib.suppress(OSError):
                self._path.unlink()
