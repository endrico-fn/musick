"""Unit tests for MediaList, LyricsBox, CavaVisualizer, and TrackTable enhancements."""

from __future__ import annotations

import pytest

from musick.domain import (
    AlbumSummary,
    ArtistRef,
    ArtistSummary,
    LyricLine,
    Lyrics,
    PlaylistSummary,
    Track,
)
from musick.ui.theme import (
    ACCENT,
    CURSOR_BG,
    NERD_ICONS,
    TR_ACTIVE,
    TR_UPCOMING,
    UNICODE_ICONS,
)
from musick.ui.widgets import (
    CavaVisualizer,
    ColumnDef,
    LyricsBox,
    MediaList,
    SectionTabs,
    TrackTable,
    build_cava_config,
    media_item_from,
    parse_cava_frame,
    render_spectrum,
)


def test_media_list_two_line_layout_and_narrow_truncation(monkeypatch) -> None:
    from textual.geometry import Size

    track = Track(
        video_id="v1",
        title="Smack That (feat. Eminem)",
        artists=(ArtistRef("Akon", "UC_AKON"), ArtistRef("Eminem", "UC_EM")),
        album="Konvicted",
        album_id="MPRE_KON",
        duration=212,
    )
    ml = MediaList()
    ml.set_items([media_item_from(track)], playing_index=0)

    # Render at standard width
    rendered = ml.render()
    lines = rendered.plain.splitlines()
    assert len(lines) == 2
    assert "Smack That (feat. Eminem)" in lines[0]
    assert lines[0].rstrip().endswith("3:32")
    assert "Akon, Eminem" in lines[1]
    assert lines[1].rstrip().endswith("Konvicted")

    # Narrow width must never overflow or produce >2 lines per item
    monkeypatch.setattr(MediaList, "size", property(lambda self: Size(24, 10)))
    narrow_lines = ml.render().plain.splitlines()
    assert len(narrow_lines) == 2
    assert all(len(ln) <= 24 for ln in narrow_lines)


def test_lyrics_box_wraps_centers_vertically_and_highlights_only_active(monkeypatch) -> None:
    from rich.cells import cell_len
    from textual.geometry import Size

    box = LyricsBox()
    lyr = Lyrics(
        lines=(
            LyricLine(1.0, "First short line"),
            LyricLine(4.0, "This is a much longer lyric line that should wrap cleanly"),
            LyricLine(8.0, "Next line coming up"),
        ),
        synced=True,
        source="LRCLIB",
    )
    box.set_lyrics(lyr, active_idx=1, has_track=True)
    rendered = box.render()
    plain = rendered.plain
    assert "▸" not in plain
    assert "This is a much longer" in plain
    bold_accent_spans = [
        sp for sp in rendered.spans if "bold" in str(sp.style) and ACCENT in str(sp.style)
    ]
    assert len(bold_accent_spans) >= 1
    assert not any(CURSOR_BG in str(sp.style) for sp in rendered.spans)

    # True vertical centering at start, middle, and end (matching tmp.txt)
    five = Lyrics(
        lines=tuple(LyricLine(float(i + 1), f"Line {i}") for i in range(5)),
        synced=True,
        source="LRCLIB",
    )
    centered = LyricsBox()
    monkeypatch.setattr(LyricsBox, "size", property(lambda self: Size(40, 5)))
    monkeypatch.setattr(LyricsBox, "has_focus", property(lambda self: True))

    centered.set_lyrics(five, active_idx=0, has_track=True)
    start_lines = centered.render().plain.split("\n")
    assert start_lines == ["", "", "Line 0", "Line 1", "Line 2"]

    centered.set_lyrics(five, active_idx=2, has_track=True)
    mid_lines = centered.render().plain.split("\n")
    assert mid_lines == ["Line 0", "Line 1", "Line 2", "Line 3", "Line 4"]

    centered.set_lyrics(five, active_idx=4, has_track=True)
    end_lines = centered.render().plain.split("\n")
    assert end_lines == ["Line 2", "Line 3", "Line 4", "", ""]

    # Even when focused, no CURSOR_BG selection bar appears and CJK wrapping respects display width
    cjk_box = LyricsBox()
    cjk_box.set_lyrics(
        Lyrics((LyricLine(1.0, "君の知らない物語を歌い続けよう"),), synced=True, source="LRCLIB"),
        active_idx=0,
        has_track=True,
    )
    monkeypatch.setattr(LyricsBox, "size", property(lambda self: Size(22, 6)))
    cjk_render = cjk_box.render()
    cjk_lines = cjk_render.plain.split("\n")
    assert len(cjk_lines) == 6
    assert all(cell_len(ln) <= 22 for ln in cjk_lines)
    assert "▸" not in cjk_render.plain
    assert not any(CURSOR_BG in str(sp.style) for sp in cjk_render.spans)


