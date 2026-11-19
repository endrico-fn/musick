"""Tests for domain models and ytmusicapi dict parsers."""

import pytest

from musick.domain import (
    AlbumSummary,
    ArtistSummary,
    PlaylistSummary,
    Thumb,
    Track,
    best_thumb_url,
    format_duration,
)
from musick.services.catalog import YTMusicCatalog
from musick.services.parsers import (
    parse_album,
    parse_artist,
    parse_home_feed,
    parse_playlist,
    parse_search,
    parse_yt_lyrics,
)


def test_thumb_upscale_and_duration() -> None:
    thumbs = (Thumb("https://lh3.googleusercontent.com/abc=w60-h60-l90-rj", 60, 60),)
    assert (
        best_thumb_url(thumbs, px=300) == "https://lh3.googleusercontent.com/abc=w300-h300-l90-rj"
    )
    assert format_duration(200) == "3:20"
    assert format_duration(3661) == "1:01:01"
    assert format_duration(None) == "--:--"


def test_parse_search_variants() -> None:
    raw = [
        {
            "category": "Top result",
            "resultType": "artist",
            "artists": [{"name": "Lana Del Rey", "id": "UC_LDR"}],
            "subscribers": "267M",
        },
        {
            "resultType": "song",
            "videoId": "v1",
            "title": "Song 1",
            "artists": None,
            "duration": "3:10",
        },
        {"resultType": "album", "browseId": "MPRE1", "title": "Alb 1", "year": "2020"},
        {"resultType": "artist", "browseId": "UC1", "artist": "Art 1", "subscribers": "10M"},
        {
            "resultType": "playlist",
            "browseId": "VLPL1",
            "title": "PL 1",
            "author": [{"name": "Bob"}],
            "itemCount": 12,
        },
    ]
    parsed = parse_search(raw)
    assert len(parsed) == 5
    assert isinstance(parsed[0], ArtistSummary)
    assert parsed[0].name == "Lana Del Rey"
    assert isinstance(parsed[1], Track)
    assert parsed[1].artist_names == "Lana Del Rey"
    assert parsed[1].duration == 190
    assert isinstance(parsed[2], AlbumSummary)
    assert parsed[2].year == "2020"
    assert isinstance(parsed[3], ArtistSummary)
    assert parsed[3].name == "Art 1"
    assert isinstance(parsed[4], PlaylistSummary)
    assert parsed[4].author == "Bob"


def test_parse_album_inherits_cover_and_artist_and_playlist() -> None:
    alb = parse_album(
        "MPRE1",
        {
            "title": "After Hours",
            "year": "2020",
            "artists": [{"name": "The Weeknd", "id": "UC1"}],
            "thumbnails": [{"url": "https://img/alb.jpg", "width": 544, "height": 544}],
            "tracks": [{"videoId": "v1", "title": "Alone Again", "duration_seconds": 250}],
        },
    )
    assert alb.tracks[0].cover_url == "https://img/alb.jpg"
    assert alb.tracks[0].artist_names == "The Weeknd"

    art = parse_artist(
        "UC1",
        {
            "name": "The Weeknd",
            "monthlyListeners": "98M",
            "songs": {
                "results": [{"videoId": "v1", "title": "Blinding Lights", "duration": "3:20"}]
            },
            "albums": {"results": [{"browseId": "MPRE1", "title": "After Hours"}], "params": "P1"},
        },
    )
    assert art.listeners == "98M"
    assert art.albums_params == "P1"

    pl = parse_playlist(
        "PL1",
        {
            "title": "Chill",
            "trackCount": 2,
            "tracks": [{"videoId": "v2", "title": "Lofi", "length": "2:05"}],
        },
    )
    assert pl.tracks[0].duration == 125

    feed = parse_home_feed(
        {
            "new_releases": [{"browseId": "MPRE9", "title": "Dawn FM"}],
            "trending": {"playlist": "PL_TR"},
        },
        {
            "artists": [{"browseId": "UC9", "title": "Daft Punk", "subscribers": "20M"}],
            "videos": [{"playlistId": "PL9", "title": "Top 100"}],
        },
    )
    assert len(feed.featured) == 1
    assert len(feed.artists) == 1
    assert len(feed.playlists) == 2

    lyr = parse_yt_lyrics(
        {"hasTimestamps": True, "source": "YTM", "lyrics": [{"text": "Hi", "start_time": 1500}]}
    )
    assert lyr
    assert lyr.synced
    assert lyr.lines[0].time == 1.5


