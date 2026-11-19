"""Spotify-style Queue management view with played/active/priority/context roles, reorder & undo."""

from __future__ import annotations

from textual.app import ComposeResult
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.events import Key
from textual.widgets import Button

from musick.config import Settings
from musick.player.controller import PlaybackState, PlayerEngine
from musick.ui.messages import NotifyStatus, OpenTrackAlbum, OpenTrackArtist
from musick.ui.theme import IconSet, icons_for
from musick.ui.widgets import ColumnDef, TrackTable

from .base import BaseView, handle_queue_shortcut

_ROLE_LABELS: dict[str, str] = {
    "playing": "Playing",
    "next": "Next Up",
    "played": "Played",
    "queued": "Up Next",
}


class QueueView(BaseView):
    can_focus = False
    tab_id = "queue"
    footer_hints = (
        ("j k", "select"),
        ("J K", "reorder"),
        ("T", "play next"),
        ("d", "remove"),
        ("z", "undo"),
        ("D", "dedup"),
        ("C", "clear next"),
        ("r/R", "repeat/radio"),
    )

    def __init__(self, engine: PlayerEngine, settings: Settings, **kw) -> None:
        super().__init__(**kw)
        self._engine = engine
        self._icons: IconSet = icons_for(settings.display.icons)
        self._seen_rev: tuple[int, int] = (-1, -1)

    def compose(self) -> ComposeResult:
        with Vertical(id="q-panel", classes="panel-fill main-content-panel") as qp:
            qp.border_title = "Queue"
            with Horizontal(classes="filter-row", id="q-actions"):
                yield Button("Shuffle", id="q-shf")
                yield Button("Repeat: Off", id="q-rpt")
                yield Button("Play Next", id="q-top")
                yield Button("Dedup", id="q-dedup")
                yield Button("Undo", id="q-undo")
                yield Button("Clear Next", id="q-clr-up")
                yield Button("Clear Queue", id="q-clr")
            with VerticalScroll(classes="main-list-scroll"):
                yield TrackTable(
                    [
                        ColumnDef("#", 4, "r"),
                        ColumnDef("Title", None, "l"),
                        ColumnDef("Artist", 22, "l"),
                        ColumnDef("Role", 8, "l"),
                        ColumnDef("Time", 8, "r"),
                    ],
                    icons=self._icons,
                    allow_a_queue=False,
                    icon_col="Title",
                    id="q-tbl",
                )

    def on_mount(self) -> None:
        super().on_mount()
        self.refresh_queue()

    def refresh_queue(self) -> None:
        if not self.ui_ready:
            return
        q = self._engine.queue
        self.query_one("#q-shf", Button).set_class(q.shuffle, "is-selected")
        rpt_btn = self.query_one("#q-rpt", Button)
        new_lbl = f"Repeat: {q.repeat.title()}"
        if str(rpt_btn.label) != new_lbl:
            rpt_btn.label = new_lbl
            rpt_btn.refresh(layout=True)
        rpt_btn.set_class(q.repeat != "off", "is-selected")
        self.query_one("#q-undo", Button).disabled = not q.can_undo()

        self.query_one("#q-panel").border_subtitle = q.summary_text()
        roles = [q.role_at(i) for i in range(len(q.tracks))]
        rows = [
            ([str(i), t.title, t.artist_names, _ROLE_LABELS[roles[i - 1]], t.duration_text], t)
            for i, t in enumerate(q.tracks, start=1)
        ]
        tbl = self.query_one("#q-tbl", TrackTable)
        tbl.set_data(rows, playing_index=q.index if q.index >= 0 else None, row_roles=roles)
        st = self._engine.state
        self._seen_rev = (st.queue_rev, st.track_rev)

    def on_player_state(self, state: PlaybackState) -> None:
        if (state.queue_rev, state.track_rev) != self._seen_rev:
            self.refresh_queue()

    def _move_cursor_to_top(self, tbl: TrackTable) -> None:
        if self._engine.queue.move_to_next(tbl.cursor):
            if self._engine.queue.index >= 0:
                tbl.set_cursor(self._engine.queue.index + 1)
            self._engine.notify_queue_changed()
            self.refresh_queue()
            self.post_message(NotifyStatus("Moved track to Next Up"))

    def on_button_pressed(self, event: Button.Pressed) -> None:
        bid = event.button.id
        tbl = self.query_one("#q-tbl", TrackTable)
        if bid == "q-shf":
            self._engine.toggle_shuffle()
            self.refresh_queue()
        elif bid == "q-rpt":
            self._engine.cycle_repeat()
            self.refresh_queue()
        elif bid == "q-top" and tbl.rows:
            self._move_cursor_to_top(tbl)
        elif bid == "q-dedup":
            removed = self._engine.queue.deduplicate()
            self._engine.notify_queue_changed()
            self.refresh_queue()
            self.post_message(
                NotifyStatus(f"Removed {removed} duplicate(s)" if removed else "No duplicates")
            )
        elif bid == "q-undo":
            if self._engine.queue.undo():
                self._engine.notify_queue_changed()
                self.refresh_queue()
                self.post_message(NotifyStatus("Restored queue state"))
        elif bid == "q-clr-up":
            n = self._engine.queue.clear_upcoming()
            self._engine.notify_queue_changed()
            self.refresh_queue()
            self.post_message(NotifyStatus(f"Cleared {n} upcoming track(s)"))
        elif bid == "q-clr":
            self._engine.queue.clear()
            self._engine.notify_queue_changed()
            self.refresh_queue()

    def on_track_table_row_activated(self, event: TrackTable.RowActivated) -> None:
        self.run_worker(self._engine.jump_to(event.index))

    def on_key(self, event: Key) -> None:
        tbl = self.query_one("#q-tbl", TrackTable)
        if event.key in ("d", "delete") and tbl.rows:
            self._engine.queue.remove(tbl.cursor)
            self._engine.notify_queue_changed()
            self.refresh_queue()
            event.stop()
        elif event.key == "a" and tbl.rows:
            t = tbl.rows[tbl.cursor][1]
            self.post_message(OpenTrackAlbum(t))
            event.stop()
        elif event.key == "A" and tbl.rows:
            t = tbl.rows[tbl.cursor][1]
            self.post_message(OpenTrackArtist(t))
            event.stop()
        else:
            cur_track = (
                tbl.rows[tbl.cursor][1]
                if (tbl.rows and 0 <= tbl.cursor < len(tbl.rows))
                else None
            )
            if handle_queue_shortcut(
                self,
                self._engine,
                event.key,
                tbl.cursor,
                set_cursor=lambda idx: tbl.set_cursor(idx),
                on_refresh=self.refresh_queue,
                current_track=cur_track,
            ):
                event.stop()

    def on_settings_changed(self, icons: IconSet, cover_size: str, protocol: str) -> None:
        self._icons = icons
        self.query_one("#q-tbl", TrackTable).set_icons(icons)