def test_bilingual_lyrics_compact_vs_spacious_mode(monkeypatch) -> None:
    from textual.geometry import Size

    lyr = Lyrics(
        lines=(
            LyricLine(1.0, "I see trees of green", "Kulihat pohon hijau"),
            LyricLine(4.0, "Red roses too", "Juga mawar merah"),
            LyricLine(8.0, "Yeah", "Yeah"),  # identical -> not repeated
        ),
        synced=True,
        source="LRCLIB",
        translation_lang="id",
    )

    # Compact LyricsBox: no blank lines between stanzas, centered vertically on active block
    box = LyricsBox(blank_between=False)
    monkeypatch.setattr(LyricsBox, "size", property(lambda self: Size(40, 6)))
    box.set_lyrics(lyr, active_idx=0, has_track=True, show_translation=True)
    rendered = box.render()
    assert "▸" not in rendered.plain
    assert rendered.plain.split("\n") == [
        "",
        "",
        "I see trees of green",
        "Kulihat pohon hijau",
        "Red roses too",
        "Juga mawar merah",
    ]
    styles = [str(sp.style) for sp in rendered.spans]
    assert any("bold" in s and ACCENT in s for s in styles)
    assert any(TR_ACTIVE in s for s in styles)
    assert any(TR_UPCOMING in s for s in styles)
    assert not any(CURSOR_BG in s for s in styles)

    # Spacious mode (Home lyrics:y): blank line separating each (en, id) stanza, centered vertically
    spacious = LyricsBox(blank_between=True)
    monkeypatch.setattr(LyricsBox, "size", property(lambda self: Size(40, 9)))
    spacious.set_lyrics(lyr, active_idx=1, has_track=True, show_translation=True)
    spacious_render = spacious.render()
    assert "▸" not in spacious_render.plain
    assert spacious_render.plain.split("\n") == [
        "I see trees of green",
        "Kulihat pohon hijau",
        "",
        "Red roses too",
        "Juga mawar merah",
        "",
        "Yeah",
        "",
        "",
    ]


