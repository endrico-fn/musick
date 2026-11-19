"""Album detail view: Showcase Hero header + full-width tracklist with mini icons."""

from __future__ import annotations

import asyncio

from rich.text import Text
from textual.app import ComposeResult
from textual.containers import Center, Horizontal, Vertical, VerticalScroll
from textual.events import Key, Resize
from textual.widgets import Button, Static

from musick.config import Settings
from musick.domain import Album, Track, format_duration
from musick.player.controller import PlaybackState
from musick.services.catalog import CATALOG_ERRORS, Catalog
from musick.services.images import ImageService
from musick.ui.messages import (
    OpenArtist,
    OpenTrackArtist,
    PlayNextTracks,
    PlayTracks,
    QueueTracks,
    SaveBookmark,
    StartRadio,
)
from musick.ui.theme import ACCENT, DIM, TEXT, IconSet, icons_for
from musick.ui.widgets import ColumnDef, CoverWidget, Skeleton, TrackTable

from .base import BaseView, HeroIds, cancel_tasks, playing_index_for

_ALBUM_HERO = HeroIds(
    hero="#alb-hero",
    head_panel="#alb-head-panel",
    info_panel="#alb-info-panel",
    body_panel="#alb-tracks-panel",
    head_skel="#alb-head-skel",
    info_skel="#alb-info-skel",
    body_skel="#alb-tbl-skel",
    cover="#alb-cov",
    head_real="#alb-head-real",
    title="#alb-title",
    subtitle="#alb-sub",
)


