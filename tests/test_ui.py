"""End-to-end Textual UI tests covering Home, secondary Hero/Split views, lyrics, & settings."""

import asyncio
from itertools import pairwise

import httpx
import pytest
from rich.cells import cell_len
from textual.color import Color
from textual.widgets import Button, Input

from musick.app import MusickApp
from musick.config import Settings
from musick.domain import ArtistRef, Thumb, Track
from musick.paths import settings_file
from musick.player import MpvController, PlayerEngine
from musick.services.images import ImageService
from musick.services.lyrics import LyricsService
from musick.services.resolver import StreamResolver
from musick.services.search import HistoryStore
from musick.ui.views import (
    AlbumView,
    ArtistView,
    HomeView,
    PlaylistView,
    QueueView,
    SettingsView,
)
from musick.ui.views.settings import SettingToggle
from musick.ui.widgets import (
    SHIMMER_CLOCK,
    CavaVisualizer,
    CoverGrid,
    CoverWidget,
    LyricsBox,
    MediaList,
    NowPlayingBar,
    SectionTabs,
    Skeleton,
    TopBar,
    TrackTable,
    cell_aspect_ratio,
    cover_dims,
    dot_frame,
    media_item_from,
    shimmer_line,
)
from tests.conftest import FakeCatalog, make_png_bytes


def _make_test_app(settings: Settings | None = None, *, with_last_track: bool = False) -> MusickApp:
    s = settings or Settings()
    cat = FakeCatalog()
    png = make_png_bytes()
    img_http = httpx.AsyncClient(
        transport=httpx.MockTransport(lambda r: httpx.Response(200, content=png))
    )
    lyr_http = httpx.AsyncClient(
        transport=httpx.MockTransport(
            lambda r: httpx.Response(
                200, json={"syncedLyrics": "[00:01.00]I said ooh\n[00:05.00]Blinded by the lights"}
            )
        )
    )
    images = ImageService(s, http=img_http)
    history = HistoryStore(s)
    if with_last_track:
        history.recent_tracks.append(
            Track(
                video_id="vid1",
                title="Blinding Lights",
                artists=(ArtistRef("The Weeknd", "UC1"),),
                album="After Hours",
                album_id="MPRE1",
                duration=200,
                thumbs=(Thumb("https://example.test/cover.png", 300, 300),),
            )
        )
    lyrics = LyricsService(s, catalog=cat, http=lyr_http)
    resolver = StreamResolver(
        runner=lambda vid: asyncio.sleep(0, result=f"https://stream.test/{vid}")
    )

    class NoopMpv(MpvController):
        async def start(self) -> None:
            self._started = True

        async def stop(self) -> None:
            self._started = False

        async def command(self, cmd, wait=True, timeout=2.0):
            return None

    mpv = NoopMpv(s, spawn_process=False)
    engine = PlayerEngine(s, resolver, lyrics, history, mpv=mpv)
    return MusickApp(settings=s, catalog=cat, images=images, history=history, engine=engine)


def test_loading_dot_and_shimmer_primitives() -> None:
    assert dot_frame(0) == "●··"
    assert dot_frame(2) == "·●·"
    assert dot_frame(4) == "··●"
    assert dot_frame(0, reduced_motion=True) == "●●●"

    line = shimmer_line("xxxx  xxxx", row=0, tick=1)
    assert len(line.plain) == 10
    assert line.plain[4:6] == "  "