def test_cava_parser_config_and_spectrum_renderer(monkeypatch) -> None:
    import shutil
    import subprocess

    from musick.ui.widgets.cava import resolve_cava_source

    cfg = build_cava_config(16, method="pipewire", max_range=100)
    assert "bars = 16" in cfg
    assert "method = pipewire" in cfg
    assert "source = auto" in cfg
    assert "source = monitor" not in cfg
    assert "method = raw" in cfg
    assert "channels = mono" in cfg
    assert "monstercat = 0" in cfg
    assert "noise_reduction = 42" in cfg

    # Legacy/explicit 'monitor' never emits literal 'source = monitor' to PipeWire
    pw_mon = build_cava_config(16, method="pipewire", audio_source="monitor")
    assert "source = auto" in pw_mon
    assert "source = monitor" not in pw_mon

    monkeypatch.setattr(shutil, "which", lambda cmd: "/usr/bin/pactl" if cmd == "pactl" else None)

    class _FakeRun:
        returncode = 0
        stdout = "alsa_output.test.analog-stereo\n"

    monkeypatch.setattr(subprocess, "run", lambda *a, **kw: _FakeRun())
    assert resolve_cava_source("pulse", "monitor") == "alsa_output.test.analog-stereo.monitor"
    assert resolve_cava_source("pipewire", "monitor") == "auto"

    # Strips OSC terminal title prefix emitted by cava 1.0.0
    vals = parse_cava_frame("\x1b]0;cava\x07 0;25;50;100;\r\n", max_range=100)
    assert vals == [0.0, 0.25, 0.5, 1.0]
    assert parse_cava_frame("not_a_frame", max_range=100) is None

    txt = render_spectrum([0.0, 0.5, 1.0, 0.25], width=30, height=8, bar_width=1, gap=1, idle=False)
    lines = txt.plain.splitlines()
    assert len(lines) == 8
    assert "█" in lines[0] or "█" in lines[1]
    assert "▁" in lines[-1]
    # Smooth TrueColor gradient produces >3 distinct hex colors across 8 rows
    distinct_colors = {str(sp.style) for sp in txt.spans if str(sp.style).startswith("#")}
    assert len(distinct_colors) >= 6

    vis = CavaVisualizer(enable_subprocess=False)
    assert vis.feed_raw_line("10;40;90;\n") is True
    assert vis.mode_label == "idle"
    vis._cava_max_range = 1000
    assert vis.feed_raw_line("500;1000;\n") is True
    assert vis._cava_bars == [0.5, 1.0]


def test_track_table_allow_none_cursor_and_narrow_widths() -> None:
    tbl = TrackTable(
        [ColumnDef("Query", None, "l"), ColumnDef("Source", 12, "r")],
        show_header=False,
        allow_none_cursor=True,
    )
    tbl.set_data([(["akon lonely", "Suggest"], "s1"), (["akon smack that", "Suggest"], "s2")])
    assert tbl.cursor == -1
    tbl.move_cursor(1)
    assert tbl.cursor == 0
    tbl.move_cursor(1)
    assert tbl.cursor == 1
    tbl.move_cursor(-1)
    assert tbl.cursor == 0
    tbl.move_cursor(-1)
    assert tbl.cursor == -1

    # Narrow terminal must scale columns without overflowing
    ws = tbl._compute_widths(22)
    assert sum(ws) + 2 + 2 <= 22

    # 80-col Artist table must protect Title width before letting Album hoard 24 cols
    art_tbl = TrackTable(
        [
            ColumnDef("#", 3, "r"),
            ColumnDef("Title", None, "l"),
            ColumnDef("Album", 24, "l"),
            ColumnDef("Plays", 10, "r"),
            ColumnDef("Time", 8, "r"),
        ],
        icon_col="Title",
    )
    art_ws = art_tbl._compute_widths(70)
    assert art_ws[1] >= 20
    assert sum(art_ws) + 2 + 8 <= 70


@pytest.mark.asyncio
async def test_cava_visualizer_real_pipe_stream(tmp_path) -> None:
    import asyncio
    import os
    import shutil
    import struct
    import threading

    if shutil.which("cava") is None:
        pytest.skip("cava binary not installed")

    in_fifo = tmp_path / "in.fifo"
    os.mkfifo(in_fifo)
    conf = tmp_path / "cava.conf"
    conf.write_text(
        "[general]\nbars = 8\nframerate = 30\nsleep_timer = 0\n"
        f"[input]\nmethod = fifo\nsource = {in_fifo}\n"
        "[output]\nmethod = raw\nraw_target = /dev/stdout\n"
        "data_format = ascii\nascii_max_range = 100\n",
        encoding="utf-8",
    )

    vis = CavaVisualizer(enable_subprocess=False)
    vis._conf_path = conf
    task = asyncio.create_task(vis._run_cava(conf))
    vis._watch_task = task

    def _feed_pcm() -> None:
        try:
            with in_fifo.open("wb") as wf:
                chunk = b"".join(struct.pack("<hh", 18000, 18000) for _ in range(4410))
                for _ in range(6):
                    wf.write(chunk)
        except OSError:
            pass

    t = threading.Thread(target=_feed_pcm, daemon=True)
    t.start()

    try:
        for _ in range(60):
            if vis._cava_bars:
                break
            await asyncio.sleep(0.05)
        assert len(vis._cava_bars) == 8
    finally:
        proc = vis._proc
        vis._stop_cava()
        if proc is not None and proc.returncode is None:
            proc.kill()
            await proc.wait()
        await asyncio.sleep(0.05)
        t.join(timeout=1.0)


