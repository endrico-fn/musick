"""Tests for PlayQueue, MpvController IPC (real Unix socket), and PlayerEngine."""

import asyncio
import contextlib
import json
import random
from pathlib import Path

import httpx
import pytest

from musick.config import Settings
from musick.player import MpvController, PlayerEngine, PlayQueue
from musick.services.lyrics import LyricsService
from musick.services.resolver import StreamResolver
from musick.services.search import HistoryStore
from tests.conftest import FakeCatalog, sample_tracks


def test_play_queue_shuffle_repeat_and_navigation() -> None:
    tracks = sample_tracks()
    q = PlayQueue(rng=random.Random(42))
    cur = q.set_tracks(tracks, start=1)
    assert cur
    assert cur.video_id == "vid2"

    # Toggle shuffle keeps current track at index 0
    assert q.toggle_shuffle() is True
    assert q.current
    assert q.current.video_id == "vid2"
    assert q.index == 0
    assert len(q.tracks) == 3

    # Unshuffle restores original order and points to current track
    assert q.toggle_shuffle() is False
    assert [t.video_id for t in q.tracks] == ["vid1", "vid2", "vid3"]
    assert q.index == 1

    assert q.cycle_repeat() == "all"
    q.jump(2)
    nxt = q.advance()
    assert nxt
    assert nxt.video_id == "vid1"

    assert q.cycle_repeat() == "one"
    same = q.advance(manual=False)
    assert same
    assert same.video_id == "vid1"
    manual_nxt = q.advance(manual=True)
    assert manual_nxt
    assert manual_nxt.video_id == "vid2"

    # Reorder queue items while keeping playing index aligned
    q.jump(0)
    assert q.move(0, 2) is True
    assert [t.video_id for t in q.tracks] == ["vid2", "vid3", "vid1"]
    assert q.index == 2
    assert q.move(2, 0) is True
    assert [t.video_id for t in q.tracks] == ["vid1", "vid2", "vid3"]
    assert q.index == 0


