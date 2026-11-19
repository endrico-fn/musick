"""Playlist view: Curator Hero header + inline filter/sort/view toggle (Table & Mini-Covers)."""

from __future__ import annotations

import asyncio
from typing import Literal

from rich.text import Text
from textual.app import ComposeResult
from textual.containers import Center, Horizontal, Vertical, VerticalScroll
from textual.events import Key, Resize
from textual.widgets import Button, Input, Static

from musick.config import Settings
from musick.domain import Playlist, Track, format_duration
from musick.player.controller import PlaybackState
from musick.services.catalog import CATALOG_ERRORS, Catalog
from musick.services.images import ImageService
from musick.ui.messages import (
    OpenTrackAlbum,
    OpenTrackArtist,
    PlayNextTracks,
    PlayTracks,
    QueueTracks,
    SaveBookmark,
    StartRadio,
)
from musick.ui.theme import ACCENT, DIM, TEXT, IconSet, icons_for
from musick.ui.widgets import (
    ColumnDef,
    CoverWidget,
    MediaList,
    Skeleton,
    TrackTable,
    media_item_from,
)

from .base import BaseView, HeroIds, cancel_tasks, playing_index_for

_PLAYLIST_HERO = HeroIds(
    hero="#pl-hero",
    head_panel="#pl-head-panel",
    info_panel="#pl-info-panel",
    body_panel="#pl-songs-panel",
    head_skel="#pl-head-skel",
    info_skel="#pl-info-skel",
    body_skel="#pl-tbl-skel",
    cover="#pl-cov",
    head_real="#pl-head-real",
    title="#pl-title",
    subtitle="#pl-author",
)

_SORT_MODES = ("Original", "Title", "Time")
PlaylistViewMode = Literal["table", "covers"]


