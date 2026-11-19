"""Main Textual application shell uniting TopBar, view navigation stack, and NowPlayingBar."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from textual.app import App, ComposeResult
from textual.containers import Container, Vertical
from textual.events import Key
from textual.timer import Timer
from textual.widgets import Input, Static

from musick.config import Settings
from musick.config import load as load_settings
from musick.config import save as save_settings
from musick.domain import AlbumSummary, ArtistSummary, PlaylistSummary, Track
from musick.paths import settings_file
from musick.player import PlaybackState, PlayerEngine
from musick.services import (
    CATALOG_ERRORS,
    BookmarkStore,
    Catalog,
    HistoryStore,
    ImageService,
    LyricsService,
    SearchService,
    StreamResolver,
    YTMusicCatalog,
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
    SettingsUpdated,
    StartRadio,
)
from musick.ui.theme import ACCENT, MUSICK_THEME, icons_for
from musick.ui.views import (
    AlbumView,
    ArtistView,
    BaseView,
    HomeView,
    LibraryView,
    PlaylistView,
    QueueView,
    SettingsView,
    format_footer_hints,
)
from musick.ui.widgets import SHIMMER_CLOCK, NowPlayingBar, SeekBar, TopBar


class MusickApp(App[None]):
    CSS_PATH = Path(__file__).parent / "ui" / "styles.tcss"
    TITLE = "musick"
    MAX_NAV_STACK = 12

    def __init__(
        self,
        *,
        settings: Settings | None = None,
        catalog: Catalog | None = None,
        images: ImageService | None = None,
        history: HistoryStore | None = None,
        bookmarks: BookmarkStore | None = None,
        engine: PlayerEngine | None = None,
        **kw,
    ) -> None:
        super().__init__(**kw)
        self.settings = settings or load_settings(settings_file())
        self.catalog: Catalog = catalog or YTMusicCatalog(region=self.settings.display.region)
        self.images = images or ImageService(self.settings)
        self.history = history or HistoryStore(self.settings)
        self.bookmarks = bookmarks or BookmarkStore()
        self.search_service = SearchService(self.catalog, self.history, self.settings)
        self.lyrics_service = LyricsService(self.settings, catalog=self.catalog)
        self.resolver = StreamResolver(settings=self.settings)
        self.engine = engine or PlayerEngine(
            self.settings,
            self.resolver,
            self.lyrics_service,
            self.history,
            radio_fn=lambda vid: self.catalog.radio(video_id=vid, limit=20),
        )
        self._icons = icons_for(self.settings.display.icons)

        self._nav_stack: list[BaseView] = []
        self._active_view: BaseView | None = None
        self._root_views: dict[str, BaseView] = {}
        self._lyrics_pushed_from_stack: bool = False
        self._unsub_engine = None
        self._status_override: str | None = None
        self._flash_timer: Timer | None = None
        self._settings_ver: int = 0

    def compose(self) -> ComposeResult:
        with Vertical(id="app-frame"):
            yield TopBar("home", icons=self._icons, id="topbar")
            yield Container(id="main-scroll")
            yield NowPlayingBar(icons=self._icons, id="now-playing")
            yield Static("", id="footer-hints")

    def on_mount(self) -> None:
        self.register_theme(MUSICK_THEME)
        self.theme = "musick"
        SHIMMER_CLOCK.bind_app(self)
        SHIMMER_CLOCK.set_reduced_motion(self.settings.display.reduced_motion)

        if self.engine.state.track is None and self.history.recent_tracks:
            self.engine.restore_last_track(self.history.recent_tracks[0])

        self._root_views = {
            "home": HomeView(
                self.catalog,
                self.search_service,
                self.history,
                self.images,
                self.engine,
                self.settings,
            ),
            "library": LibraryView(self.bookmarks, self.history, self.settings),
            "queue": QueueView(self.engine, self.settings),
            "settings": SettingsView(self.settings, on_clear_local_data=self._clear_all_caches),
        }
        slot = self.query_one("#main-scroll", Container)
        for tid, view in self._root_views.items():
            view.display = tid == "home"
            slot.mount(view)
        self._unsub_engine = self.engine.subscribe(self._on_playback_state)
        self.query_one("#now-playing", NowPlayingBar).apply_state(self.engine.state)
        self._show_root("home")

    async def on_unmount(self) -> None:
        if self._flash_timer is not None:
            self._flash_timer.stop()
            self._flash_timer = None
        SHIMMER_CLOCK.unbind_app(self)
        if self._unsub_engine:
            self._unsub_engine()
            self._unsub_engine = None
        await self.engine.shutdown()
        await self.images.aclose()
        await self.lyrics_service.aclose()

    def _clear_all_caches(self) -> None:
        if hasattr(self.catalog, "clear_cache"):
            self.catalog.clear_cache()
        self.images.clear_cache()
        self.lyrics_service.clear_cache()
        self.resolver.clear_cache()
        self.history.clear()
        self.bookmarks.clear()
        self.engine.clear_restored()
        lib = self._root_views.get("library")
        if isinstance(lib, LibraryView) and lib.is_mounted:
            lib.refresh_library()

    def _mount_view(self, view: BaseView) -> None:
        if self._active_view is not None and self._active_view is not view:
            if self.focused is not None and self._active_view in self.focused.ancestors_with_self:
                self._active_view.save_focus(self.focused)
            self._active_view.on_view_deactivated()

        slot = self.query_one("#main-scroll", Container)
        for child in list(slot.children):
            if child is view or child in self._root_views.values() or child in self._nav_stack:
                if child is not view:
                    child.display = False
            else:
                child.remove()

        if view.parent is None:
            slot.mount(view)
        view.display = True
        self._active_view = view
        self.query_one("#topbar", TopBar).set_active(view.tab_id)
        if view.is_mounted:
            self._sync_mounted_view(view)
        else:
            view.call_after_refresh(self._sync_mounted_view, view)
        self._update_footer()

    def _sync_mounted_view(self, view: BaseView) -> None:
        if not self.is_mounted or not self.is_running:
            return
        if self._active_view is not view or not view.is_mounted:
            return
        view.sync_settings_if_needed(
            self._settings_ver,
            self._icons,
            self.settings.display.cover_size,
            self.settings.display.image_protocol,
        )
        view.on_player_state(self.engine.state)
        view.on_view_activated()
        if isinstance(view, LibraryView):
            view.refresh_library()
        elif isinstance(view, QueueView):
            view.refresh_queue()
        if self.focused is None or view not in self.focused.ancestors_with_self:
            view.restore_focus()

    def _show_root(self, tab_id: str) -> None:
        self._nav_stack.clear()
        self._lyrics_pushed_from_stack = False
        home = self._root_views.get("home")
        if isinstance(home, HomeView) and home.is_lyrics_mode:
            home.exit_lyrics_mode()
        view = self._root_views[tab_id]
        self._mount_view(view)

    def _push_view(self, view: BaseView) -> None:
        if self._active_view is not None:
            self._nav_stack.append(self._active_view)
            while len(self._nav_stack) > self.MAX_NAV_STACK:
                evicted = self._nav_stack.pop(0)
                if (
                    evicted is not view
                    and evicted not in self._root_views.values()
                    and evicted not in self._nav_stack
                    and evicted.parent is not None
                ):
                    evicted.remove()
        self._mount_view(view)

    def pop_view(self) -> None:
        if not self._nav_stack:
            return
        prev = self._nav_stack.pop()
        self._mount_view(prev)

    def _update_footer(self) -> None:
        if not self.is_mounted or not self.is_running:
            return
        footers = self.query("#footer-hints")
        if not footers:
            return
        footer = footers.first(Static)
        if self._status_override:
            footer.update(f"[{ACCENT}]{self._status_override}[/]")
            return
        hints = self._active_view.footer_hints if self._active_view else ()
        footer.update(format_footer_hints(hints))

    def flash_status(self, message: str) -> None:
        self._status_override = message
        self._update_footer()
        if self._flash_timer is not None:
            self._flash_timer.stop()
        if self.is_mounted and self.is_running:
            self._flash_timer = self.set_timer(2.5, self._clear_flash)

    def _clear_flash(self) -> None:
        self._flash_timer = None
        self._status_override = None
        self._update_footer()

    def _on_playback_state(self, state: PlaybackState) -> None:
        if not self.is_mounted or not self.is_running:
            return
        bars = self.query("#now-playing")
        if not bars:
            return
        bars.first(NowPlayingBar).apply_state(state)
        if self._active_view is not None and self._active_view.is_mounted:
            self._active_view.on_player_state(state)

    # ── Messages from views & widgets ───────────────────────────
    def on_top_bar_tab_selected(self, event: TopBar.TabSelected) -> None:
        self._show_root(event.tab_id)

    def on_open_artist(self, event: OpenArtist) -> None:
        self._push_view(
            ArtistView(event.browse_id, event.title, self.catalog, self.images, self.settings)
        )

    def on_open_album(self, event: OpenAlbum) -> None:
        self._push_view(
            AlbumView(event.browse_id, event.title, self.catalog, self.images, self.settings)
        )

    def on_open_playlist(self, event: OpenPlaylist) -> None:
        self._push_view(
            PlaylistView(event.browse_id, event.title, self.catalog, self.images, self.settings)
        )

    def on_open_track_album(self, event: OpenTrackAlbum) -> None:
        t = event.track
        if t.album_id:
            self._push_view(
                AlbumView(t.album_id, t.album or t.title, self.catalog, self.images, self.settings)
            )
        elif t.album:
            self.run_worker(self._lookup_and_open_album(t), exclusive=False)
        else:
            self.flash_status("Album page unavailable for this track")

    async def _lookup_and_open_album(self, track: Track) -> None:
        artist_q = "" if track.artist_names == "Unknown artist" else track.artist_names
        q = f"{track.album} {artist_q}".strip()
        try:
            items = await self.catalog.search("albums", q, limit=5)
        except CATALOG_ERRORS as exc:
            self.flash_status(f"Album lookup failed: {exc}")
            return
        albums = [x for x in items if isinstance(x, AlbumSummary)]
        target = (track.album or "").strip().lower()
        alb = next((a for a in albums if a.title.strip().lower() == target), None) or (
            albums[0] if albums else None
        )
        if alb:
            self._push_view(
                AlbumView(alb.browse_id, alb.title, self.catalog, self.images, self.settings)
            )
        else:
            self.flash_status("Album page unavailable for this track")

    def on_open_track_artist(self, event: OpenTrackArtist) -> None:
        t = event.track
        with_id = next((a for a in t.artists if a.id), None)
        if with_id and with_id.id:
            self._push_view(
                ArtistView(with_id.id, with_id.name, self.catalog, self.images, self.settings)
            )
        elif t.artists and t.artists[0].name != "Unknown artist":
            self.run_worker(self._lookup_and_open_artist(t.artists[0].name), exclusive=False)
        else:
            self.flash_status("Artist page unavailable for this track")

    async def _lookup_and_open_artist(self, name: str) -> None:
        try:
            items = await self.catalog.search("artists", name, limit=5)
        except CATALOG_ERRORS as exc:
            self.flash_status(f"Artist lookup failed: {exc}")
            return
        artists = [x for x in items if isinstance(x, ArtistSummary)]
        target = name.strip().lower()
        art = next((a for a in artists if a.name.strip().lower() == target), None) or (
            artists[0] if artists else None
        )
        if art:
            self._push_view(
                ArtistView(art.browse_id, art.name, self.catalog, self.images, self.settings)
            )
        else:
            self.flash_status("Artist page unavailable")

    def _toggle_lyrics(self) -> None:
        home = self._root_views.get("home")
        if not isinstance(home, HomeView):
            return
        if self._active_view is home and home.is_lyrics_mode:
            home.exit_lyrics_mode(focus=not self._lyrics_pushed_from_stack)
            if self._lyrics_pushed_from_stack:
                self._lyrics_pushed_from_stack = False
                self.pop_view()
            return
        pushed = self._active_view is not home
        home.enter_lyrics_mode()
        if pushed:
            self._lyrics_pushed_from_stack = True
            self._push_view(home)
        else:
            self._lyrics_pushed_from_stack = False

    def on_play_tracks(self, event: PlayTracks) -> None:
        self.engine.queue.shuffle = event.shuffle
        self.run_worker(self.engine.play_tracks(event.tracks, start=event.start), exclusive=False)

    def on_queue_tracks(self, event: QueueTracks) -> None:
        self.engine.enqueue(event.tracks)
        self.flash_status(f"Added {len(event.tracks)} track(s) to queue")

    def on_play_next_tracks(self, event: PlayNextTracks) -> None:
        if not event.tracks:
            return
        self.engine.play_next_in_queue(event.tracks)
        n = len(event.tracks)
        lbl = event.tracks[0].title if n == 1 else f"{n} tracks"
        self.flash_status(f"Playing next: {lbl}")

    def on_play_entity_direct(self, event: PlayEntityDirect) -> None:
        self.run_worker(
            self._resolve_and_play_entity(event.item, shuffle=event.shuffle), exclusive=False
        )

    def on_queue_entity_direct(self, event: QueueEntityDirect) -> None:
        self.run_worker(self._resolve_and_queue_entity(event.item), exclusive=False)

    def on_play_next_entity_direct(self, event: PlayNextEntityDirect) -> None:
        self.run_worker(self._resolve_and_queue_entity(event.item, play_next=True), exclusive=False)

    async def _fetch_entity_tracks(self, item: Any) -> tuple[Track, ...] | None:
        try:
            if isinstance(item, AlbumSummary):
                return (await self.catalog.album(item.browse_id)).tracks
            if isinstance(item, PlaylistSummary):
                return (await self.catalog.playlist(item.browse_id, limit=100)).tracks
            if isinstance(item, ArtistSummary):
                return (await self.catalog.artist(item.browse_id)).songs
        except CATALOG_ERRORS as exc:
            self.flash_status(f"Failed to load tracks: {exc}")
            return None
        return ()

    async def _resolve_and_play_entity(self, item: Any, shuffle: bool = False) -> None:
        tracks = await self._fetch_entity_tracks(item)
        if tracks is None:
            return
        if tracks:
            self.engine.queue.shuffle = shuffle
            await self.engine.play_tracks(tracks, start=0)
        else:
            self.flash_status("No playable tracks found")

    async def _resolve_and_queue_entity(self, item: Any, *, play_next: bool = False) -> None:
        tracks = await self._fetch_entity_tracks(item)
        if tracks is None:
            return
        if tracks:
            if play_next:
                self.engine.play_next_in_queue(tracks)
                self.flash_status(f"Playing next: {len(tracks)} track(s)")
            else:
                self.engine.enqueue(tracks)
                self.flash_status(f"Added {len(tracks)} track(s) to queue")
        else:
            self.flash_status("No playable tracks found")

    def on_refresh_footer(self, event: RefreshFooter) -> None:
        del event
        self._update_footer()

    def on_start_radio(self, event: StartRadio) -> None:
        self.run_worker(self._start_radio(event.video_id, event.playlist_id), exclusive=False)

    async def _start_radio(self, video_id: str | None, playlist_id: str | None) -> None:
        try:
            tracks = await self.catalog.radio(video_id=video_id, playlist_id=playlist_id, limit=25)
        except CATALOG_ERRORS as exc:
            self.flash_status(f"Radio failed: {exc}")
            return
        if tracks:
            await self.engine.play_tracks(tracks, start=0)
            self.flash_status(f"Radio started ({len(tracks)} tracks)")
        else:
            self.flash_status("No radio tracks returned")

    def on_save_bookmark(self, event: SaveBookmark) -> None:
        added = self.bookmarks.add(event.item)
        self.flash_status("Saved to Library" if added else "Already in Library")

    def on_notify_status(self, event: NotifyStatus) -> None:
        self.flash_status(event.text)

    async def _toggle_lyrics_translation(self) -> None:
        enabled = await self.engine.toggle_lyrics_translation()
        save_settings(self.settings, settings_file())
        if enabled:
            lyr = self.engine.state.lyrics
            if lyr and not lyr.has_translations and lyr.translation_lang != "id":
                self.flash_status("Indonesian translation unavailable")
            else:
                self.flash_status("Lyrics translation: ID (On)")
        else:
            self.flash_status("Lyrics translation: Off")

    def on_settings_updated(self, event: SettingsUpdated) -> None:
        del event
        self._icons = icons_for(self.settings.display.icons)
        SHIMMER_CLOCK.set_reduced_motion(self.settings.display.reduced_motion)
        if isinstance(self.catalog, YTMusicCatalog):
            self.catalog.set_region(self.settings.display.region)
        self.engine.mpv.apply_settings(self.settings)
        self.engine.refresh_audio_settings()
        self.lyrics_service.clear_cache()
        self.engine.refresh_lyrics()
        if self.settings.privacy.save_history:
            self.history.flush()
        home = self._root_views.get("home")
        if isinstance(home, HomeView):
            home.invalidate_explore()

        self.query_one("#topbar", TopBar).set_icons(self._icons)
        self.query_one("#now-playing", NowPlayingBar).set_icons(self._icons)

        self._settings_ver += 1
        for v in self._root_views.values():
            if v.is_mounted:
                v.on_settings_changed(
                    self._icons,
                    self.settings.display.cover_size,
                    self.settings.display.image_protocol,
                )
                v.mark_settings_applied(self._settings_ver)
        if (
            self._active_view
            and self._active_view not in self._root_views.values()
            and self._active_view.is_mounted
        ):
            self._active_view.on_settings_changed(
                self._icons,
                self.settings.display.cover_size,
                self.settings.display.image_protocol,
            )
            self._active_view.mark_settings_applied(self._settings_ver)

    def on_now_playing_bar_action_triggered(self, event: NowPlayingBar.ActionTriggered) -> None:
        act = event.action
        if act == "toggle_pause":
            self.run_worker(self.engine.toggle_pause())
        elif act == "next":
            self.run_worker(self.engine.next_track())
        elif act == "prev":
            self.run_worker(self.engine.prev_track())
        elif act == "toggle_shuffle":
            self.engine.toggle_shuffle()
        elif act == "cycle_repeat":
            self.engine.cycle_repeat()

    def on_seek_bar_seeked(self, event: SeekBar.Seeked) -> None:
        self.run_worker(self.engine.seek_absolute(event.seconds))

    # ── Global keyboard shortcuts ───────────────────────────────
    def on_key(self, event: Key) -> None:
        if isinstance(self.focused, Input):
            if event.key == "escape":
                if not (self._active_view and self._active_view.handle_input_escape()):
                    self.screen.focus_next()
                event.stop()
            elif event.key == "ctrl+s" and isinstance(self._active_view, SettingsView):
                self._active_view.query_one("#btn-save-settings").press()
                event.stop()
            return

        k = event.key
        if k == "q":
            self.exit()
            event.stop()
        elif k == "slash":
            if self._active_view and self._active_view.handle_slash():
                event.stop()
                return
            self._show_root("home")
            home = self._root_views.get("home")
            if isinstance(home, HomeView):
                home.activate_search()
            event.stop()
        elif k == "1":
            self._show_root("home")
            event.stop()
        elif k == "2":
            self._show_root("library")
            event.stop()
        elif k == "3":
            self._show_root("queue")
            event.stop()
        elif k == "4":
            self._show_root("settings")
            event.stop()
        elif k == "y":
            self._toggle_lyrics()
            event.stop()
        elif k == "t" and isinstance(self._active_view, HomeView):
            self.run_worker(self._toggle_lyrics_translation(), exclusive=False)
            event.stop()
        elif k == "escape":
            if isinstance(self._active_view, HomeView) and self._active_view.is_lyrics_mode:
                self._toggle_lyrics()
                event.stop()
            elif self._active_view and self._active_view.handle_escape():
                event.stop()
            elif self._nav_stack:
                self.pop_view()
                event.stop()
        elif k == "backspace":
            if isinstance(self._active_view, HomeView) and self._active_view.is_lyrics_mode:
                self._toggle_lyrics()
            else:
                self.pop_view()
            event.stop()
        elif k == "space":
            self.run_worker(self.engine.toggle_pause())
            event.stop()
        elif k == "n":
            self.run_worker(self.engine.next_track())
            event.stop()
        elif k == "b":
            self.run_worker(self.engine.prev_track())
            event.stop()
        elif k in ("plus", "equal"):
            self.run_worker(self.engine.change_volume(5))
            event.stop()
        elif k == "minus":
            self.run_worker(self.engine.change_volume(-5))
            event.stop()
        elif k in ("left", "left_square_bracket"):
            self.run_worker(self.engine.seek(-5))
            event.stop()
        elif k in ("right", "right_square_bracket"):
            self.run_worker(self.engine.seek(5))
            event.stop()
        elif k == "ctrl+s" and isinstance(self._active_view, SettingsView):
            self._active_view.query_one("#btn-save-settings").press()
            event.stop()
