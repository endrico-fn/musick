"""Home player view: Cover, Queue/Search/Explore, Cava spectrum, and Synced Lyrics."""

from __future__ import annotations

import asyncio
from typing import Any, Literal, cast

from rich.text import Text
from textual.app import ComposeResult
from textual.containers import Center, Horizontal, Vertical, VerticalScroll
from textual.events import Key, Resize
from textual.widgets import Input, Static

from musick.config import Settings
from musick.domain import (
    AlbumSummary,
    ArtistSummary,
    HomeFeed,
    PlaylistSummary,
    SearchItem,
    Track,
)
from musick.player.controller import PlaybackState, PlayerEngine
from musick.services.catalog import CATALOG_ERRORS, Catalog
from musick.services.images import ImageService
from musick.services.search import (
    _PREFIX_MAP,
    HistoryStore,
    SearchCategory,
    SearchService,
    Suggestion,
    parse_query_prefix,
)
from musick.ui.messages import (
    NotifyStatus,
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
    CavaVisualizer,
    ColumnDef,
    CoverWidget,
    LyricsBox,
    MediaList,
    SectionTabs,
    Skeleton,
    TrackTable,
    media_item_from,
)

from .base import BaseView, cancel_tasks, compute_side_panel_geometry, handle_queue_shortcut

HomePanelMode = Literal["queue", "suggest", "results", "explore", "lyrics"]

_CATS: tuple[tuple[SearchCategory, str], ...] = (
    ("all", "All"),
    ("songs", "Songs"),
    ("videos", "Videos"),
    ("albums", "Albums"),
    ("artists", "Artists"),
    ("playlists", "Playlists"),
    ("community", "Community"),
)
_CAT_IDS: frozenset[str] = frozenset(cid for cid, _ in _CATS)

_HINTS_QUEUE: tuple[tuple[str, str], ...] = (
    ("/", "search"),
    ("e", "explore"),
    ("Enter", "play"),
    ("T", "play next"),
    ("d", "remove"),
    ("z", "undo"),
    ("Space", "play/pause"),
    ("y", "lyrics"),
    ("t", "id lyrics"),
)

_HINTS_SUGGEST: tuple[tuple[str, str], ...] = (
    ("↑↓", "pick suggest"),
    ("Tab", "complete"),
    ("Enter", "search"),
    ("Esc", "exit prompt"),
)

_HINTS_RESULTS: tuple[tuple[str, str], ...] = (
    ("/", "search"),
    ("Enter", "play/open"),
    ("N", "next up"),
    ("+", "queue"),
    ("a", "album"),
    ("A", "artist"),
    ("r", "radio"),
    ("Esc", "queue"),
)

_HINTS_EXPLORE: tuple[tuple[str, str], ...] = (
    ("/", "search"),
    ("↑↓", "move"),
    ("Enter", "open"),
    ("p", "play"),
    ("w", "save"),
    ("Esc", "queue"),
    ("q", "quit"),
)

_HINTS_LYRICS: tuple[tuple[str, str], ...] = (
    ("y", "close lyrics"),
    ("t", "id lyrics"),
    ("j k", "scroll"),
    ("Space", "play/pause"),
    ("/", "search"),
    ("e", "explore"),
    ("Esc", "back"),
)