@pytest.mark.asyncio
async def test_ui_full_navigation_covers_lyrics_and_settings() -> None:
    app = _make_test_app(with_last_track=True)
    async with app.run_test(size=(100, 42)) as pilot:
        await pilot.pause(0.15)

        # 1. Transparent bg + Home Player showing last played & no mini cover in NowPlayingBar
        assert app.screen.styles.background == Color(0, 0, 0, a=0)
        assert isinstance(app._active_view, HomeView)
        assert app.query_one("#topbar", TopBar).active_tab == "home"
        assert app.query_one("#home-cover-panel").border_title == "Last Played"
        assert app.engine.state.track is not None
        assert app.engine.state.track.title == "Blinding Lights"
        assert len(app.query_one("#now-playing", NowPlayingBar).query("#np-cover")) == 0
        assert isinstance(app.query_one("#home-cava"), CavaVisualizer)
        assert isinstance(app.query_one("#home-lyrics-box"), LyricsBox)

        # Pressing space on Home starts playing the restored last-played track
        await pilot.press("space")
        await pilot.pause(0.15)
        assert app.engine.state.playing is True
        assert app.engine.state.lyrics.synced is True
        assert app.query_one("#home-cover-panel").border_title == "Now Playing"

        # 2. Inline Search in Panel 2: '/' opens prompt + autosuggest, arrows select, Enter runs
        await pilot.press("slash")
        await pilot.pause(0.05)
        assert isinstance(app._active_view, HomeView)
        hv = app._active_view
        assert hv._mode == "suggest"
        inp = app.query_one("#home-p2-input", Input)
        assert inp.has_focus

        # Typing 'q' in prompt types 'q' instead of quitting!
        inp.insert_text_at_cursor("akon")
        await pilot.pause(0.25)
        sugg_tbl = app.query_one("#home-p2-sugg-tbl", TrackTable)
        assert len(sugg_tbl.rows) == 2
        assert sugg_tbl.cursor == -1

        await pilot.press("down")
        await pilot.pause(0.05)
        assert sugg_tbl.cursor == 0

        await pilot.press("enter")
        await pilot.pause(0.2)
        assert hv._mode == "results"
        assert inp.value == "akon official"
        res_list = app.query_one("#home-p2-list", MediaList)
        assert len(res_list.items) == 4
        # Panel 1 keeps previewing the currently playing cover, not the cursor item
        assert app.query_one("#home-cover-panel").border_title == "Now Playing"

        # 3. Cursor on Track -> 'a' opens AlbumView, Backspace returns, 'A' opens ArtistView
        await pilot.press("down")
        await pilot.pause(0.05)
        assert isinstance(res_list.current_payload, Track)

        await pilot.press("a")
        await pilot.pause(0.15)
        assert isinstance(app._active_view, AlbumView)
        assert len(app._active_view.query_one("#alb-tbl", TrackTable).rows) == 3

        await pilot.press("backspace")
        await pilot.pause(0.1)
        assert isinstance(app._active_view, HomeView)
        assert app.focused is res_list

        await pilot.press("A")
        await pilot.pause(0.15)
        assert isinstance(app._active_view, ArtistView)
        av = app._active_view
        assert len(av.query_one("#art-songs-tbl", TrackTable).rows) == 3

        # Switch to Albums tab & test S/M/L cover sizes (Gambar 2)
        av.switch_tab("albums")
        await pilot.pause(0.15)
        grid = av.query_one("#art-rel-grid", CoverGrid)
        assert grid.cover_size == "M"
        grid.set_cover_size("L")
        await pilot.pause(0.05)
        assert grid.cover_size == "L"
        grid.set_cover_size("S")
        await pilot.pause(0.05)
        assert grid.cover_size == "S"

        # Open AlbumView from Artist Albums grid and play track
        await pilot.press("enter")
        await pilot.pause(0.15)
        assert isinstance(app._active_view, AlbumView)
        await pilot.press("enter")
        await pilot.pause(0.15)
        assert app.engine.state.track is not None
        assert app.engine.state.track.title == "Blinding Lights"

        # 4. Open Home lyrics mode via 'y' from AlbumView, then toggle 'y' back to AlbumView
        await pilot.press("y")
        await pilot.pause(0.1)
        assert isinstance(app._active_view, HomeView)
        assert app._active_view.is_lyrics_mode is True
        assert app.query_one("#home-main-panel").border_title == "Synced Lyrics"
        assert app.query_one("#home-lyrics-panel").border_title == "Details"
        assert app.query_one("#home-cava").display is True
        await pilot.press("y")
        await pilot.pause(0.1)
        assert isinstance(app._active_view, AlbumView)

        # 5. Home search with prefix -> open Community Playlist -> '/' filter & Esc blur & load_more
        await pilot.press("1")
        await pilot.pause(0.1)
        assert isinstance(app._active_view, HomeView)
        sv = app._active_view
        sv.set_query("pl: chill", run=True)
        await pilot.pause(0.15)
        assert sv._category == "playlists"
        assert len(sv.query_one("#home-p2-list", MediaList).items) == 1

        await pilot.press("enter")
        await pilot.pause(0.15)
        assert isinstance(app._active_view, PlaylistView)
        pv = app._active_view
        assert len(pv.query_one("#pl-tbl", TrackTable).rows) == 3

        await pilot.press("slash")
        await pilot.pause(0.05)
        assert app.focused is pv.query_one("#pl-filter-input", Input)
        await pilot.press("escape")
        await pilot.pause(0.05)
        assert app.focused is pv.query_one("#pl-tbl", TrackTable)

        pv.load_more()
        await pilot.pause(0.15)
        assert len(pv.query_one("#pl-tbl", TrackTable).rows) == 4

        # 6. Return to Home and test Esc closing search results back to Queue
        await pilot.press("1")
        await pilot.pause(0.1)
        assert isinstance(app._active_view, HomeView)
        await pilot.press("escape")
        await pilot.pause(0.05)
        assert app._active_view._mode == "queue"
        assert app.query_one("#home-cover-panel").border_title == "Now Playing"

        # 7. SettingsView: toggle history, reduced motion, region, verify persistence, clear data
        await pilot.press("4")
        await pilot.pause(0.1)
        assert isinstance(app._active_view, SettingsView)
        app._active_view.query_one("#tog-history", SettingToggle).press()
        app._active_view.query_one("#tog-reduced-motion", SettingToggle).press()
        app._active_view.query_one("#reg-US", Button).press()
        await pilot.pause(0.1)
        assert app.settings.privacy.save_history is True
        assert app.settings.display.reduced_motion is True
        assert app.settings.display.region == "US"
        assert SHIMMER_CLOCK.reduced_motion is True
        assert settings_file().exists()

        sw, sh = cover_dims("S")
        mw, mh = cover_dims("M")
        lw, lh = cover_dims("L")
        assert sw / sh >= 2.0
        assert mw / mh >= 2.0
        assert lw / lh >= 2.0

        app._active_view.query_one("#btn-clear-data", Button).press()
        await pilot.pause(0.05)
        assert app.history.recent_tracks == []


@pytest.mark.asyncio
async def test_home_prompt_escape_stale_cursor_and_quit() -> None:
    app = _make_test_app()
    async with app.run_test(size=(90, 30)) as pilot:
        await pilot.pause(0.1)
        hv = app._active_view
        assert isinstance(hv, HomeView)
        assert hv._mode == "queue"

        # Open prompt, type, highlight suggestion, then type more -> stale cursor resets immediately
        await pilot.press("slash")
        await pilot.pause(0.05)
        inp = app.query_one("#home-p2-input", Input)
        inp.insert_text_at_cursor("akon")
        await pilot.pause(0.22)
        sugg_tbl = app.query_one("#home-p2-sugg-tbl", TrackTable)
        await pilot.press("down")
        assert sugg_tbl.cursor == 0

        # Typing 'q' inside prompt must NOT quit app and must reset stale suggestion cursor!
        await pilot.press("q")
        await pilot.pause(0.02)
        assert app.is_running
        assert sugg_tbl.cursor == -1
        assert inp.value == "akonq"

        # Pressing Esc inside prompt (with no results yet) exits prompt back to queue mode
        await pilot.press("escape")
        await pilot.pause(0.05)
        assert hv._mode == "queue"
        assert not inp.has_focus

        # Pressing 'q' outside prompt quits cleanly
        await pilot.press("q")
        await pilot.pause(0.05)
        assert not app.is_running


@pytest.mark.asyncio
async def test_track_album_and_artist_lookup_fallback() -> None:
    app = _make_test_app()
    async with app.run_test(size=(100, 36)) as pilot:
        await pilot.pause(0.1)
        hv = app._active_view
        assert isinstance(hv, HomeView)

        # Inject a track missing both album_id and artist.id to verify catalog fallback lookup
        hv._mode = "results"
        hv._apply_mode_ui()
        no_ids = Track(
            video_id="v_noid",
            title="Mystery Song",
            artists=(ArtistRef("The Weeknd", None),),
            album="After Hours",
            album_id=None,
            duration=180,
        )
        hv._results = (no_ids,)
        ml = app.query_one("#home-p2-list", MediaList)
        from musick.ui.widgets import media_item_from

        ml.set_items([media_item_from(no_ids)], reset_cursor=True)
        ml.focus()

        await pilot.press("a")
        await pilot.pause(0.15)
        assert isinstance(app._active_view, AlbumView)

        await pilot.press("backspace")
        await pilot.pause(0.1)
        await pilot.press("A")
        await pilot.pause(0.15)
        assert isinstance(app._active_view, ArtistView)


