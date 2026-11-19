"""Top-level views for Home, Library, Queue, Settings, Artist, Album, and Playlist."""

from .album import AlbumView
from .artist import ArtistView
from .base import BaseView, HeroIds, format_footer_hints
from .home import HomeView
from .library import BookmarkStore, LibraryView
from .playlist import PlaylistView
from .queue import QueueView
from .settings import SettingsView

__all__ = [
    "AlbumView",
    "ArtistView",
    "BaseView",
    "BookmarkStore",
    "HeroIds",
    "HomeView",
    "LibraryView",
    "PlaylistView",
    "QueueView",
    "SettingsView",
    "format_footer_hints",
]
