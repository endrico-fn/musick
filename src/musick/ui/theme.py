"""Visual tokens, Textual theme (transparent native bg), and verified JetBrains icons."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from textual.theme import Theme

from musick.domain import AlbumSummary, ArtistSummary, PlaylistSummary, Track

ACCENT = "#26d9b4"
ACCENT_BRIGHT = "#4ef0ce"
BORDER = "#a6a6a6"
BORDER_DIM = "#7c7c7c"
TEXT = "#f2f2f2"
DIM = "#7c7c7c"
TR_ACTIVE = "#9ef5e2"
TR_UPCOMING = "#7da8a0"
TR_PAST = "#556663"
CURSOR_BG = "#1f4f46"
CURSOR_BLUR_BG = "#16352f"
BTN_BG = "#1d3833"
BTN_HOVER_BG = "#255249"
BTN_PRIMARY_FG = "#08130f"
ERR = "#e06c75"

SKEL_DARK = "#3a3a3a"
SKEL_MID = "#5e5e5e"
SKEL_LIGHT = "#a0a0a0"

MUSICK_THEME = Theme(
    name="musick",
    primary=ACCENT,
    secondary=BORDER,
    accent=ACCENT,
    warning="#e0af68",
    error=ERR,
    success=ACCENT,
    foreground=TEXT,
    background="ansi_default",
    surface="ansi_default",
    panel="ansi_default",
    boost="ansi_default",
    dark=True,
    ansi=True,  # keep ansi_default native so the terminal's own bg shows through
    variables={
        "ansi-background": "ansi_default",
        "ansi-foreground": "ansi_default",
        "border": BORDER,
        "border-blurred": BORDER_DIM,
        "text-muted": DIM,
        "accent-lighten-1": ACCENT_BRIGHT,
        "primary-muted": BTN_BG,
        "block-hover-background": BTN_HOVER_BG,
        "block-cursor-background": CURSOR_BG,
        "block-cursor-blurred-background": CURSOR_BLUR_BG,
        "block-cursor-foreground": TEXT,
        "button-color-foreground": BTN_PRIMARY_FG,
        "input-cursor-background": ACCENT,
        "input-cursor-foreground": BTN_PRIMARY_FG,
        "input-selection-background": CURSOR_BG,
        "input-selection-foreground": TEXT,
        "scrollbar": BORDER_DIM,
        "scrollbar-hover": ACCENT,
        "scrollbar-active": ACCENT,
        "scrollbar-background": "ansi_default",
        "scrollbar-background-hover": "ansi_default",
        "scrollbar-background-active": "ansi_default",
    },
)


@dataclass(frozen=True, slots=True)
class IconSet:
    brand: str
    cursor: str
    playing: str
    pause: str
    prev: str
    next: str
    song: str
    video: str
    album: str
    artist: str
    playlist: str
    prompt: str
    dot: str
    lyrics: str
    shuffle: str
    repeat_off: str
    repeat_all: str
    repeat_one: str
    volume: str
    sort_desc: str
    check_on: str
    check_off: str

    def icon_for_kind(self, kind: str) -> str:
        if kind == "song":
            return self.song
        if kind == "video":
            return self.video
        if kind == "album":
            return self.album
        if kind == "artist":
            return self.artist
        if kind == "playlist":
            return self.playlist
        return ""


UNICODE_ICONS = IconSet(
    brand="◆",
    cursor="▸",
    playing="▶",
    pause="▌▌",
    prev="◀◀",
    next="▶▶",
    song="♪",
    video="▷",
    album="◎",
    artist="◉",
    playlist="≡",
    prompt="\u203a",
    dot="●",
    lyrics="▪",
    shuffle="SHF",
    repeat_off="RPT",
    repeat_all="RPT*",
    repeat_one="RPT1",
    volume="VOL",
    sort_desc="▾",
    check_on="[x]",
    check_off="[ ]",
)

NERD_ICONS = IconSet(
    brand="\uf001",
    cursor="▸",
    playing="\uf04b",
    pause="\uf04c",
    prev="◀◀",
    next="▶▶",
    song="\uf001",
    video="\uf03d",
    album="\uf192",
    artist="\uf007",
    playlist="\uf0ca",
    prompt="\uf002",
    dot="●",
    lyrics="\uf001",
    shuffle="SHF",
    repeat_off="\uf01e",
    repeat_all="\uf01e*",
    repeat_one="\uf01e1",
    volume="\uf028",
    sort_desc="▾",
    check_on="[✓]",
    check_off="[ ]",
)


def icons_for(mode: str) -> IconSet:
    return NERD_ICONS if mode == "nerd" else UNICODE_ICONS


_VIDEO_TYPES = frozenset(
    {
        "MUSIC_VIDEO_TYPE_OMV",
        "MUSIC_VIDEO_TYPE_UGC",
        "MUSIC_VIDEO_TYPE_VIDEO",
    }
)


def media_kind_for(item: Any) -> str:
    """Return media category key ('song', 'video', 'album', 'artist', 'playlist', or '')."""
    if isinstance(item, Track):
        return "video" if item.video_type in _VIDEO_TYPES else "song"
    if isinstance(item, AlbumSummary):
        return "album"
    if isinstance(item, ArtistSummary):
        return "artist"
    if isinstance(item, PlaylistSummary):
        return "playlist"
    if isinstance(item, dict):
        kind = str(item.get("kind") or "").strip().lower()
        if kind == "artist":
            return "artist"
        if kind == "playlist":
            return "playlist"
        if kind:
            return "album"
    return ""


def media_icon_for(item: Any, icons: IconSet) -> str:
    """Resolve the display mini icon glyph for a domain item or bookmark entry."""
    return icons.icon_for_kind(media_kind_for(item))