def test_cjk_cell_width_and_narrow_table_headers_and_section_tabs(monkeypatch) -> None:
    from rich.cells import cell_len
    from textual.geometry import Size

    # CJK wide characters in MediaList must respect terminal display cell widths
    jp_track = Track(
        video_id="jp1",
        title="夜に駆ける (Racing Into The Night)",
        artists=(ArtistRef("YOASOBI", "UC_Y"),),
        album="THE BOOK",
        duration=260,
    )
    ml = MediaList()
    ml.set_items([media_item_from(jp_track)])
    monkeypatch.setattr(MediaList, "size", property(lambda self: Size(36, 10)))
    lines = ml.render().plain.splitlines()
    assert len(lines) == 2
    assert all(cell_len(ln) <= 36 for ln in lines)
    assert lines[0].rstrip().endswith("4:20")

    # TrackTable keeps compact '#' column >= 3 chars so '# ▾' never turns into '#…'
    tbl = TrackTable(
        [
            ColumnDef("#", 3, "r"),
            ColumnDef("Title", None, "l"),
            ColumnDef("Artist", 22, "l"),
            ColumnDef("Time", 8, "r"),
        ],
        sort_col="#",
    )
    ws = tbl._compute_widths(46)
    assert ws[0] == 3
    assert ws[3] == 8

    # SectionTabs fits narrow width cleanly (full labels at 46, short labels at 42)
    tabs = SectionTabs(
        [
            ("all", "All"),
            ("songs", "Songs"),
            ("albums", "Albums"),
            ("artists", "Artists"),
            ("playlists", "Playlists"),
            ("community", "Community"),
        ],
        active="community",
        short_labels={"community": "Comm."},
    )
    monkeypatch.setattr(SectionTabs, "size", property(lambda self: Size(46, 2)))
    tab_lines = tabs.render().plain.splitlines()
    assert len(tab_lines) == 2
    assert len(tab_lines[0]) <= 46
    assert "Community" in tab_lines[0]

    monkeypatch.setattr(SectionTabs, "size", property(lambda self: Size(42, 2)))
    short_lines = tabs.render().plain.splitlines()
    assert len(short_lines[0]) <= 42
    assert "Comm." in short_lines[0]


@pytest.mark.asyncio
async def test_cava_rapid_restart_does_not_clobber_newer_handles(tmp_path, monkeypatch) -> None:
    import asyncio
    import contextlib

    vis = CavaVisualizer(enable_subprocess=False)
    conf1 = tmp_path / "c1.conf"
    conf2 = tmp_path / "c2.conf"
    conf1.write_text("", encoding="utf-8")
    conf2.write_text("", encoding="utf-8")

    class FakeProc:
        def __init__(self, gate: asyncio.Event) -> None:
            self.returncode = None
            self._gate = gate

        async def wait(self) -> int:
            await self._gate.wait()
            self.returncode = 0
            return 0

        def terminate(self) -> None:
            self.returncode = -15
            self._gate.set()

    gates = [asyncio.Event(), asyncio.Event()]
    procs: list[FakeProc] = []

    async def fake_exec(*args, **kwargs):
        p = FakeProc(gates[len(procs)])
        procs.append(p)
        return p

    monkeypatch.setattr(asyncio, "create_subprocess_exec", fake_exec)

    vis._conf_path = conf1
    t1 = asyncio.create_task(vis._run_cava(conf1))
    vis._watch_task = t1
    await asyncio.sleep(0.02)
    assert len(procs) == 1

    # Simulate rapid restart: cancel t1 and launch t2 before t1's finally finishes
    t1.cancel()
    vis._conf_path = conf2
    t2 = asyncio.create_task(vis._run_cava(conf2))
    vis._watch_task = t2
    await asyncio.sleep(0.02)
    with contextlib.suppress(asyncio.CancelledError):
        await t1

    # t1's finally must NOT have clobbered t2's active handles or conf2 file!
    assert len(procs) == 2
    assert vis._watch_task is t2
    assert vis._conf_path == conf2
    assert conf2.exists()
    assert not conf1.exists()
    assert vis._proc is procs[1]

    vis._stop_cava()
    await asyncio.sleep(0.02)
    with contextlib.suppress(asyncio.CancelledError):
        await t2
    assert vis._proc is None