@pytest.mark.asyncio
async def test_catalog_artist_expands_top_songs_and_all_search_merges_songs() -> None:
    class MockYT:
        def get_artist(self, browse_id: str):
            return {
                "name": "Tinashe",
                "channelId": browse_id,
                "monthlyListeners": "17.9M",
                "songs": {
                    "browseId": "VLPL_TOP",
                    "results": [
                        {"videoId": "v1", "title": "No Broke Boys", "views": "45M plays"},
                    ],
                },
                "videos": {
                    "results": [
                        {"videoId": "mv1", "title": "2 On (Official Video)", "views": "150M"}
                    ]
                },
            }

        def get_playlist(self, pid: str, limit: int = 200):
            assert pid == "PL_TOP"
            return {
                "id": pid,
                "title": "Top songs",
                "tracks": [
                    {
                        "videoId": f"v{i}",
                        "title": f"Song {i}",
                        "artists": [{"name": "Tinashe", "id": "UC_T"}],
                        "album": {"name": "Aquarius", "id": "MPRE_AQ"},
                        "duration": "3:20",
                    }
                    for i in range(1, 16)
                ],
            }

        def search(self, query: str, filter=None, limit: int = 30):
            if filter == "songs":
                return [
                    {
                        "resultType": "song",
                        "videoId": "v_song",
                        "title": "2 On",
                        "album": {"name": "Aquarius", "id": "MPRE_AQ"},
                        "duration": "3:47",
                    }
                ]
            return [
                {
                    "resultType": "video",
                    "videoId": "v_vid",
                    "title": "2 On (Slowed)",
                    "duration": "4:10",
                },
                {"resultType": "video", "videoId": "v_song", "title": "2 On", "duration": "3:47"},
            ]

    cat = YTMusicCatalog(client=MockYT())
    art = await cat.artist("UC_T")
    assert len(art.songs) == 15
    assert art.songs[0].duration == 200
    assert art.songs[0].plays == "45M"
    assert art.songs[0].album_id == "MPRE_AQ"
    assert len(art.videos) == 1

    res = await cat.search("all", "2 on", limit=20)
    vids = [t.video_id for t in res if isinstance(t, Track)]
    assert vids.count("v_song") == 1
    assert "v_vid" in vids
    merged_song = next(t for t in res if isinstance(t, Track) and t.video_id == "v_song")
    assert merged_song.album == "Aquarius"
    assert merged_song.video_type == "MUSIC_VIDEO_TYPE_ATV"

    cat.set_region("US")
    assert cat._region == "US"
    assert cat._cache.get(("ID", ("artist", "UC_T"))) is None