class AlbumView(BaseView):
    can_focus = False
    tab_id = "home"
    footer_hints = (
        ("j k", "select"),
        ("Enter", "play track"),
        ("p", "play"),
        ("a", "queue"),
        ("A", "artist"),
        ("r", "radio"),
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
        self._album: Album | None = None
        self._playing_vid: str | None = None
        self._load_task: asyncio.Task[None] | None = None

    def compose(self) -> ComposeResult:
        with Vertical(classes="detail-shell"):
            with Horizontal(id="alb-hero", classes="hero-shell"):
                with Vertical(id="alb-head-panel", classes="panel hero-cover-panel") as hp:
                    hp.border_title = "Album"
                    with Center():
                        yield Skeleton("cover", id="alb-head-skel")
                        yield CoverWidget(
                            "L",
                            images=self._images,
                            protocol=self._settings.display.image_protocol,
                            id="alb-cov",
                        )

                with Vertical(id="alb-info-panel", classes="panel hero-info-panel") as ip:
                    ip.border_title = "Details"
                    yield Skeleton("table_rows", rows=4, id="alb-info-skel")
                    with Vertical(id="alb-head-real", classes="hero-info-body"):
                        with VerticalScroll(classes="hero-bio-scroll"):
                            yield Static("", id="alb-title")
                            yield Static("", id="alb-sub")
                            yield Static("", id="alb-desc")
                        with Horizontal(classes="hero-actions"):
                            yield Button(
                                f"{self._icons.playing} Play", id="alb-play", classes="-primary"
                            )
                            yield Button("Shf", id="alb-shf")
                            yield Button("+ Queue", id="alb-queue")
                            yield Button("Radio", id="alb-radio")
                            yield Button("Artist", id="alb-artist")
                            yield Button("Save", id="alb-save")

            with Vertical(id="alb-tracks-panel", classes="panel-fill main-content-panel") as tp:
                tp.border_title = "Tracks"
                yield Skeleton("table_rows", rows=9, id="alb-tbl-skel")
                with VerticalScroll(id="alb-tbl-scroll", classes="main-list-scroll"):
                    yield TrackTable(
                        [
                            ColumnDef("#", 3, "r"),
                            ColumnDef("Title", None, "l"),
                            ColumnDef("Artist", 24, "l"),
                            ColumnDef("Time", 8, "r"),
                        ],
                        sort_col="#",
                        icons=self._icons,
                        icon_col="Title",
                        id="alb-tbl",
                    )

    def on_mount(self) -> None:
        super().on_mount()
        self.query_one("#alb-tbl-scroll").display = False
        self._apply_responsive_sizes()
        self.begin_hero_loading(_ALBUM_HERO)
        self._load_task = asyncio.create_task(self._load())

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
        self.apply_hero_header("#alb-hero", "#alb-head-panel", "#alb-cov", "#alb-head-skel")

    async def _load(self) -> None:
        try:
            alb = await self._catalog.album(self.browse_id)
        except asyncio.CancelledError:
            raise
        except CATALOG_ERRORS as exc:
            self.show_hero_error(_ALBUM_HERO, self._title_hint or "Album", exc)
            return

        self._album = alb
        if not self.is_mounted:
            return
        with self.app.batch_update():
            self._populate_view()

    def _populate_view(self) -> None:
        alb = self._album
        if alb is None:
            return
        self.reveal_hero_header(_ALBUM_HERO)

        self.query_one("#alb-cov", CoverWidget).set_url(alb.summary.cover_url)
        self.query_one("#alb-title", Static).update(
            Text(alb.summary.title.upper(), style=f"bold {ACCENT}")
        )
        artists_str = ", ".join(a.name for a in alb.summary.artists) or "Unknown artist"
        sub_txt = Text()
        sub_txt.append(f"{alb.summary.kind} · ", style=DIM)
        sub_txt.append(artists_str, style=TEXT)
        self.query_one("#alb-sub", Static).update(sub_txt)

        total_dur = alb.duration_text or (
            format_duration(sum(t.duration or 0 for t in alb.tracks))
            if any(t.duration for t in alb.tracks)
            else ""
        )
        info_parts: list[str] = []
        if alb.summary.year:
            info_parts.append(alb.summary.year)
        info_parts.append(f"{len(alb.tracks)} songs")
        if total_dur:
            info_parts.append(total_dur)
        self.query_one("#alb-info-panel").border_subtitle = " · ".join(info_parts)

        self.set_optional_text("#alb-desc", alb.description or "")

        has_artist_id = bool(alb.summary.artists and alb.summary.artists[0].id)
        self.query_one("#alb-artist", Button).display = has_artist_id

        self.query_one("#alb-tracks-panel").border_title = f"Tracks ({len(alb.tracks)})"
        has_plays = any(t.plays for t in alb.tracks)
        cols = [
            ColumnDef("#", 3, "r"),
            ColumnDef("Title", None, "l"),
            ColumnDef("Artist", 24, "l"),
        ]
        if has_plays:
            cols.append(ColumnDef("Plays", 10, "r"))
        cols.append(ColumnDef("Time", 8, "r"))

        rows = []
        for i, t in enumerate(alb.tracks, start=1):
            cells = [str(i), t.title, t.artist_names]
            if has_plays:
                cells.append(t.plays or "")
            cells.append(t.duration_text)
            rows.append((cells, t))

        self.query_one("#alb-tbl-scroll").display = True
        tbl = self.query_one("#alb-tbl", TrackTable)
        tbl.set_columns(cols, sort_col="#")
        tbl.set_data(rows, playing_index=playing_index_for(alb.tracks, self._playing_vid))
        tbl.focus()

    def on_player_state(self, state: PlaybackState) -> None:
        vid = state.track.video_id if (state.track and not state.restored) else None
        if vid == self._playing_vid:
            return
        self._playing_vid = vid
        if self._album and self.is_mounted:
            self.query_one("#alb-tbl", TrackTable).set_playing_index(
                playing_index_for(self._album.tracks, vid)
            )

    def _open_album_artist(self, *, allow_track_fallback: bool = False) -> None:
        if self._album is None:
            return
        if self._album.summary.artists and self._album.summary.artists[0].id:
            a = self._album.summary.artists[0]
            if a.id:
                self.post_message(OpenArtist(a.id, a.name))
                return
        if allow_track_fallback and self._album.tracks:
            tbl = self.query_one("#alb-tbl", TrackTable)
            if 0 <= tbl.cursor < len(self._album.tracks):
                self.post_message(OpenTrackArtist(self._album.tracks[tbl.cursor]))

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if self._album is None:
            return
        bid = event.button.id
        if bid == "alb-play" and self._album.tracks:
            self.post_message(PlayTracks(self._album.tracks, start=0))
        elif bid == "alb-shf" and self._album.tracks:
            self.post_message(PlayTracks(self._album.tracks, start=0, shuffle=True))
        elif bid == "alb-queue" and self._album.tracks:
            self.post_message(QueueTracks(self._album.tracks))
        elif bid == "alb-radio" and self._album.tracks:
            self.post_message(StartRadio(video_id=self._album.tracks[0].video_id))
        elif bid == "alb-artist":
            self._open_album_artist()
        elif bid == "alb-save":
            self.post_message(SaveBookmark(self._album.summary))

    def on_key(self, event: Key) -> None:
        if self._album is None:
            return
        if event.key == "w":
            self.post_message(SaveBookmark(self._album.summary))
            event.stop()
        elif event.key == "r" and self._album.tracks:
            tbl = self.query_one("#alb-tbl", TrackTable)
            idx = tbl.cursor if 0 <= tbl.cursor < len(self._album.tracks) else 0
            self.post_message(StartRadio(video_id=self._album.tracks[idx].video_id))
            event.stop()
        elif event.key == "A":
            self._open_album_artist(allow_track_fallback=True)
            event.stop()

    def on_track_table_row_activated(self, event: TrackTable.RowActivated) -> None:
        if self._album:
            self.post_message(PlayTracks(self._album.tracks, start=event.index))

    def on_track_table_row_play(self, event: TrackTable.RowPlay) -> None:
        if self._album:
            self.post_message(PlayTracks(self._album.tracks, start=event.index))

    def on_track_table_row_queued(self, event: TrackTable.RowQueued) -> None:
        if isinstance(event.payload, Track):
            self.post_message(QueueTracks([event.payload]))

    def on_track_table_row_play_next(self, event: TrackTable.RowPlayNext) -> None:
        if isinstance(event.payload, Track):
            self.post_message(PlayNextTracks([event.payload]))

    def on_settings_changed(self, icons: IconSet, cover_size: str, protocol: str) -> None:
        del cover_size
        self._icons = icons
        self.query_one("#alb-cov", CoverWidget).set_protocol(protocol)
        self.query_one("#alb-tbl", TrackTable).set_icons(icons)
        play_btn = self.query_one("#alb-play", Button)
        play_btn.label = f"{icons.playing} Play"
        play_btn.refresh(layout=True)