@pytest.mark.asyncio
async def test_cava_start_survives_eager_task_factory(monkeypatch) -> None:
    import asyncio
    import shutil

    from textual.app import App, ComposeResult

    loop = asyncio.get_running_loop()
    prev_factory = loop.get_task_factory()
    if hasattr(asyncio, "eager_task_factory"):
        loop.set_task_factory(asyncio.eager_task_factory)

    class FakeProc:
        def __init__(self) -> None:
            self.returncode: int | None = None
            self.terminated = False
            self._gate = asyncio.Event()

        async def wait(self) -> int:
            await self._gate.wait()
            return 0 if not self.terminated else -15

        def terminate(self) -> None:
            self.terminated = True
            self.returncode = -15
            self._gate.set()

    procs: list[FakeProc] = []

    async def fake_exec(*args, **kwargs):
        p = FakeProc()
        procs.append(p)
        return p

    monkeypatch.setattr(asyncio, "create_subprocess_exec", fake_exec)
    monkeypatch.setattr(shutil, "which", lambda cmd: "/usr/bin/cava" if cmd == "cava" else None)

    class MiniApp(App):
        def compose(self) -> ComposeResult:
            yield CavaVisualizer(enable_subprocess=True, id="cv")

    try:
        app = MiniApp()
        async with app.run_test(size=(80, 12)) as pilot:
            cv = app.query_one("#cv", CavaVisualizer)
            cv.set_playback_state(playing=True, paused=False, track_key="t1")
            await pilot.pause(0.1)
            assert len(procs) == 1
            assert not procs[0].terminated
            assert cv._proc is procs[0]
            assert not cv._cava_failed
            cv.feed_raw_line(";".join(["50"] * 16) + ";\n")
            await pilot.pause(0.06)
            assert cv.mode_label == "live"

            # Silent backend (>2.2s with 0 frames) rotates cleanly to next backend method
            cv._proc_has_frame = False
            cv._cava_start_ts -= 3.0
            cv._on_anim_tick()
            await pilot.pause(0.06)
            assert cv._method_idx == 1
            assert len(procs) == 2

            # Reduced motion exposes explicit 'static' mode label rather than 'no signal'
            cv.set_playback_state(playing=True, paused=False, track_key="t1", reduced_motion=True)
            assert cv.mode_label == "static"

            # While hidden (set_active_visible(False)), playback state updates must NOT spawn cava
            cv.set_active_visible(False)
            cv.set_playback_state(playing=True, paused=False, track_key="t1", reduced_motion=False)
            await pilot.pause(0.05)
            assert cv._proc is None
            assert cv._anim_timer is None
            assert len(procs) == 2
    finally:
        loop.set_task_factory(prev_factory)


