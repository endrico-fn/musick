"""Filesystem locations (XDG via platformdirs) and local-data cleanup."""

from __future__ import annotations

import shutil
from pathlib import Path

from platformdirs import user_cache_path, user_config_path, user_data_path

APP = "musick"


def config_dir() -> Path:
    return user_config_path(APP)


def cache_dir() -> Path:
    return user_cache_path(APP)


def data_dir() -> Path:
    return user_data_path(APP)


def settings_file() -> Path:
    return config_dir() / "settings.toml"


def history_file() -> Path:
    return data_dir() / "history.json"


def bookmarks_file() -> Path:
    return data_dir() / "library.json"


def atomic_write_text(path: Path, text: str, *, mode: int = 0o600) -> None:
    """Atomically write UTF-8 text via a temporary sibling file with restrictive permissions."""
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(f"{path.name}.tmp")
    tmp.write_text(text, encoding="utf-8")
    tmp.chmod(mode)
    tmp.replace(path)


def image_cache_dir() -> Path:
    return cache_dir() / "images"


def lyrics_cache_dir() -> Path:
    return cache_dir() / "lyrics"


IMAGE_CACHE_LIMITS = (256 * 1024 * 1024, 2048)
LYRICS_CACHE_LIMITS = (32 * 1024 * 1024, 4096)


def _file_mtime_size(path: Path) -> tuple[float, int]:
    try:
        st = path.stat()
    except OSError:
        return 0.0, 0
    return st.st_mtime, st.st_size


def prune_cache_dir(directory: Path, *, max_bytes: int, max_files: int) -> int:
    """Delete oldest files until the cache fits both byte and file-count limits."""
    try:
        entries = [p for p in directory.iterdir() if p.is_file()]
    except OSError:
        return 0
    stats = sorted(
        ((*_file_mtime_size(p), p) for p in entries),
        key=lambda item: item[0],
    )
    total = sum(size for _, size, _ in stats)
    removed = 0
    idx = 0
    count = len(stats)
    while idx < count and (total > max_bytes or (count - idx) > max_files):
        _, size, victim = stats[idx]
        idx += 1
        try:
            victim.unlink()
            removed += 1
            total -= size
        except OSError:
            pass
    return removed


def clear_local_data(keep_settings: bool = True) -> list[Path]:
    """Remove history and caches. Returns the paths that were removed."""
    removed: list[Path] = []
    targets = [cache_dir(), data_dir()]
    if not keep_settings:
        targets.append(config_dir())
    for t in targets:
        if t.exists():
            shutil.rmtree(t, ignore_errors=True)
            removed.append(t)
    return removed