@pytest.mark.asyncio
async def test_artist_accuracy_portrait_crop_plays_playlists_and_mpad_expansion() -> None:
    from musick.services.parsers import (
        extract_artist_playlists_from_browse,
        extract_plays_from_browse,
    )

    portrait = (Thumb("https://yt3.googleusercontent.com/art=w540-h225-p-l90-rj", 540, 225),)
    assert (
        best_thumb_url(portrait, px=300)
        == "https://yt3.googleusercontent.com/art=w300-h300-p-l90-rj"
    )

    parsed_art = parse_artist(
        "UC_CANONICAL",
        {
            "name": "Ariana Grande",
            "channelId": "UC_CHANNEL_DIFFERENT",
            "monthlyListeners": "227M",
            "subscribers": "58.4M",
            "views": "33,841,221,053 views",
            "shuffleId": "RDAO_SHF",
            "radioId": "RDEM_RAD",
            "albums": {
                "browseId": "MPADUC_CANONICAL",
                "params": "P_ALB",
                "results": [{"browseId": "MPRE_A1", "title": "Alb 1", "year": "2024"}],
            },
            "singles": {
                "browseId": "MPADUC_CANONICAL",
                "params": "P_SNG",
                "results": [{"browseId": "MPRE_S1", "title": "Sng 1", "year": "2025"}],
            },
            "videos": {
                "browseId": "VLOLAK_VID",
                "results": [{"videoId": "mv1", "title": "MV 1", "views": "120M"}],
            },
        },
    )
    assert parsed_art.summary.browse_id == "UC_CANONICAL"
    assert parsed_art.listeners == "227M"
    assert parsed_art.subscribers == "58.4M"
    assert parsed_art.views == "33,841,221,053"
    assert parsed_art.albums_browse_id == "MPADUC_CANONICAL"
    assert parsed_art.singles_browse_id == "MPADUC_CANONICAL"
    assert parsed_art.videos_playlist_id == "VLOLAK_VID"
    assert parsed_art.shuffle_id == "RDAO_SHF"

    song_row = {
        "musicResponsiveListItemRenderer": {
            "playlistItemData": {"videoId": "s1"},
            "flexColumns": [
                {
                    "musicResponsiveListItemFlexColumnRenderer": {
                        "text": {"runs": [{"text": "Song 1"}]}
                    }
                },
                {
                    "musicResponsiveListItemFlexColumnRenderer": {
                        "text": {"runs": [{"text": "301M plays"}]}
                    }
                },
            ],
        }
    }
    feat_card = {
        "musicTwoRowItemRenderer": {
            "title": {
                "runs": [
                    {
                        "text": "Presenting Artist",
                        "navigationEndpoint": {"browseEndpoint": {"browseId": "VLRDCLAK1"}},
                    }
                ]
            },
            "subtitle": {
                "runs": [{"text": "Playlist"}, {"text": " • "}, {"text": "YouTube Music"}]
            },
        }
    }
    own_card = {
        "musicTwoRowItemRenderer": {
            "title": {
                "runs": [
                    {
                        "text": "positions",
                        "navigationEndpoint": {"browseEndpoint": {"browseId": "VLPL123"}},
                    }
                ]
            },
            "subtitle": {
                "runs": [
                    {"text": "Playlist"},
                    {"text": " • "},
                    {"text": "Ariana Grande"},
                    {"text": " • "},
                    {"text": "19M views"},
                ]
            },
        }
    }
    raw_browse = {
        "contents": {
            "singleColumnBrowseResultsRenderer": {
                "tabs": [
                    {
                        "tabRenderer": {
                            "content": {
                                "sectionListRenderer": {
                                    "contents": [
                                        {"musicShelfRenderer": {"contents": [song_row]}},
                                        {
                                            "musicCarouselShelfRenderer": {
                                                "header": {
                                                    "musicCarouselShelfBasicHeaderRenderer": {
                                                        "title": {"runs": [{"text": "Featured on"}]}
                                                    }
                                                },
                                                "contents": [feat_card],
                                            }
                                        },
                                        {
                                            "musicCarouselShelfRenderer": {
                                                "header": {
                                                    "musicCarouselShelfBasicHeaderRenderer": {
                                                        "title": {
                                                            "runs": [
                                                                {"text": "Playlists by Artist"}
                                                            ]
                                                        }
                                                    }
                                                },
                                                "contents": [own_card],
                                            }
                                        },
                                    ]
                                }
                            }
                        }
                    }
                ]
            }
        }
    }
    assert extract_plays_from_browse(raw_browse) == {"s1": "301M"}
    pls = extract_artist_playlists_from_browse(raw_browse)
    assert [(p.browse_id, p.title, p.author, p.count) for p in pls] == [
        ("VLRDCLAK1", "Presenting Artist", "YouTube Music", None),
        ("VLPL123", "positions", "Ariana Grande", "19M views"),
    ]

    class FullArtistMock:
        def get_artist(self, browse_id: str):
            assert browse_id == "UC_CANONICAL"
            return {
                "name": "Artist",
                "channelId": "UC_CHANNEL_DIFFERENT",
                "songs": {
                    "browseId": "VLPL_TOP",
                    "results": [{"videoId": "s1", "title": "Song 1", "views": "301M plays"}],
                },
                "albums": {
                    "browseId": "MPADUC_CANONICAL",
                    "params": "P_ALB",
                    "results": [{"browseId": "MPRE_A1", "title": "Alb 1", "year": "2024"}],
                },
                "singles": {
                    "browseId": "MPADUC_CANONICAL",
                    "params": "P_SNG",
                    "results": [{"browseId": "MPRE_S1", "title": "Sng 1", "year": "2025"}],
                },
                "videos": {
                    "browseId": "VLPL_VID",
                    "results": [{"videoId": "mv1", "title": "MV 1", "views": "120M"}],
                },
            }

        def get_playlist(self, pid: str, limit: int = 200):
            if pid == "PL_TOP":
                return {
                    "id": pid,
                    "tracks": [{"videoId": "s1", "title": "Song 1", "duration": "3:18"}],
                }
            assert pid == "PL_VID"
            return {
                "id": pid,
                "tracks": [
                    {"videoId": "mv1", "title": "MV 1", "duration": "4:00"},
                    {"videoId": "mv2", "title": "MV 2", "duration": "3:30"},
                ],
            }

        def get_artist_albums(self, channel_id: str, params: str):
            assert channel_id == "MPADUC_CANONICAL"
            if params == "P_ALB":
                return [
                    {"browseId": f"MPRE_A{i}", "title": f"Alb {i}", "year": "2024"}
                    for i in range(1, 4)
                ]
            return [
                {"browseId": f"MPRE_S{i}", "title": f"Sng {i}", "year": "2025"}
                for i in range(1, 3)
            ]

    cat = YTMusicCatalog(client=FullArtistMock())
    full = await cat.artist("UC_CANONICAL")
    assert full.summary.browse_id == "UC_CANONICAL"
    assert len(full.albums) == 3
    assert len(full.singles) == 2
    assert len(full.videos) == 2
    assert full.videos[0].plays == "120M"
    assert full.videos[0].duration == 240
    assert full.songs[0].plays == "301M"
