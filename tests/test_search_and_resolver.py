"""Tests for smart search prefixes, fuzzy opt-in history, stream TTL cache, and image cache."""

import httpx
import pytest

from musick.config import Settings
from musick.domain import ArtistRef, ArtistSummary, Track
from musick.paths import history_file, image_cache_dir
from musick.services.images import ImageService
from musick.services.resolver import StreamResolver, parse_stream_expiry
from musick.services.search import (
    HistoryStore,
    SearchService,
    extract_youtube_target,
    parse_query_prefix,
    rank_search_items,
)
from tests.conftest import FakeCatalog, make_png_bytes


def test_parse_query_prefix() -> None:
    assert parse_query_prefix("a: the weeknd") == ("artists", "the weeknd")
    assert parse_query_prefix("al: after hours") == ("albums", "after hours")
    assert parse_query_prefix("pl: lofi beats") == ("playlists", "lofi beats")
    assert parse_query_prefix("c: synthwave") == ("community", "synthwave")
    assert parse_query_prefix("s: starboy") == ("songs", "starboy")
    assert parse_query_prefix("just a song") == ("all", "just a song")


@pytest.mark.asyncio
async def test_history_privacy_and_fuzzy_autocomplete() -> None:
    settings = Settings()  # save_history = False
    hist = HistoryStore(settings)
    hist.record_query("the weeknd")
    assert hist.queries == []
    assert not history_file().exists()

    # Opt-in history persistence + fuzzy matching
    settings.privacy.save_history = True
    hist2 = HistoryStore(settings)
    hist2.record_query("the weeknd")
    hist2.record_query("daft punk")
    assert history_file().exists()

    svc = SearchService(FakeCatalog(), hist2, settings)
    sugg = await svc.autocomplete("weeknd")
    assert sugg[0].text == "the weeknd"
    assert sugg[0].from_history is True
    assert any(s.text == "weeknd official" for s in sugg)

    hist2.clear()
    assert not history_file().exists()


@pytest.mark.asyncio
async def test_stream_resolver_expiry_cache_and_dedup() -> None:
    now = 1_700_000_000.0
    calls = 0

    async def fake_runner(vid: str) -> str:
        nonlocal calls
        calls += 1
        return f"https://rr1.googlevideo.com/videoplayback?id={vid}&expire={int(now + 1000)}"

    res = StreamResolver(clock=lambda: now, runner=fake_runner)
    assert parse_stream_expiry(f"https://x/?expire={int(now + 1000)}", now=now) == now + 880.0

    # Concurrent calls share one yt-dlp invocation
    import asyncio

    a, b = await asyncio.gather(res.resolve("v1"), res.resolve("v1"))
    assert a.url == b.url
    assert calls == 1

    # Advance past expiry -> re-resolves
    now += 900.0
    await res.resolve(Track("v1", "Track"))
    assert calls == 2


@pytest.mark.asyncio
async def test_image_service_memory_and_opt_in_disk_cache() -> None:
    png = make_png_bytes()
    hits = 0

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal hits
        hits += 1
        return httpx.Response(200, content=png)

    settings = Settings()  # cache_images = False
    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        svc = ImageService(settings, http=client)
        im = await svc.fetch("https://img.test/a.png")
        assert im is not None
        assert im.size == (32, 32)
        assert hits == 1
        assert not image_cache_dir().exists()

        # Memory hit
        im2 = await svc.fetch("https://img.test/a.png")
        assert im2 is not None
        assert hits == 1

        # Opt-in disk cache
        settings.privacy.cache_images = True
        svc.clear_cache()
        im3 = await svc.fetch("https://img.test/a.png")
        assert im3 is not None
        assert hits == 2
        assert image_cache_dir().exists()

        svc2 = ImageService(settings, http=client)
        im4 = await svc2.fetch("https://img.test/a.png")
        assert im4 is not None
        assert hits == 2


