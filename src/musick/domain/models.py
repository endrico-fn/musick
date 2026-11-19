"""Immutable domain models shared by services and UI. No I/O in here."""

from __future__ import annotations

import re
from dataclasses import dataclass, field


@dataclass(frozen=True, slots=True)
class Thumb:
    url: str
    width: int = 0
    height: int = 0


_GOOG_SIZE = re.compile(r"=w\d+-h\d+(-[a-zA-Z0-9-]+)?$")


def best_thumb_url(thumbs: tuple[Thumb, ...], px: int = 300) -> str | None:
    """Pick the smallest thumbnail >= px, else the largest; upscale google CDN urls."""
    if not thumbs:
        return None
    sized = [t for t in thumbs if t.width]
    if sized:
        big = [t for t in sized if t.width >= px]
        pick = min(big, key=lambda t: t.width) if big else max(sized, key=lambda t: t.width)
    else:
        pick = thumbs[-1]
    url = pick.url
    if ("googleusercontent.com" in url or "ggpht.com" in url) and (m := _GOOG_SIZE.search(url)):
        flags = (m.group(1) or "").split("-")
        crop = "-p" if "p" in flags else ""
        url = _GOOG_SIZE.sub(f"=w{px}-h{px}{crop}-l90-rj", url)
    return url


@dataclass(frozen=True, slots=True)
class ArtistRef:
    name: str
    id: str | None = None


def format_duration(seconds: int | None) -> str:
    if not seconds or seconds < 0:
        return "--:--"
    m, s = divmod(int(seconds), 60)
    h, m = divmod(m, 60)
    return f"{h}:{m:02d}:{s:02d}" if h else f"{m}:{s:02d}"


@dataclass(frozen=True, slots=True)
class Track:
    video_id: str
    title: str
    artists: tuple[ArtistRef, ...] = ()
    album: str | None = None
    album_id: str | None = None
    duration: int | None = None
    thumbs: tuple[Thumb, ...] = ()
    plays: str | None = None
    video_type: str | None = None

    @property
    def artist_names(self) -> str:
        return ", ".join(a.name for a in self.artists) or "Unknown artist"

    @property
    def duration_text(self) -> str:
        return format_duration(self.duration)

    @property
    def url(self) -> str:
        return f"https://music.youtube.com/watch?v={self.video_id}"

    @property
    def cover_url(self) -> str | None:
        return best_thumb_url(self.thumbs)


@dataclass(frozen=True, slots=True)
class AlbumSummary:
    browse_id: str
    title: str
    year: str | None = None
    kind: str = "Album"
    artists: tuple[ArtistRef, ...] = ()
    thumbs: tuple[Thumb, ...] = ()

    @property
    def subtitle(self) -> str:
        return " · ".join(p for p in (self.year, self.kind) if p)

    @property
    def cover_url(self) -> str | None:
        return best_thumb_url(self.thumbs)


@dataclass(frozen=True, slots=True)
class Album:
    summary: AlbumSummary
    tracks: tuple[Track, ...] = ()
    description: str | None = None
    duration_text: str | None = None


@dataclass(frozen=True, slots=True)
class ArtistSummary:
    browse_id: str
    name: str
    subtitle: str | None = None
    thumbs: tuple[Thumb, ...] = ()

    @property
    def cover_url(self) -> str | None:
        return best_thumb_url(self.thumbs)


@dataclass(frozen=True, slots=True)
class PlaylistSummary:
    browse_id: str
    title: str
    author: str | None = None
    count: str | None = None
    thumbs: tuple[Thumb, ...] = ()

    @property
    def cover_url(self) -> str | None:
        return best_thumb_url(self.thumbs)


@dataclass(frozen=True, slots=True)
class Playlist:
    summary: PlaylistSummary
    tracks: tuple[Track, ...] = ()
    description: str | None = None
    views: str | None = None
    duration_text: str | None = None
    year: str | None = None
    total: int | None = None


@dataclass(frozen=True, slots=True)
class Artist:
    summary: ArtistSummary
    description: str | None = None
    listeners: str | None = None
    songs: tuple[Track, ...] = ()
    albums: tuple[AlbumSummary, ...] = ()
    singles: tuple[AlbumSummary, ...] = ()
    playlists: tuple[PlaylistSummary, ...] = ()
    related: tuple[ArtistSummary, ...] = ()
    albums_params: str | None = None
    singles_params: str | None = None
    radio_id: str | None = None
    songs_playlist_id: str | None = None
    videos: tuple[Track, ...] = ()
    subscribers: str | None = None
    views: str | None = None
    albums_browse_id: str | None = None
    singles_browse_id: str | None = None
    videos_playlist_id: str | None = None
    shuffle_id: str | None = None


SearchItem = Track | AlbumSummary | ArtistSummary | PlaylistSummary


_TITLE_NOISE_FEAT_RE = re.compile(
    r"\s*[\(\[](?:feat\.?|ft\.?|with\s+[^\)\]]+|official\s*(?:music\s*)?video|official\s*audio|"
    r"lyric\s*video|lyrics|audio|hd|4k|mv|visualizer|explicit|remaster(?:ed)?(?:\s*\d{4})?)[^\)\]]*[\)\]]",
    re.IGNORECASE,
)

_TITLE_NOISE_CORE_RE = re.compile(
    r"\s*[\(\[](?:official\s*(?:music\s*)?video|official\s*audio|"
    r"lyric\s*video|lyrics|audio|hd|4k|mv|visualizer|explicit|remaster(?:ed)?(?:\s*\d{4})?)[^\)\]]*[\)\]]",
    re.IGNORECASE,
)


def clean_media_title(title: str, *, strip_feat: bool = True) -> str:
    """Strip common YouTube/music noise brackets while falling back to the original title."""
    pattern = _TITLE_NOISE_FEAT_RE if strip_feat else _TITLE_NOISE_CORE_RE
    cleaned = pattern.sub("", title).strip()
    return cleaned or title.strip() or title


def search_item_key(item: SearchItem) -> tuple[str, str]:
    """Return a stable deduplication key for any SearchItem variant."""
    if isinstance(item, Track):
        return ("t", item.video_id)
    if isinstance(item, AlbumSummary):
        return ("al", item.browse_id)
    if isinstance(item, ArtistSummary):
        return ("ar", item.browse_id)
    if isinstance(item, PlaylistSummary):
        return ("pl", item.browse_id)
    return ("x", str(item))


@dataclass(frozen=True, slots=True)
class LyricLine:
    time: float
    text: str
    translation: str = ""


@dataclass(frozen=True, slots=True)
class Lyrics:
    lines: tuple[LyricLine, ...] = field(default_factory=tuple)
    synced: bool = False
    source: str = ""
    translation_lang: str = ""
    times: tuple[float, ...] = ()

    def __post_init__(self) -> None:
        # Precompute the timestamp vector once so per-tick lookups stay O(log n)
        # without rebuilding a list for every playback update.
        if self.synced and not self.times and self.lines:
            object.__setattr__(self, "times", tuple(ln.time for ln in self.lines))

    def __bool__(self) -> bool:
        return bool(self.lines)

    @property
    def has_translations(self) -> bool:
        return any(ln.translation for ln in self.lines)


@dataclass(frozen=True, slots=True)
class HomeFeed:
    featured: tuple[AlbumSummary | PlaylistSummary, ...] = ()
    artists: tuple[ArtistSummary, ...] = ()
    playlists: tuple[PlaylistSummary, ...] = ()