@pytest.mark.asyncio
async def test_queue_library_and_search_view_actions() -> None:
    app = _make_test_app(with_last_track=True)
    async with app.run_test(size=(100, 36)) as pilot:
        await pilot.pause(0.15)

        # Save an album to Library via Home search
        assert isinstance(app._active_view, HomeView)
        app._active_view.set_query("al: after", run=True)
        await pilot.pause(0.15)
        await pilot.press("w")
        await pilot.pause(0.05)

        # LibraryView shows saved bookmark and supports 'd' removal
        await pilot.press("2")
        await pilot.pause(0.1)
        saved_tbl = app.query_one("#lib-saved-tbl", TrackTable)
        assert len(saved_tbl.rows) == 1
        saved_tbl.focus()
        await pilot.press("d")
        await pilot.pause(0.05)
        assert len(saved_tbl.rows) == 0

        # QueueView shows restored track and supports 'a' -> AlbumView & 'd' remove
        await pilot.press("3")
        await pilot.pause(0.1)
        q_tbl = app.query_one("#q-tbl", TrackTable)
        assert len(q_tbl.rows) == 1
        q_tbl.focus()
        await pilot.press("a")
        await pilot.pause(0.15)
        assert isinstance(app._active_view, AlbumView)


@pytest.mark.asyncio
async def test_startup_focus_player_bar_layout_and_button_bounds() -> None:
    app = _make_test_app(with_last_track=True)
    async with app.run_test(size=(80, 24)) as pilot:
        await pilot.pause(0.15)

        # 1. Startup focus lands on Home Queue MediaList (not outer HomeView)
        res_list = app.query_one("#home-p2-list", MediaList)
        assert app.focused is res_list

        # 1b. Nav divider spans the full frame width and meets the outer border
        topbar = app.query_one("#topbar", TopBar)
        frame = app.query_one("#app-frame")
        assert topbar.region.x == frame.content_region.x
        assert topbar.region.right == frame.content_region.right
        divider = topbar.render_line(1).text
        assert cell_len(divider) == topbar.region.width
        assert divider[0] == "─"
        assert divider[-1] == "─"

        # 2. NowPlayingBar volume meter is right-aligned & play/pause icon resizes cleanly
        np_bar = app.query_one("#now-playing", NowPlayingBar)
        np_rpt = app.query_one("#np-rpt", Button)
        np_vol = app.query_one("#np-vol")
        assert np_vol.region.x > np_rpt.region.right + 10
        assert np_vol.region.right <= np_bar.content_region.right

        await pilot.press("space")
        await pilot.pause(0.15)
        np_play = app.query_one("#np-play", Button)
        cluster = [
            app.query_one(f"#np-{bid}", Button)
            for bid in ("prev", "play", "next", "shf", "rpt")
        ]
        assert {btn.region.width for btn in cluster} == {4}
        assert [b.region.x - a.region.x for a, b in pairwise(cluster)] == [5, 5, 5, 5]
        assert "▌▌" in np_play.render_line(0).text

        # 3. Search -> open AlbumView & ArtistView -> playing track highlighted & buttons fit
        await pilot.press("slash")
        await pilot.pause(0.05)
        inp = app.query_one("#home-p2-input", Input)
        inp.insert_text_at_cursor("akon")
        await pilot.pause(0.22)
        await pilot.press("down")
        await pilot.press("enter")
        await pilot.pause(0.2)
        await pilot.press("down")
        await pilot.press("a")
        await pilot.pause(0.15)

        av_alb = app._active_view
        assert isinstance(av_alb, AlbumView)
        assert av_alb.query_one("#alb-tbl", TrackTable).playing_index == 0
        alb_panel = av_alb.query_one("#alb-info-panel")
        for bid in ("#alb-play", "#alb-shf", "#alb-queue", "#alb-save", "#alb-artist"):
            btn = av_alb.query_one(bid, Button)
            assert btn.region.right <= alb_panel.content_region.right
        # Hidden skeletons must unsubscribe from SHIMMER_CLOCK
        assert av_alb.query_one("#alb-head-skel", Skeleton)._unsub is None

        await pilot.press("A")
        await pilot.pause(0.15)
        av_art = app._active_view
        assert isinstance(av_art, ArtistView)
        assert av_art.query_one("#art-songs-tbl", TrackTable).playing_index == 0
        art_panel = av_art.query_one("#art-info-panel")
        for bid in ("#art-play", "#art-radio", "#art-shf", "#art-queue", "#art-save"):
            btn = av_art.query_one(bid, Button)
            assert btn.region.right <= art_panel.content_region.right

        # 4. Home search uses SectionTabs without clipping at 80x24
        await pilot.press("1")
        await pilot.pause(0.1)
        sv = app._active_view
        assert isinstance(sv, HomeView)
        tabs = sv.query_one("#home-p2-cat-tabs", SectionTabs)
        assert tabs.region.width <= sv.query_one("#home-main-panel").content_region.width


@pytest.mark.asyncio
async def test_home_tab_completion_esc_from_player_bar_and_settings_fit() -> None:
    app = _make_test_app(with_last_track=True)
    async with app.run_test(size=(100, 36)) as pilot:
        await pilot.pause(0.15)

        # Tab inside prompt completes suggestion instead of leaking focus to NowPlayingBar
        await pilot.press("slash")
        await pilot.pause(0.05)
        inp = app.query_one("#home-p2-input", Input)
        inp.insert_text_at_cursor("akon")
        await pilot.pause(0.22)
        await pilot.press("tab")
        await pilot.pause(0.05)
        assert inp.has_focus
        assert inp.value == "akon official"

        # Submit search, move focus onto NowPlayingBar, press Esc -> exits search back to queue
        await pilot.press("enter")
        await pilot.pause(0.2)
        assert app._active_view._mode == "results"
        app.query_one("#np-play", Button).focus()
        await pilot.pause(0.02)
        await pilot.press("escape")
        await pilot.pause(0.05)
        assert app._active_view._mode == "queue"

        # SettingsView at 100x36 fits Save Settings button inside viewport without clipping
        await pilot.press("4")
        await pilot.pause(0.1)
        sv = app._active_view
        assert isinstance(sv, SettingsView)
        save_btn = sv.query_one("#btn-save-settings", Button)
        assert save_btn.region.bottom <= sv.region.bottom


