"""Tests for LRC parsing, bisect lookup, LRCLIB client, fallback, and opt-in disk cache."""

import httpx
import pytest

from musick.config import Settings
from musick.domain import ArtistRef, Track
from musick.paths import lyrics_cache_dir
from musick.services.lyrics import (
    LyricsService,
    active_line_index,
    parse_json3_captions,
    parse_lrc,
    parse_vtt_captions,
)
from tests.conftest import FakeCatalog


def test_parse_lrc_multi_tag_offset_and_bisect() -> None:
    raw = """
[ti:Test Song]
[offset:500]
[00:01.00][00:10.00]Chorus line
[00:05.250]Verse one
"""
    lyr = parse_lrc(raw)
    assert lyr.synced is True
    assert [(round(ln.time, 3), ln.text) for ln in lyr.lines] == [
        (1.5, "Chorus line"),
        (5.75, "Verse one"),
        (10.5, "Chorus line"),
    ]
    assert active_line_index(lyr, 0.5) is None
    assert active_line_index(lyr, 2.0) == 0
    assert active_line_index(lyr, 5.75) == 1
    assert active_line_index(lyr, 20.0) == 2


def test_parse_plain_lyrics_fallback() -> None:
    lyr = parse_lrc("Line A\nLine B\n")
    assert lyr.synced is False
    assert [ln.text for ln in lyr.lines] == ["Line A", "Line B"]
    assert active_line_index(lyr, 1.0) is None


@pytest.mark.asyncio
async def test_lyrics_service_lrclib_and_privacy_disk_cache() -> None:
    calls = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal calls
        calls += 1
        assert "cookie" not in {k.lower() for k in request.headers}
        if request.url.path == "/api/get":
            return httpx.Response(
                200,
                json={"syncedLyrics": "[00:01.00]I said ooh\n[00:05.00]Blinded by the lights"},
            )
        return httpx.Response(404)

    settings = Settings()  # cache_lyrics = False by default
    track = Track(
        "v_lyr1", "Blinding Lights (Official Audio)", (ArtistRef("The Weeknd"),), duration=200
    )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        svc = LyricsService(settings, catalog=FakeCatalog(), http=client)
        res = await svc.fetch(track)
        assert res.synced is True
        assert res.lines[1].text == "Blinded by the lights"
        assert calls == 1
        assert not lyrics_cache_dir().exists()

        # Enable opt-in disk cache
        settings.privacy.cache_lyrics = True
        svc.clear_cache()
        res2 = await svc.fetch(track)
        assert res2.synced is True
        assert (lyrics_cache_dir() / "v_lyr1.json").exists()

        # New service instance reads from disk cache without HTTP call
        svc2 = LyricsService(settings, catalog=FakeCatalog(), http=client)
        res3 = await svc2.fetch(track)
        assert len(res3.lines) == 2
        assert calls == 2


@pytest.mark.asyncio
async def test_lyrics_service_falls_back_to_ytmusic() -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(404)

    settings = Settings()
    track = Track("vid1", "Fallback Track", (ArtistRef("Artist"),))
    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        svc = LyricsService(settings, catalog=FakeCatalog(), http=client)
        res = await svc.fetch(track)
        assert res.source == "YouTube Music"
        assert res.synced is True


@pytest.mark.asyncio
async def test_lyrics_upgrades_plain_lrclib_to_synced_youtube_and_youtube_source_priority() -> None:
    lrclib_calls = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal lrclib_calls
        lrclib_calls += 1
        return httpx.Response(200, json={"plainLyrics": "Plain line 1\nPlain line 2"})

    settings = Settings()  # source = "auto"
    track = Track("vid1", "Track", (ArtistRef("Artist"),))
    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        svc = LyricsService(settings, catalog=FakeCatalog(), http=client)
        res = await svc.fetch(track)
        assert res.source == "YouTube Music"
        assert res.synced is True
        assert lrclib_calls >= 1

        # source = "youtube" queries YouTube Music first without touching LRCLIB
        prev_calls = lrclib_calls
        settings.lyrics.source = "youtube"
        svc.clear_cache()
        res_yt = await svc.fetch(track)
        assert res_yt.source == "YouTube Music"
        assert res_yt.synced is True
        assert lrclib_calls == prev_calls