@pytest.mark.asyncio
async def test_image_service_shield_against_caller_cancellation() -> None:
    import asyncio

    png = make_png_bytes()
    gate = asyncio.Event()

    async def slow_handler(request: httpx.Request) -> httpx.Response:
        await gate.wait()
        return httpx.Response(200, content=png)

    settings = Settings()
    async with httpx.AsyncClient(transport=httpx.MockTransport(slow_handler)) as client:
        svc = ImageService(settings, http=client)
        t1 = asyncio.create_task(svc.fetch("https://img.test/slow.png"))
        t2 = asyncio.create_task(svc.fetch("https://img.test/slow.png"))
        await asyncio.sleep(0.02)
        t1.cancel()
        with pytest.raises(asyncio.CancelledError):
            await t1
        gate.set()
        im = await t2
        assert im is not None
        assert im.size == (32, 32)


def test_smart_search_ranking_and_url_detection() -> None:
    # Mirrors real-world "2 on" search where unofficial slowed/reverb/remix videos pollute results
    items = (
        Track(
            "vid_mv",
            "2 On (Official Video) (feat. ScHoolboy Q)",
            (ArtistRef("Tinashe"),),
            duration=231,
            video_type="MUSIC_VIDEO_TYPE_OMV",
        ),
        Track(
            "vid_sr",
            "2 on- tinashe (without schoolboy q/slowed reverb)",
            (ArtistRef("cyberysz"),),
            duration=223,
            video_type="MUSIC_VIDEO_TYPE_UGC",
        ),
        Track(
            "vid_rmx",
            "P110 - Aystar Ft. Safone - 2 On (Remix) [Music Video]",
            (ArtistRef("P110"),),
            duration=180,
            video_type="MUSIC_VIDEO_TYPE_UGC",
        ),
        Track(
            "vid_tt",
            "Tinashe - 2 On (TikTok Remix) slowed + reverb Lyrics",
            (ArtistRef("Tinashe"),),
            duration=275,
            video_type="MUSIC_VIDEO_TYPE_UGC",
        ),
        Track(
            "vid_official",
            "2 On (feat. ScHoolboy Q)",
            (ArtistRef("Tinashe", "UC_T"),),
            album="Aquarius",
            album_id="MPRE_AQ",
            duration=227,
            video_type="MUSIC_VIDEO_TYPE_ATV",
        ),
        ArtistSummary("UC_T", "Tinashe", "Artist"),
    )
    ranked = rank_search_items(items, "2 on", kind="all")
    assert isinstance(ranked[0], Track)
    assert ranked[0].video_id == "vid_official"
    # Unofficial slowed/reverb/tiktok/remix must be pushed behind official song & official video
    top3_ids = [getattr(x, "video_id", getattr(x, "browse_id", "")) for x in ranked[:3]]
    assert "vid_sr" not in top3_ids
    assert "vid_tt" not in top3_ids
    assert "vid_rmx" not in top3_ids

    # When the user explicitly asks for slowed reverb, do NOT penalize it
    ranked_sr = rank_search_items(items, "2 on slowed reverb", kind="all")
    assert getattr(ranked_sr[0], "video_id", "") in ("vid_sr", "vid_tt")

    # Direct YouTube / YouTube Music URL extraction
    assert extract_youtube_target(
        "https://music.youtube.com/watch?v=dQw4w9WgXcQ&feature=share"
    ) == ("track", "dQw4w9WgXcQ")
    assert extract_youtube_target("https://youtu.be/abc123XYZ") == ("track", "abc123XYZ")
    assert extract_youtube_target("https://music.youtube.com/playlist?list=PLxyz123") == (
        "playlist",
        "PLxyz123",
    )
    assert extract_youtube_target("https://music.youtube.com/browse/MPREb_123") == (
        "album",
        "MPREb_123",
    )
    assert extract_youtube_target("https://music.youtube.com/channel/UC12345") == (
        "artist",
        "UC12345",
    )
    assert extract_youtube_target("https://www.youtube.com/shorts/shortVid1") == (
        "track",
        "shortVid1",
    )
    assert extract_youtube_target("2 on tinashe") is None

    # Word-boundary noise check: "Recovery" must NOT be penalized for containing "cover"
    wb_items = (
        Track(
            "v_cov",
            "Recovery (Acoustic Cover)",
            (ArtistRef("Someone"),),
            video_type="MUSIC_VIDEO_TYPE_UGC",
        ),
        Track(
            "v_orig",
            "Recovery",
            (ArtistRef("James Arthur"),),
            album="Back from the Edge",
            video_type="MUSIC_VIDEO_TYPE_ATV",
        ),
    )
    wb_ranked = rank_search_items(wb_items, "recovery", kind="all")
    assert isinstance(wb_ranked[0], Track)
    assert wb_ranked[0].video_id == "v_orig"

    # Exact artist search ranks ArtistSummary #1 over tracks by that artist
    art_items = (
        Track(
            "v_2on",
            "2 On",
            (ArtistRef("Tinashe", "UC_T"),),
            album="Aquarius",
            video_type="MUSIC_VIDEO_TYPE_ATV",
        ),
        ArtistSummary("UC_T", "Tinashe", "Artist"),
    )
    art_ranked = rank_search_items(art_items, "tinashe", kind="all")
    assert isinstance(art_ranked[0], ArtistSummary)

    # Real song titles containing "Without" must NOT be flagged as "(without ...)" noise
    wo_items = (
        Track(
            "v_rmx",
            "Lose Yourself (Remix)",
            (ArtistRef("Eminem"),),
            video_type="MUSIC_VIDEO_TYPE_UGC",
        ),
        Track(
            "v_wo",
            "Without Me",
            (ArtistRef("Eminem"),),
            album="The Eminem Show",
            video_type="MUSIC_VIDEO_TYPE_ATV",
        ),
    )
    wo_ranked = rank_search_items(wo_items, "eminem", kind="all")
    assert isinstance(wo_ranked[0], Track)
    assert wo_ranked[0].video_id == "v_wo"

    # Diacritic folding & scheme-less YouTube URLs
    diacritic_items = (
        Track(
            "v_lat",
            "Rosalía",
            (ArtistRef("Rosalía"),),
            album="Motomami",
            video_type="MUSIC_VIDEO_TYPE_ATV",
        ),
    )
    lat_ranked = rank_search_items(diacritic_items, "rosalia", kind="all")
    assert isinstance(lat_ranked[0], Track)
    assert lat_ranked[0].video_id == "v_lat"
    assert extract_youtube_target("music.youtube.com/watch?v=dQw4w9WgXcQ") == (
        "track",
        "dQw4w9WgXcQ",
    )