@pytest.mark.asyncio
async def test_wide_left_panel_cover_fit_symmetric_gaps_and_scrollbar_gutter() -> None:
    app = _make_test_app(with_last_track=True)
    async with app.run_test(size=(170, 40)) as pilot:
        await pilot.pause(0.15)

        # 1. HomeView widens left column beyond old 36-col cap, separates left/right by 1 col,
        #    and fits 1:1 cover flush to panel borders with zero dead vertical slack.
        left_col = app.query_one("#home-cover-panel")
        right_col = app.query_one("#home-main-panel")
        cov_panel = app.query_one("#home-cover-panel")
        main_panel = app.query_one("#home-main-panel")
        cov = app.query_one("#home-cov", CoverWidget)

        assert left_col.region.width > 36
        assert right_col.region.x == left_col.region.right + 1
        assert cov_panel.region.bottom == main_panel.region.bottom
        assert left_col.region.width == cov.region.width + 4
        ratio = cov.region.width / (cov.region.height * cell_aspect_ratio())
        assert 0.90 <= ratio <= 1.10
        home_regions = tuple(
            app.query_one(pid).region
            for pid in (
                "#home-cover-panel",
                "#home-main-panel",
                "#home-lyrics-panel",
                "#home-cava-panel",
            )
        )

        # Scrollbar gutter in Home results leaves >= 1 blank col after time text
        hv = app._active_view
        assert isinstance(hv, HomeView)
        many = [
            Track(
                f"v{i}",
                f"Be With You {i}",
                (ArtistRef("Atomic Kitten", "UC1"),),
                "Album",
                "A1",
                350,
            )
            for i in range(16)
        ]
        hv._mode = "results"
        hv._results = tuple(many)
        hv._apply_mode_ui()
        ml = hv.query_one("#home-p2-list", MediaList)
        ml.set_items([media_item_from(t) for t in many], reset_cursor=True)
        await pilot.pause(0.1)
        strips = app.screen._compositor.render_strips(app.screen.size)
        rendered_rows = ["".join(seg.text for seg in st) for st in strips]
        assert any("5:50 " in line for line in rendered_rows)
        assert not any(
            "5:50▅" in line or "5:50▆" in line or "5:50▇" in line or "5:50█" in line
            for line in rendered_rows
        )

        # 2. ArtistView uses horizontal Hero header above full-width catalog,
        #    while lyrics:y preserves the exact 4-panel Home geometry.
        from musick.ui.messages import OpenArtist

        app.post_message(OpenArtist("UC1", "The Weeknd"))
        for _ in range(20):
            await pilot.pause(0.05)
            if (
                isinstance(app._active_view, ArtistView)
                and len(app._active_view.query_one("#art-songs-tbl", TrackTable).rows) > 0
            ):
                break
        assert isinstance(app._active_view, ArtistView)
        art_head = app.query_one("#art-head-panel")
        art_info = app.query_one("#art-info-panel")
        art_body = app.query_one("#art-body-panel")
        art_cov = app.query_one("#art-cov", CoverWidget)
        assert art_info.region.x == art_head.region.right + 1
        assert art_head.region.bottom == art_info.region.bottom
        assert art_head.region.width == art_cov.region.width + 4
        assert art_body.region.y >= art_head.region.bottom
        assert art_body.region.width > art_info.region.width
        assert app.query_one("#art-ov-rel-grid", CoverGrid).display is True
        assert len(app.query_one("#art-ov-sim-tbl", TrackTable).rows) == 1

        # Pressing 'y' keeps identical 4-panel Home geometry; Esc returns to ArtistView
        await pilot.press("y")
        await pilot.pause(0.1)
        assert isinstance(app._active_view, HomeView)
        assert app._active_view.is_lyrics_mode is True
        assert (
            tuple(
                app.query_one(pid).region
                for pid in (
                    "#home-cover-panel",
                    "#home-main-panel",
                    "#home-lyrics-panel",
                    "#home-cava-panel",
                )
            )
            == home_regions
        )
        await pilot.press("escape")
        await pilot.pause(0.1)
        assert isinstance(app._active_view, ArtistView)
        await pilot.press("backspace")
        await pilot.pause(0.1)
        assert isinstance(app._active_view, HomeView)
        assert app._active_view._mode == "results"

        # 3. Settings split columns share 1-col horizontal middle gap
        await pilot.press("4")
        await pilot.pause(0.1)
        assert isinstance(app._active_view, SettingsView)
        set_left = app.query_one(".split-col-left")
        set_right = app.query_one(".split-col-right")
        assert set_right.region.x == set_left.region.right + 1