def test_parse_youtube_vtt_and_json3_captions() -> None:
    vtt = """WEBVTT
Kind: captions
Language: en

1
00:00:01.500 --> 00:00:04.000
<i>First</i> line

00:00:05.250 --> 00:00:08.000
Second &amp; final line
"""
    lyr_vtt = parse_vtt_captions(vtt)
    assert lyr_vtt.synced is True
    assert lyr_vtt.source == "YouTube Captions"
    assert [(round(ln.time, 2), ln.text) for ln in lyr_vtt.lines] == [
        (1.5, "First line"),
        (5.25, "Second & final line"),
    ]

    j3 = {
        "events": [
            {"tStartMs": 2000, "segs": [{"utf8": "Hello "}, {"utf8": "world"}]},
            {"tStartMs": 4500, "segs": [{"utf8": "\n"}]},
            {"tStartMs": 6000, "segs": [{"utf8": "Next line"}]},
        ]
    }
    lyr_j3 = parse_json3_captions(j3)
    assert lyr_j3.synced is True
    assert [(round(ln.time, 1), ln.text) for ln in lyr_j3.lines] == [
        (2.0, "Hello world"),
        (6.0, "Next line"),
    ]


@pytest.mark.asyncio
async def test_lyrics_auto_upgrades_plain_to_captions_and_disk_cache_tracks_mode(
    monkeypatch,
) -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(200, json={"plainLyrics": "Plain only"})

    class NoYtmCatalog(FakeCatalog):
        async def yt_lyrics(self, video_id: str):
            return None

    settings = Settings()
    settings.privacy.cache_lyrics = True
    track = Track("v_cap1", "Song", (ArtistRef("Artist"),))

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        svc = LyricsService(settings, catalog=NoYtmCatalog(), http=client)

        async def fake_caps(_t: Track):
            return parse_vtt_captions("WEBVTT\n\n00:00:01.000 --> 00:00:03.000\nSynced cap\n")

        monkeypatch.setattr(svc, "_fetch_yt_captions", fake_caps)
        res = await svc.fetch(track)
        assert res.synced is True
        assert res.source == "YouTube Captions"

        # Switching source mode invalidates mismatched disk cache mode
        settings.lyrics.source = "lrclib"
        svc.clear_cache()
        res2 = await svc.fetch(track)
        assert res2.source == "LRCLIB"
        assert res2.synced is False


@pytest.mark.asyncio
async def test_lyrics_indonesian_translation_batch_cache_and_fallback() -> None:
    translate_calls = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal translate_calls
        assert "cookie" not in {k.lower() for k in request.headers}
        if "translate.googleapis.com" in str(request.url):
            translate_calls += 1
            q = request.url.params.get("q", "")
            lines = q.splitlines()
            if len(lines) == 2:
                return httpx.Response(
                    200, json=[[[f"{lines[0]} (ID)\n{lines[1]} (ID)", q]], None, "en"]
                )
            # Single-line fallback path
            return httpx.Response(200, json=[[[f"{q} (ID)", q]], None, "en"])
        if request.url.path == "/api/get":
            return httpx.Response(
                200,
                json={
                    "syncedLyrics": (
                        "[00:01.00]Hello world\n[00:05.00]Hello world\n[00:09.00]Good night"
                    )
                },
            )
        return httpx.Response(404)

    settings = Settings()
    settings.privacy.cache_lyrics = True
    track = Track("v_tr1", "Hello", (ArtistRef("Artist"),), duration=180)

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        svc = LyricsService(settings, catalog=FakeCatalog(), http=client)
        base = await svc.fetch(track)
        assert base.translation_lang == ""
        assert translate_calls == 0

        # Deduplicates repeated lines ("Hello world" x2) -> 1 batched call for 2 unique lines
        translated = await svc.translate_lyrics(base, video_id="v_tr1", target_lang="id")
        assert translated.translation_lang == "id"
        assert translate_calls == 1
        assert [ln.translation for ln in translated.lines] == [
            "Hello world (ID)",
            "Hello world (ID)",
            "Good night (ID)",
        ]
        assert [ln.time for ln in translated.lines] == [1.0, 5.0, 9.0]

        # Re-read from disk cache retains translation without repeating HTTP translation calls
        settings.lyrics.translate_to_id = True
        svc2 = LyricsService(settings, catalog=FakeCatalog(), http=client)
        cached = await svc2.fetch(track)
        assert cached.translation_lang == "id"
        assert cached.lines[2].translation == "Good night (ID)"
        assert translate_calls == 1

        # Already Indonesian lyrics skip network call completely
        id_base = parse_lrc("[00:01.00]Aku cinta kamu dan hati kita bersama selalu")
        id_res = await svc.translate_lyrics(id_base, target_lang="id")
        assert id_res.translation_lang == "id"
        assert id_res.lines[0].translation == ""
        assert translate_calls == 1