def test_mini_category_icons_in_media_list_and_track_table() -> None:
    from rich.cells import cell_len

    song = Track("t1", "Song Title", (ArtistRef("Artist"),), duration=180)
    video = Track(
        "v1", "MV Title", (ArtistRef("Artist"),), duration=210, video_type="MUSIC_VIDEO_TYPE_OMV"
    )
    album = AlbumSummary("MPRE1", "Album Title", "2024", "Album", (ArtistRef("Artist"),))
    artist = ArtistSummary("UC1", "Artist Name", "10M")
    playlist = PlaylistSummary("PL1", "Playlist Title", "Curator", "20")

    ml = MediaList(icons=UNICODE_ICONS)
    ml.set_items([media_item_from(x) for x in (song, video, album, artist, playlist)])
    u_plain = ml.render().plain
    assert "♪ Song Title" in u_plain
    assert "▷ MV Title" in u_plain
    assert "◎ Album Title" in u_plain
    assert "◉ Artist Name" in u_plain
    assert "≡ Playlist Title" in u_plain

    ml.set_icons(NERD_ICONS)
    n_plain = ml.render().plain
    assert f"{NERD_ICONS.song} Song Title" in n_plain
    assert f"{NERD_ICONS.video} MV Title" in n_plain
    assert f"{NERD_ICONS.album} Album Title" in n_plain
    assert f"{NERD_ICONS.artist} Artist Name" in n_plain
    assert f"{NERD_ICONS.playlist} Playlist Title" in n_plain

    tbl = TrackTable(
        [ColumnDef("#", 3, "r"), ColumnDef("Title", None, "l")],
        icons=UNICODE_ICONS,
        icon_col="Title",
    )
    tbl.set_data(
        [(["1", song.title], song), (["2", video.title], video), (["3", album.title], album)]
    )
    t_lines = tbl.render().plain.splitlines()
    assert any("♪ Song Title" in ln for ln in t_lines)
    assert any("▷ MV Title" in ln for ln in t_lines)
    assert any("◎ Album Title" in ln for ln in t_lines)
    assert all(cell_len(ln) <= 94 for ln in t_lines)


@pytest.mark.asyncio
async def test_scroll_child_region_actually_scrolls_vertical_scroll() -> None:
    from textual.app import App, ComposeResult
    from textual.containers import VerticalScroll

    class ScrollApp(App):
        def compose(self) -> ComposeResult:
            with VerticalScroll(id="vs"):
                yield TrackTable([ColumnDef("#", 3, "r"), ColumnDef("Title", None, "l")], id="tbl")

    app = ScrollApp()
    async with app.run_test(size=(50, 8)) as pilot:
        vs = app.query_one("#vs", VerticalScroll)
        tbl = app.query_one("#tbl", TrackTable)
        tbl.set_data([([str(i), f"Track {i}"], i) for i in range(1, 30)])
        tbl.focus()
        await pilot.pause(0.05)
        assert vs.scroll_offset.y == 0
        tbl.move_cursor(20)
        await pilot.pause(0.05)
        assert vs.scroll_offset.y > 0
        tbl.move_cursor(-20)
        await pilot.pause(0.05)
        assert vs.scroll_offset.y <= 2