class HomeView(BaseView):
    can_focus = False
    tab_id = "home"
    footer_hints = _HINTS_QUEUE

    def __init__(
        self,
        catalog: Catalog,
        search_service: SearchService,
        history: HistoryStore,
        images: ImageService,
        engine: PlayerEngine,
        settings: Settings,
        **kw,
    ) -> None:
        super().__init__(**kw)
        self._catalog = catalog
        self._search = search_service
        self._history = history
        self._images = images
        self._engine = engine
        self._settings = settings
        self._icons: IconSet = icons_for(settings.display.icons)

        self._mode: HomePanelMode = "queue"
        self._pre_lyrics_mode: HomePanelMode = "queue"
        self._category: SearchCategory = "all"
        self._last_query: str = ""
        self._pending_query: tuple[str, bool] | None = None
        self._state: PlaybackState = engine.state
        self._seen_queue_rev: int = -1
        self._seen_track_rev: int = -1
        self._results: tuple[SearchItem, ...] = ()
        self._explore_items: tuple[SearchItem, ...] = ()
        self._explore_region: str = ""
        self._suggestions: list[Suggestion] = []
        self._debounce = None
        self._suggest_task: asyncio.Task[None] | None = None
        self._search_task: asyncio.Task[None] | None = None
        self._explore_task: asyncio.Task[None] | None = None

    def compose(self) -> ComposeResult:
        with Vertical(id="home-workspace"):
            with Horizontal(id="home-top"):
                with Vertical(id="home-cover-panel", classes="panel") as cp:
                    cp.border_title = "Now Playing"
                    with Center(id="home-cov-wrap"):
                        yield CoverWidget(
                            "L",
                            images=self._images,
                            protocol=self._settings.display.image_protocol,
                            id="home-cov",
                        )

                with Vertical(id="home-main-panel", classes="panel") as mp:
                    mp.border_title = "Queue"
                    with Vertical(id="home-p2-prompt-wrap"):
                        with Horizontal(id="home-p2-prompt-row", classes="compact-row"):
                            yield Static(
                                Text(f"{self._icons.prompt} ", style=ACCENT),
                                id="home-p2-prompt-glyph",
                                classes="inline-label",
                            )
                            yield Input(
                                placeholder=(
                                    "Search songs, lyrics, videos, artists, URL… (v: a: al: pl: s:)"
                                ),
                                id="home-p2-input",
                            )
                            yield Static(
                                Text(" Esc", style=DIM),
                                id="home-p2-prompt-hint",
                                classes="inline-label",
                            )
                        yield SectionTabs(
                            _CATS,
                            active=self._category,
                            short_labels={"playlists": "Lists", "community": "Comm."},
                            id="home-p2-cat-tabs",
                        )

                    with VerticalScroll(id="home-p2-sugg-scroll"):
                        yield TrackTable(
                            [ColumnDef("Query", None, "l"), ColumnDef("Source", 12, "r")],
                            icons=self._icons,
                            show_header=False,
                            allow_none_cursor=True,
                            id="home-p2-sugg-tbl",
                        )

                    yield Skeleton("table_rows", rows=7, id="home-p2-skel")

                    with VerticalScroll(id="home-p2-list-scroll"):
                        yield MediaList(
                            icons=self._icons,
                            images=self._images,
                            protocol=self._settings.display.image_protocol,
                            empty_text="Queue is empty — press / to search or e to explore",
                            id="home-p2-list",
                        )

                    yield LyricsBox(blank_between=True, id="home-main-lyrics")

            with Horizontal(id="home-bottom"):
                with Vertical(id="home-lyrics-panel", classes="panel") as lp:
                    lp.border_title = "Lyrics"
                    yield LyricsBox(id="home-lyrics-box")
                    with VerticalScroll(id="home-details-scroll"):
                        yield Static("", id="home-details-title")
                        yield Static("", id="home-details-artist")
                        yield Static("", id="home-details-meta")

                with Vertical(id="home-cava-panel", classes="panel") as cvp:
                    cvp.border_title = "Cava"
                    yield CavaVisualizer(
                        palette=self._settings.display.cava_palette,
                        smoothing=self._settings.display.cava_smoothing,
                        enabled=self._settings.display.cava_enabled,
                        audio_source=self._settings.display.cava_audio_source,
                        id="home-cava",
                    )

    def on_mount(self) -> None:
        super().on_mount()
        self._apply_responsive_sizes()
        self._apply_mode_ui()
        self.on_player_state(self._engine.state)
        self.refresh_queue()
        if self._pending_query is not None:
            q, run = self._pending_query
            self._pending_query = None
            self.set_query(q, run=run)

    def on_unmount(self) -> None:
        super().on_unmount()
        if self._debounce:
            self._debounce.stop()
        cancel_tasks(self._suggest_task, self._search_task, self._explore_task)

    def invalidate_explore(self) -> None:
        self._explore_items = ()
        self._explore_region = ""

    def on_view_activated(self) -> None:
        if self.is_mounted:
            self.query_one("#home-cava", CavaVisualizer).set_active_visible(True)
            self._update_cava_subtitle()
            self.refresh_queue()
            self._refresh_cover_panel()

    def _update_cava_subtitle(self) -> None:
        cava = self.query_one("#home-cava", CavaVisualizer)
        active_audio = self._state.playing and not self._state.resolving
        cava_off = (
            not self._settings.display.cava_enabled
            or self._settings.display.cava_audio_source == "off"
        )
        self.query_one("#home-cava-panel").border_subtitle = (
            "Off"
            if cava_off
            else (cava.mode_label if active_audio and not self._state.paused else "")
        )

    def restore_focus(self) -> None:
        if self._mode == "suggest":
            self.query_one("#home-p2-input", Input).focus()
        elif self._mode == "lyrics":
            self.query_one("#home-main-lyrics", LyricsBox).focus()
        else:
            self.query_one("#home-p2-list", MediaList).focus()

    @property
    def is_lyrics_mode(self) -> bool:
        return self._mode == "lyrics"

    def enter_lyrics_mode(self) -> None:
        if self._mode == "lyrics":
            return
        if self._mode == "suggest":
            if self._debounce:
                self._debounce.stop()
            cancel_tasks(self._suggest_task)
            self._pre_lyrics_mode = "results" if self._results else "queue"
        else:
            self._pre_lyrics_mode = self._mode
        self._set_mode("lyrics")
        if self.is_mounted and self.display:
            self.restore_focus()

    def exit_lyrics_mode(self, *, focus: bool = True) -> None:
        if self._mode != "lyrics":
            return
        target = self._pre_lyrics_mode
        if target in ("lyrics", "suggest"):
            target = "results" if self._results else "queue"
        self._set_mode(target)
        if focus and self.is_mounted and self.display:
            self.restore_focus()

    def on_view_deactivated(self) -> None:
        if self.is_mounted:
            self.query_one("#home-cava", CavaVisualizer).set_active_visible(False)

    def on_resize(self, event: Resize) -> None:
        self._apply_responsive_sizes()

    def _apply_responsive_sizes(self) -> None:
        if not self.ui_ready:
            return
        w = max(40, self.size.width or 96)
        h = max(12, self.size.height or 32)
        min_bot_h = max(5, min(10, round(h * 0.27)))
        left_w, top_panel_h, cov_w, cov_h = compute_side_panel_geometry(
            w,
            h,
            min_bottom_h=min_bot_h,
        )
        self.query_one("#home-cover-panel").styles.width = left_w
        self.query_one("#home-top").styles.height = top_panel_h
        self.query_one("#home-cov", CoverWidget).set_exact_size(cov_w, cov_h)

    # ── Mode & State Synchronization ───────────────────────────

    def _set_mode(self, mode: HomePanelMode) -> None:
        self._mode = mode
        if mode == "queue":
            self.footer_hints = _HINTS_QUEUE
        elif mode == "suggest":
            self.footer_hints = _HINTS_SUGGEST
        elif mode == "explore":
            self.footer_hints = _HINTS_EXPLORE
        elif mode == "lyrics":
            self.footer_hints = _HINTS_LYRICS
        else:
            self.footer_hints = _HINTS_RESULTS
        self._apply_mode_ui()
        self._refresh_cover_panel()
        self.post_message(RefreshFooter())

    def _apply_mode_ui(self) -> None:
        if not self.ui_ready:
            return
        prompt_wrap = self.query_one("#home-p2-prompt-wrap")
        sugg_scroll = self.query_one("#home-p2-sugg-scroll")
        list_scroll = self.query_one("#home-p2-list-scroll")
        skel = self.query_one("#home-p2-skel")
        main_lyrics = self.query_one("#home-main-lyrics", LyricsBox)
        compact_lyrics = self.query_one("#home-lyrics-box", LyricsBox)
        details_scroll = self.query_one("#home-details-scroll")
        panel = self.query_one("#home-main-panel")

        is_lyrics = self._mode == "lyrics"
        main_lyrics.display = is_lyrics
        compact_lyrics.display = not is_lyrics
        details_scroll.display = is_lyrics

        if is_lyrics:
            prompt_wrap.display = False
            sugg_scroll.display = False
            skel.display = False
            list_scroll.display = False
        elif self._mode == "queue":
            prompt_wrap.display = False
            sugg_scroll.display = False
            skel.display = False
            list_scroll.display = True
            panel.border_title = "Queue"
            self.refresh_queue()
        elif self._mode == "suggest":
            prompt_wrap.display = True
            sugg_scroll.display = True
            skel.display = False
            list_scroll.display = False
            panel.border_title = "Search"
            panel.border_subtitle = "Autosuggest"
        elif self._mode == "explore":
            prompt_wrap.display = False
            sugg_scroll.display = False
            list_scroll.display = True
            panel.border_title = f"Explore ({self._settings.display.region})"
            panel.border_subtitle = "Charts & New Releases"
        else:
            prompt_wrap.display = True
            sugg_scroll.display = False
            list_scroll.display = True
            panel.border_title = (
                f"Search Results ({len(self._results)})" if self._results else "Search Results"
            )
            panel.border_subtitle = self._last_query

        self._sync_lyrics_and_details()

    def activate_search(self) -> None:
        """Open inline search prompt at top of Panel 2 and focus it."""
        sugg_tbl = self.query_one("#home-p2-sugg-tbl", TrackTable)
        sugg_tbl.reset_cursor()
        self._set_mode("suggest")
        inp = self.query_one("#home-p2-input", Input)
        inp.focus()
        if not inp.value.strip() and not self._settings.privacy.save_history:
            self._suggestions = []
            sugg_tbl.set_data([])
        self._schedule_suggestions(inp.value)

    def handle_slash(self) -> bool:
        self.activate_search()
        return True

    def set_query(self, query: str, run: bool = False) -> None:
        if not self.is_mounted or not self.ui_ready:
            self._pending_query = (query, run)
            return
        if run and query.strip():
            self._set_input_silently(query)
            self.run_search(query)
        else:
            self.activate_search()
            self._set_input_silently(query)
            self._schedule_suggestions(query)

    def _set_category(self, cat: SearchCategory, rerun: bool) -> None:
        self._category = cat
        self.query_one("#home-p2-cat-tabs", SectionTabs).set_active(cat)
        if rerun:
            q = self.query_one("#home-p2-input", Input).value
            if q.strip():
                self.run_search(q)

    def on_section_tabs_tab_selected(self, event: SectionTabs.TabSelected) -> None:
        if event.widget.id == "home-p2-cat-tabs" and event.tab_id in _CAT_IDS:
            self._set_category(cast(SearchCategory, event.tab_id), rerun=True)
            event.stop()

    def handle_input_escape(self) -> bool:
        inp = self.query_one("#home-p2-input", Input)
        if not inp.has_focus:
            return False
        if self._debounce:
            self._debounce.stop()
        if self._suggest_task and not self._suggest_task.done():
            self._suggest_task.cancel()
        if self._results:
            self._set_mode("results")
            self.query_one("#home-p2-list", MediaList).focus()
        else:
            self._close_search_to_queue()
        return True

    def handle_escape(self) -> bool:
        if self._mode in ("queue", "lyrics"):
            return False
        if self._mode == "suggest" and self._results:
            if self._debounce:
                self._debounce.stop()
            if self._suggest_task and not self._suggest_task.done():
                self._suggest_task.cancel()
            self._set_mode("results")
            self.query_one("#home-p2-list", MediaList).focus()
            return True
        self._close_search_to_queue()
        return True

    def _set_input_silently(self, value: str) -> None:
        inp = self.query_one("#home-p2-input", Input)
        if inp.value != value:
            with inp.prevent(Input.Changed):
                inp.value = value
        inp.cursor_position = len(value)

    def _close_search_to_queue(self) -> None:
        if self._debounce:
            self._debounce.stop()
        for task in (self._suggest_task, self._search_task, self._explore_task):
            if task and not task.done():
                task.cancel()
        self.bind_panel_loader(self.query_one("#home-main-panel"), False)
        self._results = ()
        self._suggestions = []
        self.query_one("#home-p2-sugg-tbl", TrackTable).set_data([])
        self._set_input_silently("")
        self._set_mode("queue")
        self.query_one("#home-p2-list", MediaList).focus()

    # ── Queue & Player State Updates ───────────────────────────

    def refresh_queue(self) -> None:
        if not self.ui_ready or self._mode != "queue":
            return
        q = self._engine.queue
        ml = self.query_one("#home-p2-list", MediaList)
        ml.set_empty_text("Queue is empty — press / to search or e to explore")
        items = [media_item_from(t, role=q.role_at(i)) for i, t in enumerate(q.tracks)]
        playing_idx = None
        if (
            q.index >= 0
            and self._state.track is not None
            and not self._state.restored
            and q.current
            and q.current.video_id == self._state.track.video_id
        ):
            playing_idx = q.index
        ml.set_items(items, playing_index=playing_idx)

        self.query_one("#home-main-panel").border_subtitle = q.summary_text()
        self._seen_queue_rev = self._state.queue_rev
        self._seen_track_rev = self._state.track_rev

    def on_player_state(self, state: PlaybackState) -> None:
        self._state = state
        if not self.ui_ready:
            return
        self._refresh_cover_panel()
        self._sync_lyrics_and_details()

        # Sync Cava visualizer
        active_audio = state.playing and not state.resolving
        cava = self.query_one("#home-cava", CavaVisualizer)
        cava.set_playback_state(
            playing=active_audio,
            paused=state.paused,
            track_key=state.track.video_id if state.track else "",
            reduced_motion=self._settings.display.reduced_motion,
        )
        self._update_cava_subtitle()

        if self._mode == "queue":
            if (state.queue_rev, state.track_rev) != (self._seen_queue_rev, self._seen_track_rev):
                self.refresh_queue()
        elif self._mode in ("results", "explore"):
            self._sync_results_playing_index()

    def _sync_lyrics_and_details(self) -> None:
        if not self.ui_ready:
            return
        state = self._state
        has_track = state.track is not None
        lyr_panel = self.query_one("#home-lyrics-panel")
        main_panel = self.query_one("#home-main-panel")

        if self._mode == "lyrics":
            self.query_one("#home-main-lyrics", LyricsBox).set_lyrics(
                state.lyrics,
                state.active_lyric_idx,
                has_track=has_track,
                show_translation=state.show_translation,
            )
            main_panel.border_title = "Synced Lyrics" if state.lyrics.synced else "Lyrics"
            if state.translating_lyrics:
                main_panel.border_subtitle = "Translating…"
            elif state.show_translation and state.lyrics.has_translations:
                main_panel.border_subtitle = "Bilingual ID · t toggle"
            else:
                main_panel.border_subtitle = "t translate ID" if state.lyrics else ""

            lyr_panel.border_title = "Details"
            lyr_panel.border_subtitle = ""
            self._sync_track_details(state)
            return

        self.query_one("#home-lyrics-box", LyricsBox).set_lyrics(
            state.lyrics,
            state.active_lyric_idx,
            has_track=has_track,
            show_translation=state.show_translation,
        )
        lyr_panel.border_title = "Lyrics"
        if state.translating_lyrics:
            base_src = state.lyrics.source or "Lyrics"
            lyr_panel.border_subtitle = f"{base_src} · Translating…"
        elif state.lyrics.source:
            parts = [state.lyrics.source]
            if state.lyrics.synced:
                parts.append("Synced")
            if state.show_translation and state.lyrics.has_translations:
                parts.append("ID")
            lyr_panel.border_subtitle = " · ".join(parts)
        else:
            lyr_panel.border_subtitle = ""

    def _sync_track_details(self, state: PlaybackState) -> None:
        title_w = self.query_one("#home-details-title", Static)
        artist_w = self.query_one("#home-details-artist", Static)
        meta_w = self.query_one("#home-details-meta", Static)
        track = state.track
        if track is None:
            title_w.update(Text("Nothing playing", style=DIM))
            artist_w.update("")
            artist_w.display = False
            meta_w.update("")
            meta_w.display = False
            return

        title_w.update(Text(track.title, style=f"bold {ACCENT}"))
        artist_w.update(Text(track.artist_names, style=TEXT))
        artist_w.display = True
        src = state.lyrics.source or "None"
        mode = (
            "Synced LRC"
            if state.lyrics.synced
            else ("Plain text" if state.lyrics else "Unavailable")
        )
        tr_note = " + ID" if (state.show_translation and state.lyrics.has_translations) else ""
        meta_lines: list[str] = []
        if track.album:
            meta_lines.append(f"Album: {track.album}")
        meta_lines.append(f"Source: {src} · {mode}{tr_note}")
        meta_w.update(Text("\n".join(meta_lines), style=DIM))
        meta_w.display = True

    def _sync_results_playing_index(self) -> None:
        vid = (
            self._state.track.video_id if (self._state.track and not self._state.restored) else None
        )
        ml = self.query_one("#home-p2-list", MediaList)
        items = self._explore_items if self._mode == "explore" else self._results
        p_idx = None
        if vid:
            for i, it in enumerate(items):
                if isinstance(it, Track) and it.video_id == vid:
                    p_idx = i
                    break
        ml.set_playing_index(p_idx)

    def _refresh_cover_panel(self) -> None:
        if not self.ui_ready:
            return
        panel = self.query_one("#home-cover-panel")
        cov = self.query_one("#home-cov", CoverWidget)

        track = self._state.track
        if track is None:
            panel.border_title = "Cover"
            panel.border_subtitle = "Press / to search"
            cov.set_url(None)
            return

        panel.border_title = "Last Played" if self._state.restored else "Now Playing"
        if self._state.paused:
            panel.border_subtitle = "Paused"
        elif self._state.playing:
            panel.border_subtitle = "Playing"
        else:
            panel.border_subtitle = ""
        cov.set_url(track.cover_url)

    # ── Inline Autosuggest & Search ────────────────────────────

    def on_input_changed(self, event: Input.Changed) -> None:
        if event.input.id != "home-p2-input":
            return
        if not event.input.has_focus:
            return
        self.query_one("#home-p2-sugg-tbl", TrackTable).reset_cursor()
        if event.value.lstrip().lower().startswith(tuple(_PREFIX_MAP)):
            cat, _ = parse_query_prefix(event.value, default_kind=self._category)
            if cat != self._category:
                self._set_category(cat, rerun=False)
        if self._mode != "suggest":
            self._set_mode("suggest")
        self._schedule_suggestions(event.value)

    def on_input_submitted(self, event: Input.Submitted) -> None:
        if event.input.id != "home-p2-input":
            return
        sugg_tbl = self.query_one("#home-p2-sugg-tbl", TrackTable)
        if (
            self._mode == "suggest"
            and self._suggestions
            and 0 <= sugg_tbl.cursor < len(self._suggestions)
        ):
            chosen = self._suggestions[sugg_tbl.cursor]
            query = self._merge_prefix(event.value, chosen.text)
            self._set_input_silently(query)
        else:
            query = event.value
        self.run_search(query)

    @staticmethod
    def _merge_prefix(raw_input: str, suggestion_text: str) -> str:
        cat, _ = parse_query_prefix(raw_input, default_kind="all")
        if cat != "all" and ":" in raw_input:
            prefix = raw_input.split(":", 1)[0].strip() + ": "
            return prefix + suggestion_text
        return suggestion_text

    def _schedule_suggestions(self, raw: str) -> None:
        if self._debounce:
            self._debounce.stop()
        if not raw.strip() and not self._settings.privacy.save_history:
            self._suggestions = []
            self.query_one("#home-p2-sugg-tbl", TrackTable).set_data([])
            return
        self._debounce = self.set_timer(0.14, lambda: self._launch_suggest(raw))

    def _launch_suggest(self, raw: str) -> None:
        if self._suggest_task and not self._suggest_task.done():
            self._suggest_task.cancel()
        self._suggest_task = asyncio.create_task(self._fetch_suggestions(raw))

    async def _fetch_suggestions(self, raw: str) -> None:
        try:
            items = await self._search.autocomplete(raw, limit=8)
        except asyncio.CancelledError:
            raise
        except CATALOG_ERRORS:
            items = []
        self._suggestions = items
        if not self.is_mounted or self._mode != "suggest":
            return
        rows = [
            ([s.text, s.kind_hint or ("History" if s.from_history else "Suggest")], s)
            for s in items
        ]
        self.query_one("#home-p2-sugg-tbl", TrackTable).set_data(rows)

    def run_search(self, raw_query: str) -> None:
        cat, clean_q = parse_query_prefix(raw_query, default_kind=self._category)
        if not clean_q:
            return
        if cat != self._category:
            self._set_category(cat, rerun=False)
        if self._debounce:
            self._debounce.stop()
        for task in (self._suggest_task, self._search_task, self._explore_task):
            if task and not task.done():
                task.cancel()
        self._suggestions = []
        self.query_one("#home-p2-sugg-tbl", TrackTable).set_data([])
        self._history.record_query(clean_q)
        self._last_query = clean_q

        self._set_mode("results")
        panel = self.query_one("#home-main-panel")
        panel.border_title = f"Results: {clean_q}"
        self.query_one("#home-p2-skel").display = True
        self.query_one("#home-p2-list-scroll").display = False
        self.bind_panel_loader(panel, True)
        self._search_task = asyncio.create_task(self._perform_search(cat, clean_q))

    def open_explore(self) -> None:
        region = self._settings.display.region
        self._set_mode("explore")
        if self._explore_items and self._explore_region == region:
            ml = self.query_one("#home-p2-list", MediaList)
            ml.set_items([media_item_from(it) for it in self._explore_items], reset_cursor=True)
            ml.focus()
            return
        if self._explore_task and not self._explore_task.done():
            self._explore_task.cancel()
        panel = self.query_one("#home-main-panel")
        self.query_one("#home-p2-skel").display = True
        self.query_one("#home-p2-list-scroll").display = False
        self.bind_panel_loader(panel, True)
        self._explore_task = asyncio.create_task(self._load_explore(region))

    async def _load_explore(self, region: str) -> None:
        panel = self.query_one("#home-main-panel")
        try:
            feed: HomeFeed = await self._catalog.home_feed(region=region)
        except asyncio.CancelledError:
            raise
        except CATALOG_ERRORS as exc:
            if self.is_mounted and self._mode == "explore":
                self.bind_panel_loader(panel, False)
                self.query_one("#home-p2-skel").display = False
                self.query_one("#home-p2-list-scroll").display = True
                ml = self.query_one("#home-p2-list", MediaList)
                ml.set_empty_text(f"Explore failed: {exc}")
                ml.set_items([])
            return

        combined: list[SearchItem] = [*feed.featured, *feed.artists, *feed.playlists]
        self._explore_items = tuple(combined)
        self._explore_region = region
        if not self.is_mounted or self._mode != "explore":
            return
        self.bind_panel_loader(panel, False)
        self.query_one("#home-p2-skel").display = False
        self.query_one("#home-p2-list-scroll").display = True
        panel.border_title = f"Explore ({region}) · {len(combined)}"
        panel.border_subtitle = "New Releases · Artists · Charts"
        ml = self.query_one("#home-p2-list", MediaList)
        ml.set_empty_text("No explore items available")
        ml.set_items([media_item_from(it) for it in combined], reset_cursor=True, emit_cursor=True)
        if combined:
            ml.focus()

    async def _perform_search(self, cat: SearchCategory, query: str) -> None:
        panel = self.query_one("#home-main-panel")
        try:
            items = await self._search.search(cat, query, limit=40)
        except asyncio.CancelledError:
            raise
        except CATALOG_ERRORS as exc:
            if self.is_mounted and self._mode == "results":
                self.bind_panel_loader(panel, False)
                self.query_one("#home-p2-skel").display = False
                self.query_one("#home-p2-list-scroll").display = True
                ml = self.query_one("#home-p2-list", MediaList)
                ml.set_empty_text(f"Search failed: {exc}")
                ml.set_items([])
            return

        self._results = items
        if not self.is_mounted or self._mode != "results":
            return

        self.bind_panel_loader(panel, False)
        self.query_one("#home-p2-skel").display = False
        self.query_one("#home-p2-list-scroll").display = True
        panel.border_title = f"Results ({len(items)})"
        panel.border_subtitle = query

        ml = self.query_one("#home-p2-list", MediaList)
        ml.set_empty_text(f"No results for '{query}'")
        vid = (
            self._state.track.video_id if (self._state.track and not self._state.restored) else None
        )
        p_idx = None
        if vid:
            for i, it in enumerate(items):
                if isinstance(it, Track) and it.video_id == vid:
                    p_idx = i
                    break
        ml.set_items(
            [media_item_from(it) for it in items],
            playing_index=p_idx,
            reset_cursor=True,
            emit_cursor=True,
        )
        if items:
            ml.focus()

    # ── Event Handlers ─────────────────────────────────────────

    def on_key(self, event: Key) -> None:
        if not self.display:
            return
        inp = self.query_one("#home-p2-input", Input)
        sugg_tbl = self.query_one("#home-p2-sugg-tbl", TrackTable)

        if inp.has_focus and self._mode == "suggest":
            if event.key == "down" and self._suggestions:
                sugg_tbl.move_cursor(1)
                event.stop()
                return
            if event.key == "up" and self._suggestions:
                sugg_tbl.move_cursor(-1)
                event.stop()
                return
            if event.key == "tab":
                if self._suggestions:
                    idx = sugg_tbl.cursor if 0 <= sugg_tbl.cursor < len(self._suggestions) else 0
                    chosen = self._suggestions[idx]
                    merged = self._merge_prefix(inp.value, chosen.text)
                    self._set_input_silently(merged)
                    sugg_tbl.reset_cursor()
                    self._schedule_suggestions(merged)
                event.stop()
                return

        if not inp.has_focus:
            if (
                event.key in ("escape", "backspace")
                and self._mode not in ("queue", "lyrics")
                and self.handle_escape()
            ):
                event.stop()
                return
            if event.key == "e" and self._mode != "suggest":
                self.open_explore()
                event.stop()
                return
            if self._mode in ("results", "explore") and event.key == "r":
                cur = self.query_one("#home-p2-list", MediaList).current_payload
                if isinstance(cur, Track):
                    self.post_message(StartRadio(video_id=cur.video_id))
                    event.stop()
                    return
                if isinstance(cur, PlaylistSummary):
                    self.post_message(StartRadio(playlist_id=cur.browse_id))
                    event.stop()
                    return
            if self._mode == "results" and event.key in ("h", "l"):
                cat_ids = [c[0] for c in _CATS]
                step = 1 if event.key == "l" else -1
                nxt_cat = cat_ids[(cat_ids.index(self._category) + step) % len(cat_ids)]
                self._set_category(nxt_cat, rerun=True)
                event.stop()
                return
            if self._mode == "queue":
                ml = self.query_one("#home-p2-list", MediaList)
                cur_payload = ml.current_payload
                if handle_queue_shortcut(
                    self,
                    self._engine,
                    event.key,
                    ml.cursor,
                    set_cursor=lambda idx: ml.set_cursor(idx),
                    on_refresh=self.refresh_queue,
                    current_track=cur_payload if isinstance(cur_payload, Track) else None,
                ):
                    event.stop()

    def on_track_table_row_activated(self, event: TrackTable.RowActivated) -> None:
        if event.table.id == "home-p2-sugg-tbl" and isinstance(event.payload, Suggestion):
            inp = self.query_one("#home-p2-input", Input)
            query = self._merge_prefix(inp.value, event.payload.text)
            self._set_input_silently(query)
            self.run_search(query)

    def on_media_list_item_activated(self, event: MediaList.ItemActivated) -> None:
        if event.widget.id != "home-p2-list":
            return
        if self._mode == "queue":
            self.run_worker(self._engine.jump_to(event.index))
            return
        self._open_search_item(event.payload)

    def on_media_list_item_play(self, event: MediaList.ItemPlay) -> None:
        if event.widget.id != "home-p2-list":
            return
        if self._mode == "queue":
            self.run_worker(self._engine.jump_to(event.index))
            return
        item = event.payload
        if isinstance(item, Track):
            self.post_message(PlayTracks([item], start=0))
        elif isinstance(item, (AlbumSummary, PlaylistSummary, ArtistSummary)):
            self.post_message(PlayEntityDirect(item))

    def on_media_list_item_queued(self, event: MediaList.ItemQueued) -> None:
        if self._mode in ("results", "explore"):
            if isinstance(event.payload, Track):
                self.post_message(QueueTracks([event.payload]))
            elif isinstance(event.payload, (AlbumSummary, PlaylistSummary, ArtistSummary)):
                self.post_message(QueueEntityDirect(event.payload))

    def on_media_list_item_play_next(self, event: MediaList.ItemPlayNext) -> None:
        if isinstance(event.payload, Track):
            self.post_message(PlayNextTracks([event.payload]))
        elif isinstance(event.payload, (AlbumSummary, PlaylistSummary, ArtistSummary)):
            self.post_message(PlayNextEntityDirect(event.payload))

    def on_media_list_item_removed(self, event: MediaList.ItemRemoved) -> None:
        if self._mode == "queue" and self._engine.queue.tracks:
            self._engine.queue.remove(event.index)
            self._engine.notify_queue_changed()
            self.refresh_queue()

    def on_media_list_item_saved(self, event: MediaList.ItemSaved) -> None:
        if isinstance(event.payload, (AlbumSummary, PlaylistSummary, ArtistSummary)):
            self.post_message(SaveBookmark(event.payload))

    def on_media_list_item_open_album(self, event: MediaList.ItemOpenAlbum) -> None:
        item = event.payload
        if isinstance(item, Track):
            self.post_message(OpenTrackAlbum(item))
        elif isinstance(item, AlbumSummary):
            self.post_message(OpenAlbum(item.browse_id, item.title))
        else:
            self.post_message(NotifyStatus("No album associated with this item"))

    def on_media_list_item_open_artist(self, event: MediaList.ItemOpenArtist) -> None:
        item = event.payload
        if isinstance(item, Track):
            self.post_message(OpenTrackArtist(item))
        elif isinstance(item, ArtistSummary):
            self.post_message(OpenArtist(item.browse_id, item.name))
        elif isinstance(item, AlbumSummary):
            if item.artists and item.artists[0].id:
                self.post_message(OpenArtist(item.artists[0].id, item.artists[0].name))
            elif item.artists:
                self.post_message(OpenTrackArtist(Track("", item.title, item.artists)))
            else:
                self.post_message(NotifyStatus("No artist associated with this album"))
        else:
            self.post_message(NotifyStatus("No artist associated with this item"))

    def on_lyrics_box_line_seek(self, event: LyricsBox.LineSeek) -> None:
        self.run_worker(self._engine.seek_absolute(event.seconds))

    def on_cava_visualizer_mode_changed(self, event: CavaVisualizer.ModeChanged) -> None:
        if (
            self.is_mounted
            and self._state.playing
            and not self._state.paused
            and not self._state.resolving
        ):
            self.query_one("#home-cava-panel").border_subtitle = event.mode

    def _open_search_item(self, item: Any) -> None:
        if isinstance(item, Track):
            pool = self._explore_items if self._mode == "explore" else self._results
            tracks = [r for r in pool if isinstance(r, Track)] or [item]
            start = tracks.index(item) if item in tracks else 0
            self.post_message(PlayTracks(tracks, start=start))
        elif isinstance(item, ArtistSummary):
            self.post_message(OpenArtist(item.browse_id, item.name))
        elif isinstance(item, AlbumSummary):
            self.post_message(OpenAlbum(item.browse_id, item.title))
        elif isinstance(item, PlaylistSummary):
            self.post_message(OpenPlaylist(item.browse_id, item.title))

    def on_settings_changed(self, icons: IconSet, cover_size: str, protocol: str) -> None:
        del cover_size
        self._icons = icons
        self.query_one("#home-p2-prompt-glyph", Static).update(
            Text(f"{icons.prompt} ", style=ACCENT)
        )
        self.query_one("#home-cov", CoverWidget).set_protocol(protocol)
        self.query_one("#home-p2-sugg-tbl", TrackTable).set_icons(icons)
        ml = self.query_one("#home-p2-list", MediaList)
        ml.set_icons(icons)
        ml.set_protocol(protocol)
        cava = self.query_one("#home-cava", CavaVisualizer)
        cava.set_appearance(
            palette=self._settings.display.cava_palette,
            smoothing=self._settings.display.cava_smoothing,
            enabled=self._settings.display.cava_enabled,
            audio_source=self._settings.display.cava_audio_source,
        )
        self.on_player_state(self._state)
