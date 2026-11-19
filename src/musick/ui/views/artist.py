"""Artist profile view: Hero Header + Bold Split Overview + Mini-Cover Songs & Fit Discography."""

from __future__ import annotations

import asyncio

from rich.text import Text
from textual.app import ComposeResult
from textual.containers import Center, Horizontal, Vertical, VerticalScroll
from textual.events import Key, Resize
from textual.widgets import Button, Static

from musick.config import Settings
from musick.domain import AlbumSummary, Artist, ArtistSummary, PlaylistSummary, Track
from musick.player.controller import PlaybackState
from musick.services.catalog import CATALOG_ERRORS, Catalog
from musick.services.images import ImageService
from musick.ui.messages import (
    OpenAlbum,
    OpenArtist,
    OpenPlaylist,
    OpenTrackAlbum,
    OpenTrackArtist,
    PlayEntityDirect,
    PlayNextEntityDirect,
    PlayNextTracks,
    PlayTracks,
    QueueEntityDirect,
    QueueTracks,
    RefreshFooter,
    SaveBookmark,
    StartRadio,
)
from musick.ui.theme import ACCENT, DIM, TEXT, IconSet, icons_for
from musick.ui.widgets import (
    ColumnDef,
    CoverGrid,
    CoverWidget,
    GridCardItem,
    MediaList,
    SectionTabs,
    Skeleton,
    TrackTable,
    media_item_from,
)

from .base import BaseView, HeroIds, cancel_tasks, playing_index_for

_ARTIST_HERO = HeroIds(
    hero="#art-hero",
    head_panel="#art-head-panel",
    info_panel="#art-info-panel",
    body_panel="#art-body-panel",
    head_skel="#art-head-skel",
    info_skel="#art-info-skel",
    body_skel="#art-body-skel",
    cover="#art-cov",
    head_real="#art-head-real",
    title="#art-name",
    subtitle="#art-sub",
)

_ARTIST_TABS: tuple[tuple[str, str], ...] = (
    ("overview", "Overview"),
    ("songs", "Songs"),
    ("albums", "Albums"),
    ("singles", "Singles"),
    ("videos", "Videos"),
    ("playlists", "Playlists"),
    ("similar", "Similar"),
)

_ARTIST_SHORT_LABELS: dict[str, str] = {
    "overview": "Overv.",
    "songs": "Songs",
    "albums": "Albums",
    "singles": "Singles",
    "videos": "Videos",
    "playlists": "Playl.",
    "similar": "Similar",
}

_HINTS_OVERVIEW: tuple[tuple[str, str], ...] = (
    ("Tab", "next tab"),
    ("j k", "select"),
    ("Enter", "open/play"),
    ("t", "top 10/all"),
    ("a", "queue"),
    ("r", "radio"),
    ("w", "save"),
    ("Bksp", "back"),
)

_HINTS_SONGS: tuple[tuple[str, str], ...] = (
    ("Tab", "next tab"),
    ("j k", "select"),
    ("Enter", "play"),
    ("a", "queue"),
    ("o", "album"),
    ("A", "artist"),
    ("r", "radio"),
    ("Bksp", "back"),
)

_HINTS_RELEASES: tuple[tuple[str, str], ...] = (
    ("Tab", "next tab"),
    ("hjkl", "move"),
    ("Enter", "open"),
    ("p", "play"),
    ("a", "queue"),
    ("c/S/M/L", "size"),
    ("v", "grid/list"),
    ("Bksp", "back"),
)

_HINTS_OTHER: tuple[tuple[str, str], ...] = (
    ("Tab", "next tab"),
    ("j k", "select"),
    ("Enter", "open/play"),
    ("a", "queue"),
    ("r", "radio"),
    ("w", "save"),
    ("Bksp", "back"),
)

_SPLIT_BREAKPOINT = 106


