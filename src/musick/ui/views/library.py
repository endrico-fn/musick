"""Library view: explicit local bookmarks and session recent tracks (privacy-safe)."""

from __future__ import annotations

from textual.app import ComposeResult
from textual.containers import Vertical, VerticalScroll
from textual.events import Key

from musick.config import Settings
from musick.domain import Track
from musick.paths import bookmarks_file
from musick.services.search import HistoryStore
from musick.services.storage import BookmarkStore
from musick.ui.messages import (
    OpenAlbum,
    OpenArtist,
    OpenPlaylist,
    PlayNextTracks,
    PlayTracks,
    QueueTracks,
)
from musick.ui.theme import IconSet, icons_for
from musick.ui.widgets import ColumnDef, TrackTable

from .base import BaseView

__all__ = ["BookmarkStore", "LibraryView", "bookmarks_file"]


class LibraryView(BaseView):
    can_focus = False
    tab_id = "library"
    footer_hints = (
        ("j k", "select"),
        ("Enter", "open/play"),
        ("d", "remove bookmark"),
        ("Tab", "switch section"),
    )

    def __init__(
        self, bookmarks: BookmarkStore, history: HistoryStore, settings: Settings, **kw
    ) -> None:
        super().__init__(**kw)
        self._bookmarks = bookmarks
        self._history = history
        self._settings = settings
        self._icons: IconSet = icons_for(settings.display.icons)

    def compose(self) -> ComposeResult:
        with Vertical(id="lib-saved-panel", classes="panel-fill") as sp:
            sp.border_title = "Saved Bookmarks"
            with VerticalScroll(classes="main-list-scroll"):
                yield TrackTable(
                    [
                        ColumnDef("#", 3, "r"),
                        ColumnDef("Kind", 12, "l"),
                        ColumnDef("Title", None, "l"),
                        ColumnDef("Info", 26, "l"),
                    ],
                    icons=self._icons,
                    icon_col="Kind",
                    id="lib-saved-tbl",
                )

        with Vertical(id="lib-recent-panel", classes="panel-fill") as rp:
            rp.border_title = "Recent Tracks"
            with VerticalScroll(classes="main-list-scroll"):
                yield TrackTable(
                    [
                        ColumnDef("#", 3, "r"),
                        ColumnDef("Title", None, "l"),
                        ColumnDef("Artist", 26, "l"),
                        ColumnDef("Time", 8, "r"),
                    ],
                    icons=self._icons,
                    icon_col="Title",
                    id="lib-recent-tbl",
                )

    def on_mount(self) -> None:
        super().on_mount()
        self.refresh_library()

    def refresh_library(self) -> None:
        if not self.ui_ready:
            return
        self.query_one("#lib-recent-panel").border_subtitle = (
            "Saved to disk" if self._settings.privacy.save_history else "Session-only"
        )

        saved_rows = [
            ([str(i), x.get("kind", "Item"), x.get("title", ""), x.get("info", "")], x)
            for i, x in enumerate(self._bookmarks.items, start=1)
        ]
        self.query_one("#lib-saved-tbl", TrackTable).set_data(saved_rows)

        recent_rows = [
            ([str(i), t.title, t.artist_names, t.duration_text], t)
            for i, t in enumerate(self._history.recent_tracks, start=1)
        ]
        self.query_one("#lib-recent-tbl", TrackTable).set_data(recent_rows)

    def on_track_table_row_activated(self, event: TrackTable.RowActivated) -> None:
        p = event.payload
        if isinstance(p, Track):
            self.post_message(PlayTracks(self._history.recent_tracks, start=event.index))
        elif isinstance(p, dict):
            kind = p.get("kind", "")
            bid = p.get("browse_id", "")
            title = p.get("title", "")
            if kind == "Artist":
                self.post_message(OpenArtist(bid, title))
            elif kind == "Playlist":
                self.post_message(OpenPlaylist(bid, title))
            else:
                self.post_message(OpenAlbum(bid, title))

    def on_track_table_row_play(self, event: TrackTable.RowPlay) -> None:
        if isinstance(event.payload, Track):
            self.post_message(PlayTracks(self._history.recent_tracks, start=event.index))

    def on_track_table_row_queued(self, event: TrackTable.RowQueued) -> None:
        if isinstance(event.payload, Track):
            self.post_message(QueueTracks([event.payload]))

    def on_track_table_row_play_next(self, event: TrackTable.RowPlayNext) -> None:
        if isinstance(event.payload, Track):
            self.post_message(PlayNextTracks([event.payload]))

    def on_key(self, event: Key) -> None:
        tbl = self.query_one("#lib-saved-tbl", TrackTable)
        if tbl.has_focus and event.key in ("d", "delete") and tbl.rows:
            self._bookmarks.remove(tbl.cursor)
            self.refresh_library()
            event.stop()

    def on_settings_changed(self, icons: IconSet, cover_size: str, protocol: str) -> None:
        self._icons = icons
        self.query_one("#lib-saved-tbl", TrackTable).set_icons(icons)
        self.query_one("#lib-recent-tbl", TrackTable).set_icons(icons)
        self.refresh_library()