@pytest.mark.asyncio
async def test_search_service_preserves_seed_track_from_url_when_radio_omits_it() -> None:
    cat = FakeCatalog()
    settings = Settings()
    svc = SearchService(cat, HistoryStore(settings), settings)
    res = await svc.search("all", "https://youtu.be/SEED999")
    assert isinstance(res[0], Track)
    assert res[0].video_id == "SEED999"
    assert len(res) >= 2


@pytest.mark.asyncio
async def test_stream_resolver_parses_rich_metadata_and_drc_free_flags(monkeypatch) -> None:
    import asyncio

    captured_args: list[tuple[str, ...]] = []

    class FakeProc:
        returncode = 0

        async def communicate(self):
            return (
                b"opus\t162.4\t48000\nhttps://rr1.googlevideo.com/videoplayback?expire=9999999999\n",
                b"",
            )

    async def fake_exec(*args, **kwargs):
        captured_args.append(tuple(str(a) for a in args))
        return FakeProc()

    monkeypatch.setattr(asyncio, "create_subprocess_exec", fake_exec)
    res = StreamResolver()
    stream = await res.resolve("vid_opus")
    assert stream.codec == "opus"
    assert stream.bitrate == 162
    assert stream.sample_rate == 48000
    assert stream.quality_label == "opus 162k"
    assert any("format_id!*=drc" in a for a in captured_args[0])
    assert "--format-sort" in captured_args[0]