class ArtistView(BaseView):
    can_focus = False
    tab_id = "home"
    footer_hints = _HINTS_OVERVIEW

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

        self._artist: Artist | None = None
        self._active_tab: str = "overview"
        self._cover_size: str = settings.display.cover_size
        self._sort_newest: bool = True
        self._grid_mode: bool = True
        self._top10_only: bool = True
        self._visible_tracks: tuple[Track, ...] = ()
        self._all_albums: tuple[AlbumSummary, ...] | None = None
        self._all_singles: tuple[AlbumSummary, ...] | None = None
        self._latest_release: AlbumSummary | None = None
        self._playing_vid: str | None = None
        self._load_task: asyncio.Task[None] | None = None
        self._rel_task: asyncio.Task[None] | None = None

    def compose(self) -> ComposeResult:
        with Vertical(classes="detail-shell"):
            with Horizontal(id="art-hero", classes="hero-shell"):
                with Vertical(id="art-head-panel", classes="panel hero-cover-panel") as hp:
                    hp.border_title = "Artist"
                    with Center():
                        yield Skeleton("cover", id="art-head-skel")
                        yield CoverWidget(
                            "L",
                            images=self._images,
                            protocol=self._settings.display.image_protocol,
                            id="art-cov",
                        )

                with Vertical(id="art-info-panel", classes="panel hero-info-panel") as ip:
                    ip.border_title = "Profile"
                    yield Skeleton("table_rows", rows=4, id="art-info-skel")
                    with Vertical(id="art-head-real", classes="hero-info-body"):
                        with VerticalScroll(classes="hero-bio-scroll"):
                            yield Static("", id="art-name")
                            yield Static("", id="art-sub")
                            yield Static("", id="art-bio")
                        with Horizontal(classes="hero-actions"):
                            yield Button(
                                f"{self._icons.playing} Play", id="art-play", classes="-primary"
                            )
                            yield Button("Shf", id="art-shf")
                            yield Button("+ Queue", id="art-queue")
                            yield Button("Radio", id="art-radio")
                            yield Button("Save", id="art-save")

            with Vertical(id="art-body-panel", classes="panel-fill main-content-panel") as bp:
                bp.border_title = "Catalog"
                yield SectionTabs(
                    _ARTIST_TABS,
                    active="overview",
                    short_labels=_ARTIST_SHORT_LABELS,
                    id="art-tabs",
                )
                yield Skeleton("table_rows", rows=8, id="art-body-skel")
                with VerticalScroll(id="art-body-scroll", classes="main-list-scroll"):
                    with Horizontal(id="art-ov-grid", classes="overview-grid"):
                        with Vertical(id="art-ov-left"):
                            with Horizontal(id="art-songs-ctrl", classes="filter-row"):
                                yield Static("", id="art-songs-crumb")
                                yield Button("Show All", id="art-songs-toggle")
                            yield TrackTable(
                                [
                                    ColumnDef("#", 3, "r"),
                                    ColumnDef("Title", None, "l"),
                                    ColumnDef("Album", 24, "l"),
                                    ColumnDef("Plays", 10, "r"),
                                    ColumnDef("Time", 8, "r"),
                                ],
                                sort_col="#",
                                icons=self._icons,
                                icon_col="Title",
                                bubble_boundary=True,
                                id="art-songs-tbl",
                            )
                        with Vertical(id="art-ov-right"):
                            with Vertical(id="art-ov-latest-wrap"):
                                yield Static(
                                    "LATEST RELEASE", classes="overview-section-label-first"
                                )
                                with Horizontal(id="art-ov-latest-card"):
                                    yield CoverWidget(
                                        "S",
                                        images=self._images,
                                        protocol=self._settings.display.image_protocol,
                                        id="art-ov-latest-cov",
                                    )
                                    with Vertical(id="art-ov-latest-meta"):
                                        yield Static("", id="art-ov-latest-title")
                                        yield Static("", id="art-ov-latest-sub")
                                        with Horizontal(id="art-ov-latest-actions"):
                                            yield Button(
                                                f"{self._icons.playing} Play",
                                                id="art-latest-play",
                                                classes="-primary",
                                            )
                                            yield Button("Open", id="art-latest-open")
                            with Vertical(id="art-ov-rel-wrap"):
                                yield Static("DISCOGRAPHY", classes="overview-section-label")
                                yield CoverGrid(
                                    self._images,
                                    cover_size="S",
                                    protocol=self._settings.display.image_protocol,
                                    icons=self._icons,
                                    single_row=True,
                                    fit_width=True,
                                    id="art-ov-rel-grid",
                                )
                            with Vertical(id="art-ov-sim-wrap"):
                                yield Static("FANS ALSO LIKE", classes="overview-section-label")
                                yield TrackTable(
                                    [
                                        ColumnDef("#", 3, "r"),
                                        ColumnDef("Title", None, "l"),
                                        ColumnDef("Info", 22, "l"),
                                    ],
                                    icons=self._icons,
                                    icon_col="Title",
                                    show_header=False,
                                    bubble_boundary=True,
                                    id="art-ov-sim-tbl",
                                )

                    with Vertical(id="art-songs-media-wrap"):
                        with Horizontal(id="art-songs-media-ctrl", classes="filter-row"):
                            yield Static("", id="art-songs-media-crumb")
                        yield MediaList(
                            icons=self._icons,
                            images=self._images,
                            protocol=self._settings.display.image_protocol,
                            allow_a_queue=True,
                            empty_text="No songs available",
                            id="art-songs-media",
                        )

                    with Vertical(id="art-rel-wrap"):
                        with Horizontal(id="art-rel-ctrl", classes="filter-row"):
                            yield Static("", id="art-rel-crumb")
                            yield Button("Sort: Newest", id="art-rel-sort")
                            yield Button("List", id="art-rel-view")
                            yield Button("S", id="art-sz-S")
                            yield Button("M", id="art-sz-M")
                            yield Button("L", id="art-sz-L")
                        yield CoverGrid(
                            self._images,
                            cover_size=self._cover_size,
                            protocol=self._settings.display.image_protocol,
                            icons=self._icons,
                            fit_width=True,
                            scale=1.25,
                            id="art-rel-grid",
                        )
                        yield TrackTable(
                            [
                                ColumnDef("#", 3, "r"),
                                ColumnDef("Title", None, "l"),
                                ColumnDef("Kind", 10, "l"),
                                ColumnDef("Year", 6, "r"),
                            ],
                            icons=self._icons,
                            icon_col="Title",
                            id="art-rel-tbl",
                        )

                    yield TrackTable(
                        [
                            ColumnDef("#", 3, "r"),
                            ColumnDef("Title", None, "l"),
                            ColumnDef("Info", 28, "l"),
                        ],
                        icons=self._icons,
                        icon_col="Title",
                        id="art-misc-tbl",
                    )

    def on_mount(self) -> None:
        super().on_mount()
        self.query_one("#art-body-scroll").display = False
        self.query_one("#art-ov-grid").display = False
        self.query_one("#art-songs-ctrl").display = False
        self.query_one("#art-songs-media-wrap").display = False
        self.query_one("#art-rel-wrap").display = False
        self.query_one("#art-misc-tbl").display = False
        self._apply_responsive_sizes()
        self._sync_size_buttons()
        self.begin_hero_loading(_ARTIST_HERO)
        self._load_task = asyncio.create_task(self._load_artist())

    def on_view_activated(self) -> None:
        if self.is_mounted:
            self._apply_responsive_sizes()

    def on_unmount(self) -> None:
        super().on_unmount()
        cancel_tasks(self._load_task, self._rel_task)

    def on_resize(self, event: Resize) -> None:
        del event
        self._apply_responsive_sizes()

    def _is_overview_split(self) -> bool:
        w = self.size.width or 96
        has_right = self.query_one("#art-ov-right").display
        return self._active_tab == "overview" and w >= _SPLIT_BREAKPOINT and has_right

    def _sync_overview_layout(self) -> None:
        if not self.ui_ready:
            return
        ov_grid = self.query_one("#art-ov-grid")
        split = self._is_overview_split()
        was_split = ov_grid.has_class("is-split")
        ov_grid.set_class(split, "is-split")
        if (
            split != was_split
            and self._artist is not None
            and self._active_tab == "overview"
        ):
            self._populate_songs_table(self._visible_tracks, focus=False)

    def _apply_responsive_sizes(self) -> None:
        if not self.ui_ready:
            return
        h = self.size.height or 32
        w = self.size.width or 96
        min_body_h = 10 if h >= 26 else 8
        self.apply_hero_header(
            "#art-hero",
            "#art-head-panel",
            "#art-cov",
            "#art-head-skel",
            min_body_h=min_body_h,
            max_cov_h=8,
        )
        cov_w, cov_h = (11, 5) if w >= _SPLIT_BREAKPOINT else (9, 4)
        self.query_one("#art-ov-latest-cov", CoverWidget).set_exact_size(cov_w, cov_h)
        self._sync_overview_layout()

    async def _load_artist(self) -> None:
        try:
            art = await self._catalog.artist(self.browse_id)
        except asyncio.CancelledError:
            raise
        except CATALOG_ERRORS as exc:
            self.show_hero_error(_ARTIST_HERO, self._title_hint or "Artist", exc)
            return

        self._artist = art
        if not self.is_mounted:
            return
        with self.app.batch_update():
            self._render_header()
            self.switch_tab(self._active_tab)

    def _refresh_catalog_counts(self) -> None:
        art = self._artist
        if art is None or not self.is_mounted:
            return
        albums_cnt = len(self._all_albums) if self._all_albums is not None else len(art.albums)
        singles_cnt = len(self._all_singles) if self._all_singles is not None else len(art.singles)
        counts = [
            lbl
            for lbl in (
                f"{len(art.songs)} songs" if art.songs else "",
                f"{albums_cnt} albums" if albums_cnt else "",
                f"{singles_cnt} singles" if singles_cnt else "",
                f"{len(art.videos)} videos" if art.videos else "",
                f"{len(art.playlists)} playlists" if art.playlists else "",
            )
            if lbl
        ]
        sub_text = " · ".join(counts) if counts else "Artist"
        sub_w = self.query_one("#art-sub", Static)
        sub_w.update(Text(sub_text, style=DIM))
        sub_w.display = True

        tabs = [
            ("overview", "Overview"),
            ("songs", f"Songs ({len(art.songs)})" if art.songs else "Songs"),
            ("albums", f"Albums ({albums_cnt})" if albums_cnt else "Albums"),
            ("singles", f"Singles ({singles_cnt})" if singles_cnt else "Singles"),
            ("videos", f"Videos ({len(art.videos)})" if art.videos else "Videos"),
            ("playlists", f"Playlists ({len(art.playlists)})" if art.playlists else "Playlists"),
            ("similar", f"Similar ({len(art.related)})" if art.related else "Similar"),
        ]
        self.query_one("#art-tabs", SectionTabs).set_tabs(tabs, short_labels=_ARTIST_SHORT_LABELS)

    def _render_header(self) -> None:
        art = self._artist
        if art is None:
            return
        self.reveal_hero_header(_ARTIST_HERO)

        self.query_one("#art-cov", CoverWidget).set_url(art.summary.cover_url)
        self.query_one("#art-name", Static).update(
            Text(art.summary.name.upper(), style=f"bold {ACCENT}")
        )
        info_panel = self.query_one("#art-info-panel")
        aud_parts: list[str] = []
        if art.listeners:
            low = art.listeners.lower()
            if "monthly" in low or "listener" in low or "view" in low:
                aud_parts.append(art.listeners)
            elif art.subscribers and art.listeners == art.subscribers:
                aud_parts.append(f"{art.listeners} subscribers")
            else:
                aud_parts.append(f"{art.listeners} listeners")
        if art.subscribers and art.subscribers != art.listeners:
            aud_parts.append(f"{art.subscribers} subs")
        if art.views and art.views != art.listeners and art.views != art.subscribers:
            aud_parts.append(f"{art.views} views")
        info_panel.border_subtitle = " · ".join(aud_parts)

        self._refresh_catalog_counts()
        self.set_optional_text("#art-bio", art.description or "")

    def _populate_songs_table(
        self, songs: tuple[Track, ...], *, focus: bool = True
    ) -> None:
        songs_tbl = self.query_one("#art-songs-tbl", TrackTable)
        has_plays = any(t.plays for t in songs)
        has_dur = any(t.duration for t in songs)
        album_w = 16 if self._is_overview_split() else 24
        cols = [
            ColumnDef("#", 3, "r"),
            ColumnDef("Title", None, "l"),
            ColumnDef("Album", album_w, "l"),
        ]
        if has_plays:
            cols.append(ColumnDef("Plays", 8, "r"))
        if has_dur:
            cols.append(ColumnDef("Time", 7, "r"))
        songs_tbl.set_icon_fn(None)
        songs_tbl.set_columns(cols, sort_col="#")
        rows = []
        for i, t in enumerate(songs, start=1):
            cells = [str(i), t.title, t.album or ""]
            if has_plays:
                cells.append(t.plays or "")
            if has_dur:
                cells.append(t.duration_text if t.duration else "")
            rows.append((cells, t))
        songs_tbl.set_data(rows, playing_index=playing_index_for(songs, self._playing_vid))
        if focus:
            songs_tbl.focus()

    def _render_overview_highlights(self) -> None:
        art = self._artist
        if art is None:
            return
        ov_right = self.query_one("#art-ov-right")
        latest_wrap = self.query_one("#art-ov-latest-wrap")
        rel_wrap = self.query_one("#art-ov-rel-wrap")
        sim_wrap = self.query_one("#art-ov-sim-wrap")

        albums_src = self._all_albums if self._all_albums is not None else art.albums
        singles_src = self._all_singles if self._all_singles is not None else art.singles
        combined_releases = sorted(
            (*albums_src, *singles_src),
            key=lambda a: a.year or "0000",
            reverse=True,
        )
        has_any = bool(combined_releases or art.related)
        ov_right.display = has_any

        if combined_releases:
            latest = combined_releases[0]
            self._latest_release = latest
            latest_wrap.display = True
            self.query_one("#art-ov-latest-cov", CoverWidget).set_url(latest.cover_url)
            self.query_one("#art-ov-latest-title", Static).update(
                Text(latest.title, style=f"bold {TEXT}", overflow="ellipsis", no_wrap=True)
            )
            self.query_one("#art-ov-latest-sub", Static).update(
                Text(latest.subtitle or latest.kind, style=DIM, overflow="ellipsis", no_wrap=True)
            )

            rel_wrap.display = True
            grid_items = (
                combined_releases[1:9] if len(combined_releases) >= 2 else combined_releases
            )
            cards = [
                GridCardItem(a.title, a.subtitle or a.kind, a.cover_url, a)
                for a in grid_items
            ]
            self.query_one("#art-ov-rel-grid", CoverGrid).set_items(cards)
        else:
            self._latest_release = None
            latest_wrap.display = False
            rel_wrap.display = False

        if art.related:
            sim_wrap.display = True
            sim_rows = [
                ([str(i), a.name, a.subtitle or "Artist"], a)
                for i, a in enumerate(art.related[:6], start=1)
            ]
            self.query_one("#art-ov-sim-tbl", TrackTable).set_data(sim_rows)
        else:
            sim_wrap.display = False

        self._sync_overview_layout()

    def switch_tab(self, tab: str) -> None:
        self._active_tab = tab
        self.query_one("#art-tabs", SectionTabs).set_active(tab)
        if tab == "overview":
            self.footer_hints = _HINTS_OVERVIEW
        elif tab == "songs":
            self.footer_hints = _HINTS_SONGS
        elif tab in ("albums", "singles"):
            self.footer_hints = _HINTS_RELEASES
        else:
            self.footer_hints = _HINTS_OTHER
        if self.is_mounted:
            self.post_message(RefreshFooter())

        art = self._artist
        if art is None:
            return

        self.query_one("#art-body-skel").display = False
        self.query_one("#art-body-scroll").display = True
        body_panel = self.query_one("#art-body-panel")
        ov_grid = self.query_one("#art-ov-grid")
        ov_right = self.query_one("#art-ov-right")
        songs_ctrl = self.query_one("#art-songs-ctrl")
        songs_tbl = self.query_one("#art-songs-tbl", TrackTable)
        songs_media_wrap = self.query_one("#art-songs-media-wrap")
        rel_wrap = self.query_one("#art-rel-wrap")
        misc_tbl = self.query_one("#art-misc-tbl", TrackTable)

        ov_grid.display = tab in ("overview", "videos")
        songs_tbl.display = tab in ("overview", "videos")
        songs_ctrl.display = False
        ov_right.display = False
        songs_media_wrap.display = tab == "songs"
        rel_wrap.display = tab in ("albums", "singles")
        misc_tbl.display = tab in ("playlists", "similar")

        if tab == "overview":
            all_songs = art.songs
            has_more_than_10 = len(all_songs) > 10
            songs = all_songs[:10] if (self._top10_only and has_more_than_10) else all_songs
            self._visible_tracks = songs
            if has_more_than_10:
                songs_ctrl.display = True
                crumb = Text()
                crumb.append("TOP SONGS · ", style=f"bold {ACCENT}")
                crumb.append(
                    f"Top 10 of {len(all_songs)}"
                    if self._top10_only
                    else f"All {len(all_songs)} Songs",
                    style=TEXT,
                )
                self.query_one("#art-songs-crumb", Static).update(crumb)
                tog = self.query_one("#art-songs-toggle", Button)
                tog.label = f"Show All ({len(all_songs)})" if self._top10_only else "Top 10 Only"
                tog.refresh(layout=True)
                body_panel.border_title = (
                    f"Overview · Top 10 Songs (of {len(all_songs)})"
                    if self._top10_only
                    else f"Overview · Songs ({len(all_songs)})"
                )
            else:
                body_panel.border_title = f"Overview · {art.summary.name}"
            self._render_overview_highlights()
            self._populate_songs_table(songs)
        elif tab == "songs":
            all_songs = art.songs
            self._visible_tracks = all_songs
            self._sync_overview_layout()
            body_panel.border_title = f"Songs ({len(all_songs)})"
            crumb = Text(overflow="ellipsis", no_wrap=True)
            crumb.append(f"{art.summary.name} \u203a ", style=DIM)
            crumb.append(f"All Songs ({len(all_songs)})", style=TEXT)
            self.query_one("#art-songs-media-crumb", Static).update(crumb)
            media = self.query_one("#art-songs-media", MediaList)
            media.set_items(
                [media_item_from(t) for t in all_songs],
                playing_index=playing_index_for(all_songs, self._playing_vid),
            )
            media.focus()
        elif tab == "videos":
            vids = art.videos
            self._visible_tracks = vids
            self._sync_overview_layout()
            body_panel.border_title = f"Videos ({len(vids)})"
            has_plays = any(t.plays for t in vids)
            has_dur = any(t.duration for t in vids)
            cols = [ColumnDef("#", 3, "r"), ColumnDef("Title", None, "l")]
            if has_plays:
                cols.append(ColumnDef("Views", 9, "r"))
            if has_dur:
                cols.append(ColumnDef("Time", 8, "r"))
            songs_tbl.set_icon_fn(lambda _p, ic: ic.video)
            songs_tbl.set_columns(cols, sort_col="#")
            rows = []
            for i, t in enumerate(vids, start=1):
                cells = [str(i), t.title]
                if has_plays:
                    cells.append(t.plays or "")
                if has_dur:
                    cells.append(t.duration_text if t.duration else "")
                rows.append((cells, t))
            songs_tbl.set_data(rows, playing_index=playing_index_for(vids, self._playing_vid))
            songs_tbl.focus()
        elif tab in ("albums", "singles"):
            self._sync_overview_layout()
            items = self._all_albums if tab == "albums" else self._all_singles
            cnt = len(items) if items is not None else len(
                art.albums if tab == "albums" else art.singles
            )
            body_panel.border_title = f"{'Albums' if tab == 'albums' else 'Singles & EPs'} ({cnt})"
            self._render_releases(tab)
        elif tab == "playlists":
            self._sync_overview_layout()
            body_panel.border_title = f"Playlists ({len(art.playlists)})"
            rows = [
                ([str(i), p.title, p.count or p.author or "Playlist"], p)
                for i, p in enumerate(art.playlists, start=1)
            ]
            misc_tbl.set_data(rows)
            misc_tbl.focus()
        elif tab == "similar":
            self._sync_overview_layout()
            body_panel.border_title = f"Similar Artists ({len(art.related)})"
            rows = [
                ([str(i), a.name, a.subtitle or "Artist"], a)
                for i, a in enumerate(art.related, start=1)
            ]
            misc_tbl.set_data(rows)
            misc_tbl.focus()

    def _render_releases(self, tab: str) -> None:
        art = self._artist
        if art is None:
            return
        is_album = tab == "albums"
        items = self._all_albums if is_album else self._all_singles
        params = art.albums_params if is_album else art.singles_params

        if items is None:
            items = art.albums if is_album else art.singles
            if params:
                self._fetch_full_releases(tab, params, "Album" if is_album else "Single")

        if not self._sort_newest:
            items = tuple(sorted(items, key=lambda x: x.year or "0000"))
        else:
            items = tuple(sorted(items, key=lambda x: x.year or "9999", reverse=True))

        crumb = Text(overflow="ellipsis", no_wrap=True)
        crumb.append(f"{art.summary.name} \u203a ", style=DIM)
        crumb.append("Albums" if is_album else "Singles & EPs", style=TEXT)
        self.query_one("#art-rel-crumb", Static).update(crumb)

        grid = self.query_one("#art-rel-grid", CoverGrid)
        tbl = self.query_one("#art-rel-tbl", TrackTable)
        grid.display = self._grid_mode
        tbl.display = not self._grid_mode

        if self._grid_mode:
            cards = [GridCardItem(a.title, a.subtitle or a.kind, a.cover_url, a) for a in items]
            grid.set_items(cards, cover_size=self._cover_size)
            grid.focus()
        else:
            rows = [
                ([str(i), a.title, a.kind, a.year or ""], a) for i, a in enumerate(items, start=1)
            ]
            tbl.set_data(rows)
            tbl.focus()

    def _fetch_full_releases(self, tab: str, params: str, kind: str) -> None:
        cancel_tasks(self._rel_task)
        panel = self.query_one("#art-body-panel")
        self.bind_panel_loader(panel, True)
        art = self._artist
        target_id = (
            (art.albums_browse_id if tab == "albums" else art.singles_browse_id)
            if art is not None
            else None
        ) or self.browse_id

        async def _job() -> None:
            try:
                res = await self._catalog.artist_releases(target_id, params, kind)
            except asyncio.CancelledError:
                raise
            except CATALOG_ERRORS:
                res = ()
            if not self.is_mounted:
                return
            self.bind_panel_loader(panel, False)
            if res:
                prev = (
                    art.albums
                    if (tab == "albums" and art is not None)
                    else (art.singles if art is not None else ())
                )
                if tab == "albums":
                    self._all_albums = res
                else:
                    self._all_singles = res
                self._refresh_catalog_counts()
                if self._active_tab == tab and res != prev:
                    panel.border_title = (
                        f"{'Albums' if tab == 'albums' else 'Singles & EPs'} ({len(res)})"
                    )
                    self._render_releases(tab)

        self._rel_task = asyncio.create_task(_job())

    def _sync_size_buttons(self) -> None:
        for s in ("S", "M", "L"):
            self.query_one(f"#art-sz-{s}", Button).set_class(s == self._cover_size, "is-selected")
        sort_btn = self.query_one("#art-rel-sort", Button)
        sort_btn.label = "Sort: Newest" if self._sort_newest else "Sort: Oldest"
        sort_btn.refresh(layout=True)
        view_btn = self.query_one("#art-rel-view", Button)
        view_btn.label = "Grid" if not self._grid_mode else "List"
        view_btn.refresh(layout=True)

    def on_section_tabs_tab_selected(self, event: SectionTabs.TabSelected) -> None:
        if event.widget.id == "art-tabs":
            self.switch_tab(event.tab_id)

    def on_player_state(self, state: PlaybackState) -> None:
        vid = state.track.video_id if (state.track and not state.restored) else None
        if vid == self._playing_vid:
            return
        self._playing_vid = vid
        if not (self._artist and self.is_mounted):
            return
        if self._active_tab in ("overview", "videos"):
            tracks = self._artist.videos if self._active_tab == "videos" else self._visible_tracks
            self.query_one("#art-songs-tbl", TrackTable).set_playing_index(
                playing_index_for(tracks, vid)
            )
        elif self._active_tab == "songs":
            self.query_one("#art-songs-media", MediaList).set_playing_index(
                playing_index_for(self._artist.songs, vid)
            )

    def _start_artist_radio(self) -> None:
        if self._artist is None:
            return
        if self._artist.radio_id:
            self.post_message(StartRadio(playlist_id=self._artist.radio_id))
        elif self._artist.songs:
            self.post_message(StartRadio(video_id=self._artist.songs[0].video_id))

    def on_button_pressed(self, event: Button.Pressed) -> None:
        bid = event.button.id or ""
        if self._artist is None:
            return
        if bid == "art-songs-toggle":
            self._top10_only = not self._top10_only
            if self._active_tab in ("overview", "songs"):
                self.switch_tab(self._active_tab)
        elif bid == "art-play" and self._artist.songs:
            self.post_message(PlayTracks(self._artist.songs, start=0))
        elif bid == "art-shf" and self._artist.songs:
            self.post_message(PlayTracks(self._artist.songs, start=0, shuffle=True))
        elif bid == "art-queue" and self._artist.songs:
            self.post_message(QueueTracks(self._artist.songs))
        elif bid == "art-save":
            self.post_message(SaveBookmark(self._artist.summary))
        elif bid == "art-radio":
            self._start_artist_radio()
        elif bid == "art-latest-play" and self._latest_release is not None:
            self.post_message(PlayEntityDirect(self._latest_release))
        elif bid == "art-latest-open" and self._latest_release is not None:
            self.post_message(
                OpenAlbum(self._latest_release.browse_id, self._latest_release.title)
            )
        elif bid.startswith("art-sz-"):
            self._cover_size = bid.removeprefix("art-sz-")
            self._sync_size_buttons()
            if self._active_tab in ("albums", "singles"):
                self._render_releases(self._active_tab)
        elif bid == "art-rel-sort":
            self._sort_newest = not self._sort_newest
            self._sync_size_buttons()
            if self._active_tab in ("albums", "singles"):
                self._render_releases(self._active_tab)
        elif bid == "art-rel-view":
            self._grid_mode = not self._grid_mode
            self._sync_size_buttons()
            if self._active_tab in ("albums", "singles"):
                self._render_releases(self._active_tab)

    def on_track_table_row_activated(self, event: TrackTable.RowActivated) -> None:
        p = event.payload
        if isinstance(p, Track) and self._artist:
            tracks = self._artist.videos if self._active_tab == "videos" else self._artist.songs
            self.post_message(PlayTracks(tracks, start=event.index))
        elif isinstance(p, AlbumSummary):
            self.post_message(OpenAlbum(p.browse_id, p.title))
        elif isinstance(p, PlaylistSummary):
            self.post_message(OpenPlaylist(p.browse_id, p.title))
        elif isinstance(p, ArtistSummary):
            self.post_message(OpenArtist(p.browse_id, p.name))

    def on_track_table_row_play(self, event: TrackTable.RowPlay) -> None:
        p = event.payload
        if isinstance(p, Track) and self._artist:
            tracks = self._artist.videos if self._active_tab == "videos" else self._artist.songs
            self.post_message(PlayTracks(tracks, start=event.index))
        elif isinstance(p, (AlbumSummary, PlaylistSummary, ArtistSummary)):
            self.post_message(PlayEntityDirect(p))

    def on_track_table_row_queued(self, event: TrackTable.RowQueued) -> None:
        p = event.payload
        if isinstance(p, Track):
            self.post_message(QueueTracks([p]))
        elif isinstance(p, (AlbumSummary, PlaylistSummary, ArtistSummary)):
            self.post_message(QueueEntityDirect(p))

    def on_track_table_row_play_next(self, event: TrackTable.RowPlayNext) -> None:
        p = event.payload
        if isinstance(p, Track):
            self.post_message(PlayNextTracks([p]))
        elif isinstance(p, (AlbumSummary, PlaylistSummary, ArtistSummary)):
            self.post_message(PlayNextEntityDirect(p))

    def on_media_list_item_activated(self, event: MediaList.ItemActivated) -> None:
        if event.widget.id == "art-songs-media" and self._artist:
            self.post_message(PlayTracks(self._artist.songs, start=event.index))

    def on_media_list_item_play(self, event: MediaList.ItemPlay) -> None:
        if event.widget.id == "art-songs-media" and self._artist:
            self.post_message(PlayTracks(self._artist.songs, start=event.index))

    def on_media_list_item_queued(self, event: MediaList.ItemQueued) -> None:
        if event.widget.id == "art-songs-media" and isinstance(event.payload, Track):
            self.post_message(QueueTracks([event.payload]))

    def on_media_list_item_play_next(self, event: MediaList.ItemPlayNext) -> None:
        if event.widget.id == "art-songs-media" and isinstance(event.payload, Track):
            self.post_message(PlayNextTracks([event.payload]))

    def on_media_list_item_open_album(self, event: MediaList.ItemOpenAlbum) -> None:
        if event.widget.id == "art-songs-media" and isinstance(event.payload, Track):
            self.post_message(OpenTrackAlbum(event.payload))

    def on_media_list_item_open_artist(self, event: MediaList.ItemOpenArtist) -> None:
        if event.widget.id == "art-songs-media" and isinstance(event.payload, Track):
            self.post_message(OpenTrackArtist(event.payload))

    def on_cover_grid_item_activated(self, event: CoverGrid.ItemActivated) -> None:
        if isinstance(event.payload, AlbumSummary):
            self.post_message(OpenAlbum(event.payload.browse_id, event.payload.title))

    def on_cover_grid_item_play(self, event: CoverGrid.ItemPlay) -> None:
        if isinstance(event.payload, AlbumSummary):
            self.post_message(PlayEntityDirect(event.payload))

    def on_cover_grid_item_queued(self, event: CoverGrid.ItemQueued) -> None:
        if isinstance(event.payload, (AlbumSummary, PlaylistSummary, ArtistSummary)):
            self.post_message(QueueEntityDirect(event.payload))

    def on_cover_grid_item_play_next(self, event: CoverGrid.ItemPlayNext) -> None:
        if isinstance(event.payload, (AlbumSummary, PlaylistSummary, ArtistSummary)):
            self.post_message(PlayNextEntityDirect(event.payload))

    def on_cover_grid_item_saved(self, event: CoverGrid.ItemSaved) -> None:
        if isinstance(event.payload, (AlbumSummary, PlaylistSummary, ArtistSummary)):
            self.post_message(SaveBookmark(event.payload))

    def on_cover_grid_size_changed(self, event: CoverGrid.SizeChanged) -> None:
        if event.grid.id == "art-rel-grid":
            self._cover_size = event.size
            self._sync_size_buttons()

    def _step_overview_focus(self, direction: int) -> bool:
        if self._active_tab != "overview":
            return False
        songs_tbl = self.query_one("#art-songs-tbl", TrackTable)
        rel_wrap = self.query_one("#art-ov-rel-wrap")
        rel_grid = self.query_one("#art-ov-rel-grid", CoverGrid)
        sim_wrap = self.query_one("#art-ov-sim-wrap")
        sim_tbl = self.query_one("#art-ov-sim-tbl", TrackTable)
        scroll = self.query_one("#art-body-scroll", VerticalScroll)

        order = []
        if songs_tbl.display and songs_tbl.rows:
            order.append(songs_tbl)
        if rel_wrap.display and rel_grid.items:
            order.append(rel_grid)
        if sim_wrap.display and sim_tbl.rows:
            order.append(sim_tbl)
        if len(order) < 2:
            return False

        cur_w = self.app.focused
        if cur_w not in order:
            return False
        idx = order.index(cur_w)
        nxt_idx = idx + direction
        if not (0 <= nxt_idx < len(order)):
            return False
        target = order[nxt_idx]
        if target is songs_tbl and direction < 0:
            songs_tbl.set_cursor(len(songs_tbl.rows) - 1, scroll=False)
        elif target is sim_tbl and direction > 0:
            sim_tbl.set_cursor(0, scroll=False)
        target.focus()
        scroll.scroll_to_widget(target, animate=False)
        return True

    def on_key(self, event: Key) -> None:
        if event.key in ("tab", "shift+tab"):
            ids = [t[0] for t in _ARTIST_TABS]
            step = -1 if event.key == "shift+tab" else 1
            nxt = ids[(ids.index(self._active_tab) + step) % len(ids)]
            self.switch_tab(nxt)
            event.stop()
        elif (event.key in ("down", "j") and self._step_overview_focus(1)) or (
            event.key in ("up", "k") and self._step_overview_focus(-1)
        ):
            event.stop()
        elif (
            event.key == "t"
            and self._artist
            and self._active_tab == "overview"
            and len(self._artist.songs) > 10
        ):
            self._top10_only = not self._top10_only
            self.switch_tab("overview")
            event.stop()
        elif event.key == "v" and self._artist and self._active_tab in ("albums", "singles"):
            self._grid_mode = not self._grid_mode
            self._sync_size_buttons()
            self._render_releases(self._active_tab)
            event.stop()
        elif event.key == "r" and self._artist:
            self._start_artist_radio()
            event.stop()
        elif event.key == "w" and self._artist:
            self.post_message(SaveBookmark(self._artist.summary))
            event.stop()

    def on_settings_changed(self, icons: IconSet, cover_size: str, protocol: str) -> None:
        self._icons = icons
        self._cover_size = cover_size
        self._sync_size_buttons()
        self.query_one("#art-cov", CoverWidget).set_protocol(protocol)
        self.query_one("#art-ov-latest-cov", CoverWidget).set_protocol(protocol)
        media = self.query_one("#art-songs-media", MediaList)
        media.set_icons(icons)
        media.set_protocol(protocol)
        rel_grid = self.query_one("#art-rel-grid", CoverGrid)
        rel_grid.set_protocol(protocol)
        rel_grid.set_icons(icons)
        if self._active_tab in ("albums", "singles") and self._artist:
            self._render_releases(self._active_tab)
        else:
            rel_grid.set_cover_size(cover_size)
        ov_grid = self.query_one("#art-ov-rel-grid", CoverGrid)
        ov_grid.set_protocol(protocol)
        ov_grid.set_icons(icons)
        for tid in ("#art-songs-tbl", "#art-rel-tbl", "#art-misc-tbl", "#art-ov-sim-tbl"):
            self.query_one(tid, TrackTable).set_icons(icons)
        for bid in ("#art-play", "#art-latest-play"):
            btn = self.query_one(bid, Button)
            btn.label = f"{icons.playing} Play"
            btn.refresh(layout=True)