@pytest.mark.asyncio
async def test_tgp_medialist_full_highlight_explore_artist_top10_and_queue_reorder(
    monkeypatch,
) -> None:
    from textual_image import _terminal
    from textual_image.renderable import tgp as tgp_mod

    from musick.domain import Artist
    from musick.ui.messages import OpenArtist

    prev_probe = getattr(_terminal.probe_terminal, "_result", None)
    _terminal.probe_terminal._result = _terminal.TerminalCapabilities(  # type: ignore[attr-defined]
        _terminal.CellSize(10, 20), sixel=False, tgp=True
    )
    monkeypatch.setattr(tgp_mod, "_send_tgp_message", lambda **kw: None)

    try:
        app = _make_test_app(with_last_track=True)
        # Expand FakeCatalog artist with 15 songs + 1 video for Top 10 / All & Videos tests
        cat = app.catalog
        assert isinstance(cat, FakeCatalog)
        fifteen = tuple(
            Track(
                f"top{i}",
                f"Hit Song {i}",
                (ArtistRef("The Weeknd", "UC1"),),
                "After Hours",
                "MPRE1",
                200,
                cat.tracks[0].thumbs,
                f"{20 - i}M",
            )
            for i in range(1, 16)
        )
        cat.artist_obj = Artist(
            summary=cat.artist_obj.summary,
            description=cat.artist_obj.description,
            listeners=cat.artist_obj.listeners,
            songs=fifteen,
            albums=cat.artist_obj.albums,
            singles=cat.artist_obj.singles,
            playlists=cat.artist_obj.playlists,
            related=cat.artist_obj.related,
            albums_params=cat.artist_obj.albums_params,
            singles_params=cat.artist_obj.singles_params,
            radio_id=cat.artist_obj.radio_id,
            videos=(
                Track(
                    "mv1",
                    "Blinding Lights (MV)",
                    (ArtistRef("The Weeknd", "UC1"),),
                    duration=260,
                    plays="800M",
                ),
            ),
        )

        async with app.run_test(size=(120, 36)) as pilot:
            await pilot.pause(0.15)
            hv = app._active_view
            assert isinstance(hv, HomeView)

            # Redundant cover caption widgets must be removed
            assert len(hv.query("#home-cov-title")) == 0
            assert len(hv.query("#home-cov-sub")) == 0

            # 'e' opens Explore (New Releases / Artists / Charts) in Panel 2, 'Esc' returns to Queue
            await pilot.press("e")
            await pilot.pause(0.15)
            assert hv._mode == "explore"
            ml = hv.query_one("#home-p2-list", MediaList)
            assert len(ml.items) == 5

            # Full-width TGP cursor highlight reaches entire MediaList width (even empty top_right)
            from textual.geometry import Region

            ml.cursor = 2
            ml.focus()
            await pilot.pause(0.1)
            expected_body_w = (
                ml.size.width
                - (1 if ml.parent and getattr(ml.parent, "show_vertical_scrollbar", False) else 0)
                - 5
            )
            strips = ml.render_lines(Region(0, 0, ml.size.width, ml.size.height))
            base_row = ml.cursor * ml.row_h
            for r_off in (0, 1):
                cells = [
                    (seg.cell_length, str(seg.style.bgcolor) if seg.style else "")
                    for seg in strips[base_row + r_off]
                ]
                cursor_bg_cells = sum(cl for cl, bg in cells if "#1f4f46" in bg.lower())
                assert cursor_bg_cells == expected_body_w

            await pilot.press("escape")
            await pilot.pause(0.05)
            assert hv._mode == "queue"

            # ArtistView defaults to Top 10 Songs (>10 songs), 't' toggles All, 'Videos' works
            app.post_message(OpenArtist("UC1", "The Weeknd"))
            for _ in range(30):
                await pilot.pause(0.05)
                if (
                    isinstance(app._active_view, ArtistView)
                    and len(app._active_view.query_one("#art-songs-tbl", TrackTable).rows) == 10
                ):
                    break
            av = app._active_view
            assert isinstance(av, ArtistView)
            songs_tbl = av.query_one("#art-songs-tbl", TrackTable)
            assert len(songs_tbl.rows) == 10
            assert "Top 10" in str(av.query_one("#art-body-panel").border_title)

            await pilot.press("t")
            await pilot.pause(0.05)
            assert len(songs_tbl.rows) == 15

            av.switch_tab("videos")
            await pilot.pause(0.05)
            assert len(songs_tbl.rows) == 1

            # HomeView queue J/K reorder & results h/l category tab switch
            await pilot.press("1")
            await pilot.pause(0.08)
            app.engine.queue.set_tracks(fifteen[:3], start=0)
            hv.refresh_queue()
            hm_list = hv.query_one("#home-p2-list", MediaList)
            hm_list.focus()
            hm_list.cursor = 0
            await pilot.press("J")
            await pilot.pause(0.05)
            assert [t.video_id for t in app.engine.queue.tracks] == ["top2", "top1", "top3"]

            hv.set_query("after", run=True)
            for _ in range(20):
                await pilot.pause(0.03)
                if hv._mode == "results" and hv._results:
                    break
            assert hv._category == "all"
            await pilot.press("l")
            await pilot.pause(0.08)
            assert hv._category == "songs"

            # QueueView supports J/K reordering
            app.engine.queue.set_tracks(fifteen[:3], start=0)
            await pilot.press("3")
            await pilot.pause(0.1)
            q_tbl = app.query_one("#q-tbl", TrackTable)
            q_tbl.focus()
            assert [r[1].video_id for r in q_tbl.rows] == ["top1", "top2", "top3"]
            await pilot.press("J")
            await pilot.pause(0.05)
            assert [r[1].video_id for r in q_tbl.rows] == ["top2", "top1", "top3"]
            assert q_tbl.cursor == 1
            assert q_tbl.playing_index == 1

            # SettingsView lyrics source selector persists
            await pilot.press("4")
            await pilot.pause(0.1)
            app._active_view.query_one("#lyrsrc-youtube", Button).press()
            await pilot.pause(0.05)
            assert app.settings.lyrics.source == "youtube"
    finally:
        if prev_probe is not None:
            _terminal.probe_terminal._result = prev_probe  # type: ignore[attr-defined]