def test_search_lyric_semantic_parity_and_video_prefix() -> None:
    from musick.domain import AlbumSummary

    assert parse_query_prefix("v: thriller") == ("videos", "thriller")
    assert parse_query_prefix("video: starboy") == ("videos", "starboy")

    # Mirrors real-world "you say i'm crazy" YTMusic search (lyric match -> Sam Smith)
    ytm_items = (
        Track(
            "v_sam_omv",
            "I'm Not The Only One (Official Video)",
            (ArtistRef("Sam Smith", "UC_SAM"),),
            duration=280,
            plays="1.7B",
            video_type="MUSIC_VIDEO_TYPE_OMV",
        ),
        Track(
            "v_sam_atv",
            "I'm Not The Only One",
            (ArtistRef("Sam Smith", "UC_SAM"),),
            album="In The Lonely Hour",
            album_id="MPRE_SAM",
            duration=239,
            plays="2.9B",
            video_type="MUSIC_VIDEO_TYPE_ATV",
        ),
        Track(
            "v_conan",
            "Maniac",
            (ArtistRef("Conan Gray", "UC_CG"),),
            album="Kid Krow",
            duration=186,
            plays="337M",
            video_type="MUSIC_VIDEO_TYPE_ATV",
        ),
        ArtistSummary("UC_SAM", "Sam Smith", "58M"),
        Track(
            "v_sam_lyr",
            "Sam Smith - I'm Not The Only One (Lyrics)",
            (ArtistRef("Dan Music"),),
            duration=240,
            plays="45M",
            video_type="MUSIC_VIDEO_TYPE_VIDEO",
        ),
        Track(
            "v_gracie",
            "I Love You, I'm Sorry",
            (ArtistRef("Gracie Abrams", "UC_GA"),),
            album="The Secret of Us",
            duration=157,
            plays="204M",
            video_type="MUSIC_VIDEO_TYPE_ATV",
        ),
        # Obscure exact-literal-title songs / singles injected later must not hijack top results
        Track(
            "v_liv",
            "You Say I'm Crazy",
            (ArtistRef("liv."),),
            video_type="MUSIC_VIDEO_TYPE_ATV",
        ),
        Track(
            "v_rhythm",
            "You Say I'm Crazy",
            (ArtistRef("The Rhythm Slaves"),),
            video_type="MUSIC_VIDEO_TYPE_ATV",
        ),
        AlbumSummary("MPRE_2026", "You say i'm crazy", "2026", "Single", (ArtistRef("Indie"),)),
    )

    ranked = rank_search_items(ytm_items, "you say i'm crazy", kind="all")
    assert isinstance(ranked[0], Track)
    assert ranked[0].video_id == "v_sam_atv"
    top4_ids = [getattr(x, "video_id", getattr(x, "browse_id", "")) for x in ranked[:4]]
    assert "v_liv" not in top4_ids
    assert "v_rhythm" not in top4_ids
    assert "MPRE_2026" not in top4_ids
    assert "v_conan" in top4_ids
    assert "v_sam_lyr" not in top4_ids


@pytest.mark.asyncio
async def test_stream_resolver_audio_quality_profiles_and_cache_invalidation(monkeypatch) -> None:
    import asyncio

    captured: list[tuple[str, ...]] = []

    class FakeProc:
        returncode = 0

        async def communicate(self):
            return (
                b"opus\t64.0\t48000\nhttps://rr1.googlevideo.com/videoplayback?expire=9999999999\n",
                b"",
            )

    async def fake_exec(*args, **kwargs):
        captured.append(tuple(str(a) for a in args))
        return FakeProc()

    monkeypatch.setattr(asyncio, "create_subprocess_exec", fake_exec)
    settings = Settings()
    res = StreamResolver(settings=settings)
    s1 = await res.resolve("q1")
    assert s1.quality == "best"
    assert res.get_cached("q1") is not None

    settings.player.audio_quality = "saver"
    res.set_quality("saver")
    assert res.get_cached("q1") is None
    s2 = await res.resolve("q1")
    assert s2.quality == "saver"
    assert "saver" in s2.quality_label
    assert any("abr<=80" in a for a in captured[1])