def test_init_terminal_image_support_fallbacks(monkeypatch) -> None:
    import sys

    from textual_image import _terminal

    from musick.ui.widgets import cover as cover_mod

    class FakeTTY:
        def __init__(self, tty: bool) -> None:
            self._tty = tty

        def isatty(self) -> bool:
            return self._tty

    has_prev = hasattr(_terminal.probe_terminal, "_result")
    prev_result = getattr(_terminal.probe_terminal, "_result", None)
    monkeypatch.delenv("PYTEST_CURRENT_TEST", raising=False)
    monkeypatch.setenv("TERM", "xterm-256color")

    try:
        # 1. Real 0x0 PTY TIOCSWINSZ triggers ZeroDivisionError in probe_terminal() -> fallback
        monkeypatch.setattr(sys, "__stdout__", FakeTTY(True))
        monkeypatch.setattr(sys, "__stdin__", FakeTTY(True))
        monkeypatch.setattr(cover_mod, "_TERMINAL_PROBED", False)
        if hasattr(_terminal.probe_terminal, "_result"):
            delattr(_terminal.probe_terminal, "_result")
        monkeypatch.setattr(_terminal, "get_tiocgwinsz", lambda: (0, 0, 0, 0))
        cover_mod.init_terminal_image_support()
        res = _terminal.probe_terminal._result  # type: ignore[attr-defined]
        assert res.cell_size == _terminal.CellSize(10, 20)
        assert (res.sixel, res.tgp) == (False, False)

        # 2. TerminalError from probe_terminal() falls back safely without crashing
        def raise_terminal_error():
            if hasattr(raise_terminal_error, "_result"):
                return raise_terminal_error._result  # type: ignore[attr-defined]
            raise _terminal.TerminalError("closed")

        monkeypatch.setattr(_terminal, "probe_terminal", raise_terminal_error)
        monkeypatch.setattr(cover_mod, "_TERMINAL_PROBED", False)
        cover_mod.init_terminal_image_support()
        res = _terminal.probe_terminal._result  # type: ignore[attr-defined]
        assert res.cell_size == _terminal.CellSize(10, 20)

        # 3. Non-TTY stdin skips terminal escape probing even when stdout is a TTY
        called = False

        def unexpected_probe():
            nonlocal called
            if hasattr(unexpected_probe, "_result"):
                return unexpected_probe._result  # type: ignore[attr-defined]
            called = True
            return _terminal.TerminalCapabilities(_terminal.CellSize(8, 16), False, False)

        monkeypatch.setattr(_terminal, "probe_terminal", unexpected_probe)
        monkeypatch.setattr(sys, "__stdin__", FakeTTY(False))
        monkeypatch.setattr(cover_mod, "_TERMINAL_PROBED", False)
        cover_mod.init_terminal_image_support()
        assert called is False
        assert _terminal.probe_terminal._result.cell_size == _terminal.CellSize(10, 20)  # type: ignore[attr-defined]
    finally:
        monkeypatch.undo()
        if has_prev:
            _terminal.probe_terminal._result = prev_result  # type: ignore[attr-defined]
        elif hasattr(_terminal.probe_terminal, "_result"):
            delattr(_terminal.probe_terminal, "_result")
        cover_mod._TERMINAL_PROBED = True


def test_cava_smooth_profiles_palettes_and_media_list_queue_roles() -> None:
    from musick.config import CAVA_PALETTES

    smooth_cfg = build_cava_config(24, method="pipewire", max_range=100, smoothing="smooth")
    assert "monstercat = 1" in smooth_cfg
    assert "noise_reduction = 62" in smooth_cfg

    # Smooth rise progressive envelope avoids hard 1-frame snap jumps
    vis = CavaVisualizer(enable_subprocess=False, smoothing="smooth", palette="aurora")
    vis._bars = [0.0] * 8
    vis._playing = True
    vis._paused = False
    vis._cava_bars = [1.0] * 8
    vis._last_frame_ts = 1e12  # simulate fresh live frame timestamp cleanly
    import time

    vis._last_frame_ts = time.monotonic()
    vis._on_anim_tick()
    assert 0.4 < vis._bars[0] < 0.85

    # Every gradient palette produces rich continuous TrueColor shades
    for pal in CAVA_PALETTES:
        txt = render_spectrum([0.1, 0.4, 0.8, 1.0], width=24, height=6, palette=pal)
        shades = {str(sp.style) for sp in txt.spans if str(sp.style).startswith("#")}
        assert len(shades) >= 6

    vis.set_appearance(palette="sunset", smoothing="balanced", enabled=False)
    assert vis.mode_label == "off"

    # Zero-amplitude frames must remain zero (no silent upward drift)
    silent = CavaVisualizer(enable_subprocess=False, smoothing="smooth")
    silent._bars = [0.0] * 8
    silent._playing = True
    silent._paused = False
    silent._cava_bars = [0.0] * 8
    silent._last_frame_ts = time.monotonic()
    silent._on_anim_tick()
    assert silent._bars == [0.0] * 8

    silent.set_appearance(audio_source="off")
    assert silent.mode_label == "off"

    # MediaList renders plays badge cleanly & dims played / highlights Next Up roles
    t = Track("v1", "Song", (ArtistRef("Artist"),), album="Alb", duration=200, plays="2.9B")
    ml = MediaList()
    ml.set_items(
        [
            media_item_from(t, role="played"),
            media_item_from(t, role="playing"),
            media_item_from(t, role="next"),
        ],
        playing_index=1,
    )
    plain = ml.render().plain
    assert "Alb · 2.9B" in plain
    assert "Next · Alb · 2.9B" in plain