@pytest.mark.asyncio
async def test_lyrics_translation_toggle_t_home_and_lyrics_view() -> None:
    s = Settings()
    cat = FakeCatalog()
    png = make_png_bytes()
    img_http = httpx.AsyncClient(
        transport=httpx.MockTransport(lambda r: httpx.Response(200, content=png))
    )

    def lyr_handler(r: httpx.Request) -> httpx.Response:
        if "translate.googleapis.com" in str(r.url):
            q = r.url.params.get("q", "")
            tr = "\n".join(
                {"I said ooh": "Aku bilang ooh", "Blinded by the lights": "Silau oleh cahaya"}.get(
                    ln, ln
                )
                for ln in q.splitlines()
            )
            return httpx.Response(200, json=[[[tr, q]], None, "en"])
        return httpx.Response(
            200,
            json={"syncedLyrics": "[00:01.00]I said ooh\n[00:05.00]Blinded by the lights"},
        )

    lyr_http = httpx.AsyncClient(transport=httpx.MockTransport(lyr_handler))
    images = ImageService(s, http=img_http)
    history = HistoryStore(s)
    track = Track("vid1", "Blinding Lights", (ArtistRef("The Weeknd", "UC1"),), duration=200)
    history.recent_tracks.append(track)
    lyrics = LyricsService(s, catalog=cat, http=lyr_http)
    resolver = StreamResolver(
        runner=lambda vid: asyncio.sleep(0, result=f"https://stream.test/{vid}")
    )

    class NoopMpv(MpvController):
        async def start(self) -> None:
            self._started = True

        async def stop(self) -> None:
            self._started = False

        async def command(self, cmd, wait=True, timeout=2.0):
            return None

    mpv = NoopMpv(s, spawn_process=False)
    engine = PlayerEngine(s, resolver, lyrics, history, mpv=mpv)
    app = MusickApp(settings=s, catalog=cat, images=images, history=history, engine=engine)

    async with app.run_test(size=(120, 36)) as pilot:
        await pilot.pause(0.15)
        box = app.query_one("#home-lyrics-box", LyricsBox)
        assert "Aku bilang ooh" not in box.render().plain
        assert "· ID" not in str(app.query_one("#home-lyrics-panel").border_subtitle)
        panel_ids = (
            "#home-cover-panel",
            "#home-main-panel",
            "#home-lyrics-panel",
            "#home-cava-panel",
        )
        home_regions = tuple(app.query_one(pid).region for pid in panel_ids)

        # Pressing 't' enables Indonesian translation without hiding original English lyrics
        await pilot.press("t")
        await pilot.pause(0.15)
        assert app.settings.lyrics.translate_to_id is True
        home_plain = box.render().plain
        assert "▸" not in home_plain
        assert "I said ooh" in home_plain
        assert "Aku bilang ooh" in home_plain
        assert "Silau oleh cahaya" in home_plain
        assert "· ID" in str(app.query_one("#home-lyrics-panel").border_subtitle)

        # Pressing 'y' keeps identical 4-panel Home geometry, swapping to Synced Lyrics + Details
        await pilot.press("y")
        await pilot.pause(0.1)
        assert isinstance(app._active_view, HomeView)
        assert app._active_view.is_lyrics_mode is True
        assert tuple(app.query_one(pid).region for pid in panel_ids) == home_regions
        assert app.query_one("#home-main-panel").border_title == "Synced Lyrics"
        assert app.query_one("#home-lyrics-panel").border_title == "Details"
        assert "Blinding Lights" in app.query_one("#home-details-title").render().plain

        main_lyr = app.query_one("#home-main-lyrics", LyricsBox)
        assert app.focused is main_lyr
        full_render = main_lyr.render()
        full_lines = full_render.plain.splitlines()
        assert "▸" not in full_render.plain
        assert any("I said ooh" in ln for ln in full_lines)
        assert any("Aku bilang ooh" in ln for ln in full_lines)
        assert "" in full_lines  # blank gap between bilingual stanzas
        from musick.ui.theme import CURSOR_BG

        assert not any(CURSOR_BG in str(sp.style) for sp in full_render.spans)
        # First synced stanza is vertically centered with empty padding above
        act_row = next(i for i, ln in enumerate(full_lines) if "I said ooh" in ln)
        assert act_row == max(0, (main_lyr.size.height - 2) // 2)

        # Pressing 't' inside Home lyrics:y mode toggles translation off immediately
        await pilot.press("t")
        await pilot.pause(0.08)
        assert app.settings.lyrics.translate_to_id is False
        off_plain = main_lyr.render().plain
        assert "Aku bilang ooh" not in off_plain
        assert "I said ooh" in off_plain
        assert "Synced LRC" in app.query_one("#home-details-meta").render().plain
        assert "Bilingual ID" not in str(app.query_one("#home-main-panel").border_subtitle)

        # Pressing 'y' again restores normal Home Queue + compact Lyrics panels
        await pilot.press("y")
        await pilot.pause(0.08)
        assert app._active_view.is_lyrics_mode is False
        assert app.query_one("#home-main-panel").border_title == "Queue"
        assert app.query_one("#home-lyrics-panel").border_title == "Lyrics"
        assert app.focused is app.query_one("#home-p2-list", MediaList)


@pytest.mark.asyncio
async def test_hero_album_playlist_view_modes_and_mini_icons() -> None:
    from musick.ui.messages import OpenAlbum, OpenPlaylist

    app = _make_test_app(with_last_track=True)
    async with app.run_test(size=(120, 36)) as pilot:
        await pilot.pause(0.15)

        # 1. AlbumView uses Hero Showcase header above full-width tracklist with mini song icons
        app.post_message(OpenAlbum("MPRE1", "After Hours"))
        for _ in range(20):
            await pilot.pause(0.05)
            if (
                isinstance(app._active_view, AlbumView)
                and len(app._active_view.query_one("#alb-tbl", TrackTable).rows) == 3
            ):
                break
        av = app._active_view
        assert isinstance(av, AlbumView)
        alb_head = av.query_one("#alb-head-panel")
        alb_info = av.query_one("#alb-info-panel")
        alb_tracks = av.query_one("#alb-tracks-panel")
        assert alb_info.region.x == alb_head.region.right + 1
        assert alb_tracks.region.y >= alb_head.region.bottom
        assert alb_tracks.region.width > alb_info.region.width
        assert "♪" in av.query_one("#alb-tbl", TrackTable).render().plain

        # 2. PlaylistView Curator Hero + 'v' toggling Table <-> Mini-Cover MediaList + filter
        app.post_message(OpenPlaylist("PL_COMM", "Lofi Beats to Study"))
        for _ in range(20):
            await pilot.pause(0.05)
            if (
                isinstance(app._active_view, PlaylistView)
                and len(app._active_view.query_one("#pl-tbl", TrackTable).rows) == 3
            ):
                break
        pv = app._active_view
        assert isinstance(pv, PlaylistView)
        pl_tbl = pv.query_one("#pl-tbl", TrackTable)
        pl_media = pv.query_one("#pl-media", MediaList)
        assert pl_tbl.display is True
        assert pl_media.display is False

        await pilot.press("v")
        await pilot.pause(0.08)
        assert pl_tbl.display is False
        assert pl_media.display is True
        assert app.focused is pl_media
        assert len(pl_media.items) == 3
        assert "♪" in pl_media.render().plain

        # Filter in covers mode narrows visible items and Esc returns focus to MediaList
        await pilot.press("slash")
        await pilot.pause(0.04)
        inp = pv.query_one("#pl-filter-input", Input)
        inp.insert_text_at_cursor("Starboy")
        await pilot.pause(0.06)
        assert len(pl_media.items) == 1
        await pilot.press("escape")
        await pilot.pause(0.04)
        assert app.focused is pl_media

        # 3. Switching Settings to Nerd Icons updates mini icons dynamically across views
        await pilot.press("4")
        await pilot.pause(0.08)
        app._active_view.query_one("#icon-nerd", Button).press()
        await pilot.pause(0.05)
        assert app.settings.display.icons == "nerd"

        await pilot.press("1")
        await pilot.pause(0.08)
        assert "\uf001" in app.query_one("#home-p2-list", MediaList).render().plain

        await pilot.press("3")
        await pilot.pause(0.08)
        assert "\uf001" in app.query_one("#q-tbl", TrackTable).render().plain


@pytest.mark.asyncio
async def test_flash_status_teardown_and_rapid_override() -> None:
    app = _make_test_app()
    async with app.run_test(size=(100, 30)) as pilot:
        await pilot.pause(0.08)
        app.flash_status("First message")
        first_timer = app._flash_timer
        assert first_timer is not None
        app.flash_status("Second message")
        assert app._flash_timer is not None
        assert app._flash_timer is not first_timer
        assert app._status_override == "Second message"
        # Simulate flash firing after screen teardown / unmounted state without crashing
        app._clear_flash()
        assert app._status_override is None
        app.flash_status("Pending on exit")
    assert app._flash_timer is None


@pytest.mark.asyncio
async def test_cover_grid_labels_overview_navigation_and_queue_entity() -> None:
    from musick.domain import Artist
    from musick.ui.messages import OpenArtist

    app = _make_test_app()
    cat = app.catalog
    assert isinstance(cat, FakeCatalog)
    cat.artist_obj = Artist(
        summary=cat.artist_obj.summary,
        description=cat.artist_obj.description,
        listeners=cat.artist_obj.listeners,
        songs=cat.artist_obj.songs,
        albums=cat.artist_obj.albums,
        singles=cat.artist_obj.singles,
        playlists=cat.artist_obj.playlists,
        related=cat.artist_obj.related,
        albums_params=cat.artist_obj.albums_params,
        singles_params=cat.artist_obj.singles_params,
        radio_id=cat.artist_obj.radio_id,
        videos=(
            Track(
                "mv1",
                "Blinding Lights (MV)",
                (ArtistRef("The Weeknd", "UC1"),),
                duration=260,
                plays="800M",
            ),
        ),
    )

    async with app.run_test(size=(120, 36)) as pilot:
        await pilot.pause(0.1)
        app.post_message(OpenArtist("UC1", "The Weeknd"))
        for _ in range(30):
            await pilot.pause(0.05)
            g = app.query("#art-ov-rel-grid")
            if g and g.first(CoverGrid).children and g.first(CoverGrid).children[0].is_mounted:
                break
        await pilot.pause(0.05)

        av = app._active_view
        assert isinstance(av, ArtistView)
        ov_grid = av.query_one("#art-ov-rel-grid", CoverGrid)
        assert ov_grid.size.height >= 6
        titles = [w.render().plain.strip() for w in ov_grid.query(".card-title")]
        subs = [w.render().plain.strip() for w in ov_grid.query(".card-sub")]
        assert any("Starboy" in t or "After" in t or "Take" in t for t in titles)
        assert any("202" in s or "201" in s for s in subs)

        # Keyboard 'down' past bottom of Top Songs steps into Overview CoverGrid & Similar table
        songs_tbl = av.query_one("#art-songs-tbl", TrackTable)
        sim_tbl = av.query_one("#art-ov-sim-tbl", TrackTable)
        assert app.focused is songs_tbl
        for _ in range(len(songs_tbl.rows)):
            await pilot.press("down")
        await pilot.pause(0.04)
        assert app.focused is ov_grid

        # Pressing 'a' on Overview CoverGrid queues the selected album's tracks
        before_q = len(app.engine.queue.tracks)
        await pilot.press("a")
        await pilot.pause(0.1)
        assert len(app.engine.queue.tracks) == before_q + 3

        await pilot.press("down")
        await pilot.pause(0.04)
        assert app.focused is sim_tbl

        await pilot.press("up")
        await pilot.pause(0.04)
        assert app.focused is ov_grid
        await pilot.press("up")
        await pilot.pause(0.04)
        assert app.focused is songs_tbl

        # Videos tab forces video mini icon ▷ even when video_type is unset on Track
        av.switch_tab("videos")
        await pilot.pause(0.05)
        assert "▷" in songs_tbl.render().plain


@pytest.mark.asyncio
async def test_nested_back_navigation_preserves_artist_and_playlist_state() -> None:
    from musick.ui.messages import OpenArtist, OpenPlaylist

    app = _make_test_app()
    async with app.run_test(size=(120, 36)) as pilot:
        await pilot.pause(0.1)

        # 1. Artist -> Albums tab -> Open Album -> Backspace preserves Albums tab without re-fetch
        app.post_message(OpenArtist("UC1", "The Weeknd"))
        for _ in range(20):
            await pilot.pause(0.05)
            if (
                isinstance(app._active_view, ArtistView)
                and len(app._active_view.query_one("#art-songs-tbl", TrackTable).rows) > 0
            ):
                break
        av = app._active_view
        assert isinstance(av, ArtistView)
        av.switch_tab("albums")
        await pilot.pause(0.1)
        assert av._active_tab == "albums"
        await pilot.press("enter")
        await pilot.pause(0.15)
        assert isinstance(app._active_view, AlbumView)

        await pilot.press("backspace")
        await pilot.pause(0.08)
        assert app._active_view is av
        assert av._active_tab == "albums"

        # 2. Playlist 'v' preserves cursor index & Esc clears active filter before popping
        app.post_message(OpenPlaylist("PL_COMM", "Lofi Beats to Study"))
        for _ in range(20):
            await pilot.pause(0.05)
            if (
                isinstance(app._active_view, PlaylistView)
                and len(app._active_view.query_one("#pl-tbl", TrackTable).rows) == 3
            ):
                break
        pv = app._active_view
        assert isinstance(pv, PlaylistView)
        pl_tbl = pv.query_one("#pl-tbl", TrackTable)
        pl_media = pv.query_one("#pl-media", MediaList)
        await pilot.press("down")
        assert pl_tbl.cursor == 1

        await pilot.press("v")
        await pilot.pause(0.05)
        assert pl_media.cursor == 1
        assert app.focused is pl_media

        await pilot.press("slash")
        await pilot.pause(0.03)
        pv.query_one("#pl-filter-input", Input).insert_text_at_cursor("Starboy")
        await pilot.pause(0.05)
        assert len(pl_media.items) == 1
        await pilot.press("escape")
        await pilot.pause(0.03)
        assert app.focused is pl_media
        await pilot.press("escape")
        await pilot.pause(0.05)
        assert app._active_view is pv
        assert len(pl_media.items) == 3


@pytest.mark.asyncio
async def test_narrow_80x24_playlist_hero_title_desc_and_artist_table() -> None:
    from musick.ui.messages import OpenArtist, OpenPlaylist

    app = _make_test_app()
    async with app.run_test(size=(80, 24)) as pilot:
        await pilot.pause(0.1)

        app.post_message(OpenPlaylist("PL_COMM", "Lofi Beats to Study"))
        for _ in range(20):
            await pilot.pause(0.05)
            if (
                isinstance(app._active_view, PlaylistView)
                and len(app._active_view.query_one("#pl-tbl", TrackTable).rows) == 3
            ):
                break
        strips = app.screen._compositor.render_strips(app.screen.size)
        lines = ["".join(seg.text for seg in st) for st in strips]
        assert any("─ Playlist ─" in ln for ln in lines)
        assert not any("Playli…" in ln for ln in lines)
        assert any("Relaxed beats to study" in ln for ln in lines)

        app.post_message(OpenArtist("UC1", "The Weeknd"))
        for _ in range(20):
            await pilot.pause(0.05)
            if (
                isinstance(app._active_view, ArtistView)
                and len(app._active_view.query_one("#art-songs-tbl", TrackTable).rows) == 3
            ):
                break
        strips_art = app.screen._compositor.render_strips(app.screen.size)
        art_lines = ["".join(seg.text for seg in st) for st in strips_art]
        assert any("Blinding Lights" in ln for ln in art_lines)


@pytest.mark.asyncio
async def test_settings_cava_audio_options_and_queue_next_up_controls() -> None:
    app = _make_test_app(with_last_track=True)
    async with app.run_test(size=(100, 36)) as pilot:
        await pilot.pause(0.15)

        # SettingsView updates new Cava palette, motion, autoplay, normalize & audio quality live
        await pilot.press("4")
        await pilot.pause(0.1)
        sv = app._active_view
        assert isinstance(sv, SettingsView)
        sv.query_one("#cvpal-aurora", Button).press()
        sv.query_one("#cvsm-balanced", Button).press()
        sv.query_one("#csrc-off", Button).press()
        sv.query_one("#qual-balanced", Button).press()
        sv.query_one("#tog-normalize", SettingToggle).press()
        sv.query_one("#tog-autoplay", SettingToggle).press()
        await pilot.pause(0.1)

        assert app.settings.display.cava_palette == "aurora"
        assert app.settings.display.cava_smoothing == "balanced"
        assert app.settings.display.cava_audio_source == "off"
        assert app.query_one("#home-cava", CavaVisualizer).mode_label == "off"
        sv.query_one("#csrc-auto", Button).press()
        await pilot.pause(0.05)
        assert app.settings.display.cava_audio_source == "auto"
        assert app.query_one("#home-cava", CavaVisualizer)._audio_source == "auto"
        assert app.settings.player.audio_quality == "balanced"
        assert app.settings.player.normalize_audio is True
        assert app.settings.player.autoplay is False
        assert app.resolver.audio_quality == "balanced"

        # Enqueue sample tracks and test QueueView Next Up promotion, dedup, clear upcoming & undo
        tracks = FakeCatalog().tracks
        await app.engine.play_tracks(tracks, start=0)
        app.engine.enqueue([tracks[1]])  # intentional duplicate
        await pilot.press("3")
        await pilot.pause(0.1)
        qv = app._active_view
        assert isinstance(qv, QueueView)
        q_tbl = qv.query_one("#q-tbl", TrackTable)
        assert len(q_tbl.rows) == 4

        qv.query_one("#q-dedup", Button).press()
        await pilot.pause(0.05)
        assert len(q_tbl.rows) == 3

        q_tbl.set_cursor(2)
        qv.query_one("#q-top", Button).press()
        await pilot.pause(0.05)
        assert app.engine.queue.tracks[1].video_id == tracks[2].video_id
        assert app.engine.queue.role_at(1) == "next"

        qv.query_one("#q-clr-up", Button).press()
        await pilot.pause(0.05)
        assert len(q_tbl.rows) == 1

        qv.query_one("#q-undo", Button).press()
        await pilot.pause(0.05)
        assert len(q_tbl.rows) == 3

        # Search view includes Videos tab
        await pilot.press("1")
        await pilot.pause(0.1)
        hv = app._active_view
        assert isinstance(hv, HomeView)
        hv.set_query("v: blinding", run=True)
        await pilot.pause(0.15)
        assert hv._category == "videos"


@pytest.mark.asyncio
async def test_artist_redesign_split_overview_songs_mini_covers_and_fit_grid() -> None:
    from musick.ui.messages import OpenArtist

    app = _make_test_app(with_last_track=True)
    async with app.run_test(size=(120, 36)) as pilot:
        await pilot.pause(0.1)
        app.post_message(OpenArtist("UC1", "The Weeknd"))
        for _ in range(30):
            await pilot.pause(0.05)
            if (
                isinstance(app._active_view, ArtistView)
                and len(app._active_view.query_one("#art-songs-tbl", TrackTable).rows) == 3
            ):
                break
        await pilot.pause(0.05)

        av = app._active_view
        assert isinstance(av, ArtistView)
        ov_grid = av.query_one("#art-ov-grid")
        assert ov_grid.has_class("is-split") is True
        assert av.query_one("#art-ov-latest-wrap").display is True
        assert "Take My Breath" in av.query_one("#art-ov-latest-title").render().plain
        assert av.query_one("#art-ov-rel-grid", CoverGrid).display is True
        assert len(av.query_one("#art-ov-sim-tbl", TrackTable).rows) == 1
        assert "1.4B" in av.query_one("#art-songs-tbl", TrackTable).render().plain

        # Latest release 'Open' button navigates to AlbumView; Backspace returns
        av.query_one("#art-latest-open", Button).press()
        await pilot.pause(0.15)
        assert isinstance(app._active_view, AlbumView)
        await pilot.press("backspace")
        await pilot.pause(0.08)
        assert app._active_view is av

        # Songs tab uses MediaList with mini covers, plays on Enter, queues on 'a'
        av.switch_tab("songs")
        await pilot.pause(0.12)
        songs_media = av.query_one("#art-songs-media", MediaList)
        assert songs_media.display is True
        assert songs_media.show_thumbs is True
        assert app.focused is songs_media
        assert len(songs_media.items) == 3

        before_q = len(app.engine.queue.tracks)
        await pilot.press("a")
        await pilot.pause(0.06)
        assert len(app.engine.queue.tracks) == before_q + 1

        await pilot.press("down")
        await pilot.press("enter")
        await pilot.pause(0.12)
        assert app.engine.state.track is not None
        assert app.engine.state.track.video_id == "vid2"
        assert songs_media.playing_index == 1

        # Albums & Singles tabs default to enlarged fit-width CoverGrid
        for rel_tab in ("albums", "singles"):
            av.switch_tab(rel_tab)
            await pilot.pause(0.12)
            rel_grid = av.query_one("#art-rel-grid", CoverGrid)
            assert av._grid_mode is True
            assert rel_grid.display is True
            assert rel_grid.fit_width is True
            assert rel_grid.scale == 1.25
            std_w, _ = cover_dims("M")
            assert rel_grid._layout_sig is not None
            assert rel_grid._layout_sig[1] > std_w
            assert rel_grid.region.right <= av.query_one("#art-body-panel").content_region.right

    # Narrow 80x24 drops split mode cleanly
    narrow_app = _make_test_app()
    async with narrow_app.run_test(size=(80, 24)) as pilot:
        await pilot.pause(0.08)
        narrow_app.post_message(OpenArtist("UC1", "The Weeknd"))
        for _ in range(25):
            await pilot.pause(0.05)
            if (
                isinstance(narrow_app._active_view, ArtistView)
                and len(narrow_app._active_view.query_one("#art-songs-tbl", TrackTable).rows) == 3
            ):
                break
        av_narrow = narrow_app._active_view
        assert isinstance(av_narrow, ArtistView)
        assert av_narrow.query_one("#art-ov-grid").has_class("is-split") is False
