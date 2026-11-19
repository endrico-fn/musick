"""Reusable Textual UI widgets for musick."""

from .cava import (
    CavaVisualizer,
    build_cava_config,
    detect_cava_methods,
    parse_cava_frame,
    render_spectrum,
)
from .cover import (
    CoverWidget,
    cell_aspect_ratio,
    cover_dims,
    fit_square_dims,
    init_terminal_image_support,
)
from .cover_grid import CoverGrid, GridCardItem
from .loading import SHIMMER_CLOCK, Skeleton, dot_frame, shimmer_line
from .lyrics_box import LyricsBox
from .media_list import MediaList, MediaListItem, media_item_from
from .player_bar import NowPlayingBar
from .progress_bar import SeekBar
from .scroll import scroll_child_region
from .tabs import SectionTabs
from .topbar import NAV_TABS, TopBar
from .track_table import ColumnDef, TrackTable

__all__ = [
    "NAV_TABS",
    "SHIMMER_CLOCK",
    "CavaVisualizer",
    "ColumnDef",
    "CoverGrid",
    "CoverWidget",
    "GridCardItem",
    "LyricsBox",
    "MediaList",
    "MediaListItem",
    "NowPlayingBar",
    "SectionTabs",
    "SeekBar",
    "Skeleton",
    "TopBar",
    "TrackTable",
    "build_cava_config",
    "cell_aspect_ratio",
    "cover_dims",
    "detect_cava_methods",
    "dot_frame",
    "fit_square_dims",
    "init_terminal_image_support",
    "media_item_from",
    "parse_cava_frame",
    "render_spectrum",
    "scroll_child_region",
    "shimmer_line",
]