@pytest.mark.asyncio
async def test_mpv_controller_unix_ipc_and_engine(tmp_path: Path) -> None:
    sock = tmp_path / "mpv_test.sock"
    received_cmds: list[list] = []
    server_writer_holder: list[asyncio.StreamWriter] = []

    async def handle_client(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        server_writer_holder.append(writer)
        with contextlib.suppress(OSError, json.JSONDecodeError, ValueError):
            while True:
                raw = await reader.readline()
                if not raw:
                    break
                msg = json.loads(raw.decode("utf-8"))
                received_cmds.append(msg["command"])
                rid = msg.get("request_id")
                if rid is not None:
                    reply = {"request_id": rid, "error": "success", "data": None}
                    writer.write((json.dumps(reply) + "\n").encode("utf-8"))
                    await writer.drain()
        writer.close()

    server = await asyncio.start_unix_server(handle_client, path=str(sock))
    try:
        settings = Settings()
        mpv = MpvController(settings, socket_path=sock, spawn_process=False)

        async def fake_resolve(vid: str) -> str:
            return f"https://stream.test/{vid}"

        async with httpx.AsyncClient(
            transport=httpx.MockTransport(
                lambda r: httpx.Response(
                    200, json={"syncedLyrics": "[00:02.00]Line at 2s\n[00:10.00]Line at 10s"}
                )
            )
        ) as http_client:
            lyr_svc = LyricsService(settings, catalog=FakeCatalog(), http=http_client)
            resolver = StreamResolver(runner=fake_resolve)
            history = HistoryStore(settings)
            engine = PlayerEngine(settings, resolver, lyr_svc, history, mpv=mpv)

            await engine.play_tracks(sample_tracks(), start=0)
            await asyncio.sleep(0.05)

            assert engine.state.track
            assert engine.state.track.video_id == "vid1"
            assert any(cmd[0] == "loadfile" for cmd in received_cmds)
            assert engine.state.lyrics.synced is True

            # Simulate mpv pushing time-pos = 3.0s over IPC
            sw = server_writer_holder[0]
            sw.write(
                (
                    json.dumps({"event": "property-change", "name": "time-pos", "data": 3.0}) + "\n"
                ).encode("utf-8")
            )
            sw.write(
                (
                    json.dumps(
                        {"event": "property-change", "name": "paused-for-cache", "data": False}
                    )
                    + "\n"
                ).encode("utf-8")
            )
            await sw.drain()
            await asyncio.sleep(0.05)

            assert engine.state.position == 3.0
            assert engine.state.active_lyric_text == "Line at 2s"

            # Overdrive volume > 100% routes gain via @musicklim & caps mpv softvol at 100
            await engine.set_volume(115)
            await asyncio.sleep(0.03)
            assert engine.state.volume == 115
            assert ["set_property", "volume", 100] in received_cmds
            assert any(c[:2] == ["af", "add"] and "level=false" in c[2] for c in received_cmds)
            # mpv echoing volume=100 while overdrive target is 115 must not overwrite UI volume
            sw.write(
                (
                    json.dumps({"event": "property-change", "name": "volume", "data": 100}) + "\n"
                ).encode("utf-8")
            )
            await sw.drain()
            await asyncio.sleep(0.03)
            assert engine.state.volume == 115
            # Updating overdrive gain (115 -> 125) removes prior @musicklim before re-adding
            received_cmds.clear()
            await engine.set_volume(125)
            await asyncio.sleep(0.03)
            assert engine.state.volume == 125
            assert ["af", "remove", "@musicklim"] in received_cmds
            assert any(c[:2] == ["af", "add"] and "level=false" in c[2] for c in received_cmds)
            await engine.set_volume(80)
            await asyncio.sleep(0.03)
            assert ["af", "remove", "@musicklim"] in received_cmds

            # Simulate natural EOF advancing to track 2
            sw.write((json.dumps({"event": "end-file", "reason": "eof"}) + "\n").encode("utf-8"))
            await sw.drain()
            await asyncio.sleep(0.08)

            assert engine.state.track
            assert engine.state.track.video_id == "vid2"

            # Simulate mpv error -> invalidates cache, surfaces error, ignores stray core-idle
            assert resolver.get_cached("vid2") is not None
            sw.write(
                (
                    json.dumps({"event": "end-file", "reason": "error", "file_error": "http 403"})
                    + "\n"
                ).encode("utf-8")
            )
            await sw.drain()
            await asyncio.sleep(0.05)
            assert engine.state.playing is False
            assert engine.state.error == "http 403"
            assert resolver.get_cached("vid2") is None
            sw.write(
                (
                    json.dumps({"event": "property-change", "name": "core-idle", "data": True})
                    + "\n"
                ).encode("utf-8")
            )
            await sw.drain()
            await asyncio.sleep(0.03)
            assert engine.state.playing is False

            await engine.shutdown()
    finally:
        server.close()
        server.close_clients()
        await server.wait_closed()


@pytest.mark.asyncio
async def test_restore_last_track_and_play_on_toggle(tmp_path: Path) -> None:
    settings = Settings()

    class FakeMpv(MpvController):
        def __init__(self) -> None:
            super().__init__(settings, socket_path=tmp_path / "noop.sock", spawn_process=False)
            self.loaded: list[str] = []

        async def start(self) -> None:
            self._started = True

        async def stop(self) -> None:
            self._started = False

        async def load_url(self, url: str) -> None:
            self.loaded.append(url)

    mpv = FakeMpv()
    async with httpx.AsyncClient(
        transport=httpx.MockTransport(lambda r: httpx.Response(404))
    ) as client:
        lyr_svc = LyricsService(settings, catalog=FakeCatalog(), http=client)
        resolver = StreamResolver(
            runner=lambda vid: asyncio.sleep(0, result=f"https://stream.test/{vid}")
        )
        history = HistoryStore(settings)
        engine = PlayerEngine(settings, resolver, lyr_svc, history, mpv=mpv)

        last = sample_tracks()[0]
        engine.restore_last_track(last, fetch_lyrics=False)
        assert engine.state.track == last
        assert engine.state.restored is True
        assert engine.state.playing is False
        assert mpv.loaded == []

        # Pressing toggle_pause on restored track resolves and starts playback
        await engine.toggle_pause()
        assert engine.state.restored is False
        assert engine.state.playing is True
        assert mpv.loaded == [f"https://stream.test/{last.video_id}"]

        # clear_restored should NOT wipe an actively playing track
        engine.clear_restored()
        assert engine.state.track == last

        await engine.stop()
        engine.restore_last_track(last, fetch_lyrics=False)
        engine.clear_restored()
        assert engine.state.track is None
        await engine.shutdown()


@pytest.mark.asyncio
async def test_engine_error_retry_and_restored_seek_guard(tmp_path: Path) -> None:
    settings = Settings()
    attempts = 0

    async def flaky_runner(vid: str) -> str:
        nonlocal attempts
        attempts += 1
        if attempts == 1:
            raise RuntimeError("transient network failure")
        return f"https://stream.test/{vid}"

    class FakeMpv(MpvController):
        def __init__(self) -> None:
            super().__init__(settings, socket_path=tmp_path / "noop2.sock", spawn_process=False)
            self.loaded: list[str] = []

        async def load_url(self, url: str) -> None:
            self._started = True
            self.loaded.append(url)

    mpv = FakeMpv()
    async with httpx.AsyncClient(
        transport=httpx.MockTransport(lambda r: httpx.Response(404))
    ) as client:
        lyr_svc = LyricsService(settings, catalog=FakeCatalog(), http=client)
        resolver = StreamResolver(runner=flaky_runner)
        history = HistoryStore(settings)
        engine = PlayerEngine(settings, resolver, lyr_svc, history, mpv=mpv)

        trk = sample_tracks()[0]
        # seek_absolute on restored track must NOT advance position
        engine.restore_last_track(trk, fetch_lyrics=False)
        await engine.seek_absolute(42.0)
        assert engine.state.position == 0.0

        # First play fails -> _loaded_vid stays None so toggle_pause retries instead of pausing
        await engine.play_track(trk)
        assert engine.state.error == "transient network failure"
        assert engine.state.playing is False

        await engine.toggle_pause()
        assert engine.state.error is None
        assert engine.state.playing is True
        assert mpv.loaded == [f"https://stream.test/{trk.video_id}"]
        await engine.shutdown()


def test_play_queue_priority_next_up_undo_dedup_and_clear_upcoming() -> None:
    from musick.domain import Track

    t1, t2, t3 = sample_tracks()
    p1 = Track("prio1", "Next Priority 1", duration=180)
    p2 = Track("prio2", "Next Priority 2", duration=190)

    q = PlayQueue(rng=random.Random(7))
    q.set_tracks([t1, t2, t3], start=0)
    assert q.role_at(0) == "playing"
    assert q.role_at(1) == "queued"

    q.play_next(p1)
    assert q.priority_count == 1
    assert q.role_at(1) == "next"
    assert q.role_at(2) == "queued"

    # Promoting t3 to top moves it right after active track as priority Next Up
    assert q.move_to_next(3) is True
    assert q.tracks[1].video_id == "vid3"
    assert q.role_at(1) == "next"
    assert q.priority_count == 2

    # Shuffle preserves active track and Next Up priority items ahead of context
    q.toggle_shuffle()
    assert q.tracks[0].video_id == "vid1"
    assert {q.tracks[1].video_id, q.tracks[2].video_id} == {"vid3", "prio1"}

    # Deduplicate removes repeated upcoming track IDs safely
    q.append([p1, p2, p2])
    removed = q.deduplicate()
    assert removed == 2

    # Clear upcoming keeps active track, undo restores previous state
    cleared = q.clear_upcoming()
    assert cleared >= 3
    assert len(q.tracks) == 1
    assert q.undo() is True
    assert len(q.tracks) > 1


@pytest.mark.asyncio
async def test_mpv_normalize_audio_and_engine_autoplay_radio(tmp_path: Path) -> None:
    from musick.domain import Track

    sock = tmp_path / "mpv_norm.sock"
    cmds: list[list] = []
    writer_holder: list[asyncio.StreamWriter] = []

    async def handle_client(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        writer_holder.append(writer)
        with contextlib.suppress(OSError, json.JSONDecodeError, ValueError):
            while True:
                raw = await reader.readline()
                if not raw:
                    break
                msg = json.loads(raw.decode("utf-8"))
                cmds.append(msg["command"])
                if (rid := msg.get("request_id")) is not None:
                    reply = {"request_id": rid, "error": "success", "data": None}
                    writer.write((json.dumps(reply) + "\n").encode("utf-8"))
                    await writer.drain()
        writer.close()

    server = await asyncio.start_unix_server(handle_client, path=str(sock))
    try:
        settings = Settings()
        settings.player.autoplay = True
        mpv = MpvController(settings, socket_path=sock, spawn_process=False)

        radio_track = Track("vid_radio_1", "Autoplay Followup", duration=210)
        radio_calls: list[str] = []

        async def fake_radio(seed_vid: str) -> list[Track]:
            radio_calls.append(seed_vid)
            return [sample_tracks()[0], radio_track]

        async with httpx.AsyncClient(
            transport=httpx.MockTransport(lambda r: httpx.Response(404))
        ) as client:
            lyr_svc = LyricsService(settings, catalog=FakeCatalog(), http=client)
            resolver = StreamResolver(
                runner=lambda vid: asyncio.sleep(0, result=f"https://stream.test/{vid}")
            )
            history = HistoryStore(settings)
            engine = PlayerEngine(
                settings, resolver, lyr_svc, history, mpv=mpv, radio_fn=fake_radio
            )

            await engine.play_track(sample_tracks()[0])
            await asyncio.sleep(0.04)

            # Live toggle loudness normalization adds and removes @musicknorm filter cleanly
            settings.player.normalize_audio = True
            mpv.apply_settings(settings)
            await asyncio.sleep(0.04)
            assert any(c[:2] == ["af", "add"] and "@musicknorm:" in str(c[2]) for c in cmds)
            settings.player.normalize_audio = False
            mpv.apply_settings(settings)
            await asyncio.sleep(0.04)
            assert ["af", "remove", "@musicknorm"] in cmds

            # Simulate EOF at end of queue -> Autoplay seamlessly queues and starts radio_track
            sw = writer_holder[0]
            sw.write((json.dumps({"event": "end-file", "reason": "eof"}) + "\n").encode("utf-8"))
            await sw.drain()
            await asyncio.sleep(0.08)

            assert radio_calls == ["vid1"]
            assert engine.state.autoplay_active is True
            assert engine.state.track is not None
            assert engine.state.track.video_id == "vid_radio_1"
            await engine.shutdown()
    finally:
        server.close()
        server.close_clients()
        await server.wait_closed()