@pytest.mark.asyncio
async def test_cover_grid_fit_scale_and_media_list_allow_a_queue(monkeypatch) -> None:
    import httpx
    from textual.app import App, ComposeResult
    from textual.geometry import Size

    from musick.config import Settings
    from musick.domain import Thumb
    from musick.services.images import ImageService
    from musick.ui.widgets import CoverGrid, GridCardItem, cover_dims
    from musick.ui.widgets.cover_grid import _GridCard
    from tests.conftest import make_png_bytes

    png = make_png_bytes()
    http = httpx.AsyncClient(
        transport=httpx.MockTransport(lambda r: httpx.Response(200, content=png))
    )
    images = ImageService(Settings(), http=http)

    class GridApp(App):
        def compose(self) -> ComposeResult:
            yield CoverGrid(
                images,
                cover_size="M",
                fit_width=True,
                scale=1.25,
                id="g",
            )

    app = GridApp()
    async with app.run_test(size=(100, 30)) as pilot:
        grid = app.query_one("#g", CoverGrid)
        items = [
            GridCardItem(
                f"Album {i}",
                "2025 · Album",
                "https://example.test/c.png",
                AlbumSummary(f"MPRE{i}", f"Album {i}", "2025"),
            )
            for i in range(6)
        ]
        grid.set_items(items)
        await pilot.pause(0.1)

        std_w, _ = cover_dims("M")
        cards = list(grid.query(_GridCard))
        assert cards
        assert cards[0].styles.width.value > std_w
        first_row = grid.children[0]
        assert list(first_row.children)[-1].styles.margin.right == 0
        assert all(c.region.right <= grid.region.right for c in cards)

    # MediaList allow_a_queue=True emits ItemQueued on 'a' and ItemOpenAlbum on 'o'
    ml = MediaList(allow_a_queue=True, images=images)
    tracks = [
        Track(
            f"v{i}",
            f"Song {i}",
            (ArtistRef("Artist", "UC1"),),
            album="Alb",
            album_id="MPRE1",
            duration=180,
            thumbs=(Thumb(f"https://example.test/t{i}.png", 120, 120),),
        )
        for i in range(90)
    ]
    ml.set_items([media_item_from(t) for t in tracks])

    posted: list[object] = []
    monkeypatch.setattr(ml, "post_message", lambda msg: posted.append(msg))
    monkeypatch.setattr(MediaList, "is_mounted", property(lambda self: True))
    monkeypatch.setattr(MediaList, "size", property(lambda self: Size(80, 30)))

    from textual.events import Key

    ml.on_key(Key("a", "a"))
    ml.on_key(Key("o", "o"))
    assert isinstance(posted[0], MediaList.ItemQueued)
    assert isinstance(posted[1], MediaList.ItemOpenAlbum)

    # Cursor-centered sliding thumbnail window reaches items beyond index 60
    requested: list[str] = []

    async def fake_fetch(url: str) -> None:
        requested.append(url)

    monkeypatch.setattr(ml, "_fetch_thumb", fake_fetch)
    ml.set_cursor(80)
    await __import__("asyncio").sleep(0.02)
    assert any("t80.png" in u for u in requested)
    await images.aclose()