class PlaylistView(BaseView):
    can_focus = False
    tab_id = "home"
    footer_hints = (
        ("/", "filter"),
        ("j k", "select"),
        ("Enter", "play"),
        ("v", "table/covers"),
        ("o", "sort"),
        ("a", "queue"),
        ("A", "artist"),
        ("r", "radio"),
        ("PgDn", "load more"),
        ("w", "save"),
        ("Bksp", "back"),
    )

    def __init__(
        self,
        browse_id: str,
        title_hint: str,
        catalog: Catalog,
        images: ImageService,
        settings: Settings,
        **kw,
    ) -> None:
        super().__init__(**kw)
        self.browse_id = browse_id
        self._title_hint = title_hint
        self._catalog = catalog
        self._images = images
        self._settings = settings
        self._icons: IconSet = icons_for(settings.display.icons)

        self._playlist: Playlist | None = None
        self._limit: int = 50
        self._sort_idx: int = 0
        self._view_mode: PlaylistViewMode = "table"
        self._filter_text: str = ""
        self._visible_tracks: list[Track] = []
        self._playing_vid: str | None = None
        self._loading_more: bool = False
        self._load_task: asyncio.Task[None] | None = None

    def compose(self) -> ComposeResult:
        with Vertical(classes="detail-shell"):
            with Horizontal(id="pl-hero", classes="hero-shell"):
                with Vertical(id="pl-head-panel", classes="panel hero-cover-panel") as hp:
                    hp.border_title = "Playlist"
                    with Center():
                        yield Skeleton("cover", id="pl-head-skel")
                        yield CoverWidget(
                            "L",
                            images=self._images,
                            protocol=self._settings.display.image_protocol,
                            id="pl-cov",
                        )

                with Vertical(id="pl-info-panel", classes="panel hero-info-panel") as ip:
                    ip.border_title = "Curator"
                    yield Skeleton("table_rows", rows=4, id="pl-info-skel")
                    with Vertical(id="pl-head-real", classes="hero-info-body"):
                        with VerticalScroll(classes="hero-bio-scroll"):
                            yield Static("", id="pl-title")
                            yield Static("", id="pl-author")
                            yield Static("", id="pl-desc")
                        with Horizontal(classes="hero-actions"):
                            yield Button(
                                f"{self._icons.playing} Play", id="pl-play", classes="-primary"
                            )
                            yield Button("Shf", id="pl-shf")
                            yield Button("+ Queue", id="pl-queue")
                            yield Button("Radio", id="pl-radio")
                            yield Button("Save", id="pl-save")
                            yield Button("More", id="pl-more-btn")

            with Vertical(id="pl-songs-panel", classes="panel-fill main-content-panel") as sp:
                sp.border_title = "Songs"
                with Horizontal(classes="filter-row", id="pl-filter-row"):
                    yield Static(
                        Text(f"{self._icons.prompt} ", style=ACCENT),
                        id="pl-filter-prompt",
                        classes="inline-label",
                    )
                    yield Input(placeholder="Filter songs…", id="pl-filter-input")
                    yield Button(f"Sort {_SORT_MODES[0]} {self._icons.sort_desc}", id="pl-sort-btn")
                    yield Button("View: Table", id="pl-view-btn")
                yield Skeleton("table_rows", rows=9, id="pl-tbl-skel")
                with VerticalScroll(id="pl-tbl-scroll", classes="main-list-scroll"):
                    yield TrackTable(
                        [
                            ColumnDef("#", 4, "r"),
                            ColumnDef("Title", None, "l"),
                            ColumnDef("Artist", 24, "l"),
                            ColumnDef("Time", 8, "r"),
                        ],
                        sort_col="#",
                        icons=self._icons,
                        icon_col="Title",
                        id="pl-tbl",
                    )
                    yield MediaList(
                        icons=self._icons,
                        images=self._images,
                        protocol=self._settings.display.image_protocol,
                        empty_text="No matching tracks in playlist",
                        allow_a_queue=True,
                        id="pl-media",
                    )

    def on_mount(self) -> None:
        super().on_mount()
        self.query_one("#pl-tbl-scroll").display = False
        self.query_one("#pl-media").display = False
        self.query_one("#pl-more-btn").display = False
        self._apply_responsive_sizes()
        self.begin_hero_loading(_PLAYLIST_HERO)
        self._load_task = asyncio.create_task(self._load(self._limit))

    def on_view_activated(self) -> None:
        if self.is_mounted:
            self._apply_responsive_sizes()

    def on_unmount(self) -> None:
        super().on_unmount()
        cancel_tasks(self._load_task)

    def on_resize(self, event: Resize) -> None:
        del event
        self._apply_responsive_sizes()

    def _apply_responsive_sizes(self) -> None:
        if not self.ui_ready:
            return
        self.apply_hero_header("#pl-hero", "#pl-head-panel", "#pl-cov", "#pl-head-skel")

    def _active_list_widget(self) -> TrackTable | MediaList:
        if self._view_mode == "covers":
            return self.query_one("#pl-media", MediaList)
        return self.query_one("#pl-tbl", TrackTable)

    def handle_slash(self) -> bool:
        self.query_one("#pl-filter-input", Input).focus()
        return True

    def handle_input_escape(self) -> bool:
        inp = self.query_one("#pl-filter-input", Input)
        if not inp.has_focus:
            return False
        self._active_list_widget().focus()
        return True

    def handle_escape(self) -> bool:
        if not self._filter_text:
            return False
        self._filter_text = ""
        inp = self.query_one("#pl-filter-input", Input)
        with inp.prevent(Input.Changed):
            inp.value = ""
        self._refresh_table(focus=True)
        return True

    async def _load(self, limit: int) -> None:
        try:
            pl = await self._catalog.playlist(self.browse_id, limit=limit)
        except asyncio.CancelledError:
            raise
        except CATALOG_ERRORS as exc:
            self._loading_more = False
            if self.is_mounted:
                self.show_hero_error(
                    _PLAYLIST_HERO,
                    self._title_hint or "Playlist",
                    exc,
                    populate_text=self._playlist is None,
                )
                if self._playlist is not None:
                    self.query_one("#pl-tbl", TrackTable).set_skeleton_tail(0)
            return

        self._playlist = pl
        self._loading_more = False
        if self.is_mounted:
            with self.app.batch_update():
                self._populate_view()

    def _populate_view(self) -> None:
        pl = self._playlist
        if pl is None:
            return
        self.reveal_hero_header(_PLAYLIST_HERO, body_loading=self._loading_more)

        self.query_one("#pl-cov", CoverWidget).set_url(pl.summary.cover_url)
        self.query_one("#pl-title", Static).update(
            Text(pl.summary.title.upper(), style=f"bold {ACCENT}")
        )
        author_txt = Text()
        if pl.summary.author:
            author_txt.append("Curated by ", style=DIM)
            author_txt.append(pl.summary.author, style=TEXT)
        else:
            author_txt.append("Playlist", style=DIM)
        self.query_one("#pl-author", Static).update(author_txt)

        stats_parts = [f"{pl.total or len(pl.tracks)} songs"]
        if pl.duration_text:
            stats_parts.append(pl.duration_text)
        if pl.views:
            stats_parts.append(f"{pl.views} views")
        if pl.year:
            stats_parts.append(pl.year)
        self.query_one("#pl-info-panel").border_subtitle = " · ".join(stats_parts)

        self.set_optional_text("#pl-desc", pl.description or "")

        has_more = bool(pl.total and len(pl.tracks) < pl.total)
        self.query_one("#pl-more-btn").display = has_more
        self.query_one("#pl-songs-panel").border_title = f"Songs ({len(pl.tracks)})"

        self._refresh_table(focus=True)

    def _sync_view_mode_ui(self) -> None:
        view_btn = self.query_one("#pl-view-btn", Button)
        is_covers = self._view_mode == "covers"
        view_btn.label = "View: Covers" if is_covers else "View: Table"
        view_btn.set_class(is_covers, "is-selected")
        view_btn.refresh(layout=True)

        self.query_one("#pl-tbl", TrackTable).display = not is_covers
        self.query_one("#pl-media", MediaList).display = is_covers

    def toggle_view_mode(self, *, focus: bool = True) -> None:
        prev_cur = max(0, self._active_list_widget().cursor)
        self._view_mode = "covers" if self._view_mode == "table" else "table"
        self._refresh_table(focus=focus)
        target = self._active_list_widget()
        if self._visible_tracks:
            target.set_cursor(prev_cur, scroll=True)

    def _refresh_table(self, focus: bool = False) -> None:
        if self._playlist is None:
            return
        tracks = list(self._playlist.tracks)
        q = self._filter_text.strip().lower()
        if q:
            tracks = [
                t
                for t in tracks
                if q in t.title.lower()
                or q in t.artist_names.lower()
                or (t.album and q in t.album.lower())
            ]

        mode = _SORT_MODES[self._sort_idx]
        if mode == "Title":
            tracks.sort(key=lambda t: t.title.lower())
        elif mode == "Time":
            tracks.sort(key=lambda t: t.duration or 0)

        self._visible_tracks = tracks
        vis_dur = (
            format_duration(sum(t.duration or 0 for t in tracks))
            if any(t.duration for t in tracks)
            else ""
        )
        total_cnt = len(self._playlist.tracks)
        sub_parts: list[str] = []
        if q:
            sub_parts.append(f"Filtered {len(tracks)}/{total_cnt}")
        else:
            sub_parts.append(f"{len(tracks)} loaded")
        if vis_dur:
            sub_parts.append(vis_dur)
        self.query_one("#pl-songs-panel").border_subtitle = " · ".join(sub_parts)
        p_idx = playing_index_for(tracks, self._playing_vid)
        sort_col = "#" if mode == "Original" else mode
        rows = [
            ([str(i), t.title, t.artist_names, t.duration_text], t)
            for i, t in enumerate(tracks, start=1)
        ]

        self.query_one("#pl-tbl-scroll").display = True
        tbl = self.query_one("#pl-tbl", TrackTable)
        tbl.set_data(
            rows,
            playing_index=p_idx,
            sort_col=sort_col,
            skeleton_tail=2 if self._loading_more else 0,
        )
        media = self.query_one("#pl-media", MediaList)
        media.set_items([media_item_from(t) for t in tracks], playing_index=p_idx)
        self._sync_view_mode_ui()

        if focus and tracks:
            self._active_list_widget().focus()

    def load_more(self) -> None:
        if self._playlist is None or self._loading_more:
            return
        if self._playlist.total and len(self._playlist.tracks) >= self._playlist.total:
            return
        self._loading_more = True
        self._limit += 100
        self.bind_panel_loader(self.query_one("#pl-songs-panel"), True)
        self.query_one("#pl-tbl", TrackTable).set_skeleton_tail(2)
        cancel_tasks(self._load_task)
        self._load_task = asyncio.create_task(self._load(self._limit))

    def on_player_state(self, state: PlaybackState) -> None:
        vid = state.track.video_id if (state.track and not state.restored) else None
        if vid == self._playing_vid:
            return
        self._playing_vid = vid
        if self._playlist and self.is_mounted:
            p_idx = playing_index_for(self._visible_tracks, vid)
            self.query_one("#pl-tbl", TrackTable).set_playing_index(p_idx)
            self.query_one("#pl-media", MediaList).set_playing_index(p_idx)

    def on_input_changed(self, event: Input.Changed) -> None:
        if event.input.id == "pl-filter-input":
            self._filter_text = event.value
            self._refresh_table(focus=False)

    def on_input_submitted(self, event: Input.Submitted) -> None:
        if event.input.id == "pl-filter-input":
            self._active_list_widget().focus()

    def _cycle_sort(self) -> None:
        self._sort_idx = (self._sort_idx + 1) % len(_SORT_MODES)
        sort_btn = self.query_one("#pl-sort-btn", Button)
        sort_btn.label = f"Sort {_SORT_MODES[self._sort_idx]} {self._icons.sort_desc}"
        sort_btn.refresh(layout=True)
        self._refresh_table(focus=False)

    def on_button_pressed(self, event: Button.Pressed) -> None:
        bid = event.button.id
        if bid == "pl-sort-btn":
            self._cycle_sort()
        elif bid == "pl-view-btn":
            self.toggle_view_mode(focus=True)
        elif bid == "pl-more-btn":
            self.load_more()
        elif self._playlist is None:
            return
        elif bid == "pl-play" and self._visible_tracks:
            self.post_message(PlayTracks(self._visible_tracks, start=0))
        elif bid == "pl-shf" and self._visible_tracks:
            self.post_message(PlayTracks(self._visible_tracks, start=0, shuffle=True))
        elif bid == "pl-queue" and self._visible_tracks:
            self.post_message(QueueTracks(self._visible_tracks))
        elif bid == "pl-radio":
            vid = self._visible_tracks[0].video_id if self._visible_tracks else None
            self.post_message(
                StartRadio(video_id=vid, playlist_id=self._playlist.summary.browse_id)
            )
        elif bid == "pl-save":
            self.post_message(SaveBookmark(self._playlist.summary))

    def on_track_table_row_activated(self, event: TrackTable.RowActivated) -> None:
        if self._visible_tracks:
            self.post_message(PlayTracks(self._visible_tracks, start=event.index))

    def on_track_table_row_play(self, event: TrackTable.RowPlay) -> None:
        if self._visible_tracks:
            self.post_message(PlayTracks(self._visible_tracks, start=event.index))

    def on_track_table_row_queued(self, event: TrackTable.RowQueued) -> None:
        if isinstance(event.payload, Track):
            self.post_message(QueueTracks([event.payload]))

    def on_track_table_row_play_next(self, event: TrackTable.RowPlayNext) -> None:
        if isinstance(event.payload, Track):
            self.post_message(PlayNextTracks([event.payload]))

    def on_media_list_item_activated(self, event: MediaList.ItemActivated) -> None:
        if event.widget.id == "pl-media" and self._visible_tracks:
            self.post_message(PlayTracks(self._visible_tracks, start=event.index))

    def on_media_list_item_play(self, event: MediaList.ItemPlay) -> None:
        if event.widget.id == "pl-media" and self._visible_tracks:
            self.post_message(PlayTracks(self._visible_tracks, start=event.index))

    def on_media_list_item_queued(self, event: MediaList.ItemQueued) -> None:
        if event.widget.id == "pl-media" and isinstance(event.payload, Track):
            self.post_message(QueueTracks([event.payload]))

    def on_media_list_item_play_next(self, event: MediaList.ItemPlayNext) -> None:
        if event.widget.id == "pl-media" and isinstance(event.payload, Track):
            self.post_message(PlayNextTracks([event.payload]))

    def on_media_list_item_open_artist(self, event: MediaList.ItemOpenArtist) -> None:
        if event.widget.id == "pl-media" and isinstance(event.payload, Track):
            self.post_message(OpenTrackArtist(event.payload))

    def on_media_list_item_open_album(self, event: MediaList.ItemOpenAlbum) -> None:
        if event.widget.id == "pl-media" and isinstance(event.payload, Track):
            self.post_message(OpenTrackAlbum(event.payload))

    def on_key(self, event: Key) -> None:
        if self.query_one("#pl-filter-input", Input).has_focus:
            return
        if event.key == "o":
            self._cycle_sort()
            event.stop()
        elif event.key == "v":
            self.toggle_view_mode(focus=True)
            event.stop()
        elif event.key == "w" and self._playlist:
            self.post_message(SaveBookmark(self._playlist.summary))
            event.stop()
        elif event.key == "A" and self._visible_tracks:
            cur = self._active_list_widget().cursor
            if 0 <= cur < len(self._visible_tracks):
                self.post_message(OpenTrackArtist(self._visible_tracks[cur]))
                event.stop()
        elif event.key == "r" and self._visible_tracks:
            cur = self._active_list_widget().cursor
            idx = cur if 0 <= cur < len(self._visible_tracks) else 0
            self.post_message(StartRadio(video_id=self._visible_tracks[idx].video_id))
            event.stop()
        elif event.key == "pagedown":
            cur = self._active_list_widget().cursor
            if cur >= max(0, len(self._visible_tracks) - 5):
                self.load_more()

    def on_settings_changed(self, icons: IconSet, cover_size: str, protocol: str) -> None:
        del cover_size
        self._icons = icons
        self.query_one("#pl-filter-prompt", Static).update(Text(f"{icons.prompt} ", style=ACCENT))
        self.query_one("#pl-cov", CoverWidget).set_protocol(protocol)
        self.query_one("#pl-tbl", TrackTable).set_icons(icons)
        media = self.query_one("#pl-media", MediaList)
        media.set_icons(icons)
        media.set_protocol(protocol)
        play_btn = self.query_one("#pl-play", Button)
        play_btn.label = f"{icons.playing} Play"
        play_btn.refresh(layout=True)
