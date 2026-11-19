"""Shared pytest fixtures: isolated XDG paths, safe terminal image init, and fake services."""

from __future__ import annotations

import io
from pathlib import Path

import pytest
from PIL import Image as PILImage

from musick.domain import (
    Album,
    AlbumSummary,
    Artist,
    ArtistRef,
    ArtistSummary,
    HomeFeed,
    LyricLine,
    Lyrics,
    Playlist,
    PlaylistSummary,
    SearchItem,
    Thumb,
    Track,
)
from musick.services.catalog import Catalog
from musick.ui.widgets.cover import init_terminal_image_support


@pytest.fixture(autouse=True)
def _isolated_xdg(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path / "cache"))
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    init_terminal_image_support()


def make_png_bytes(color: tuple[int, int, int] = (38, 217, 180)) -> bytes:
    buf = io.BytesIO()
    im = PILImage.new("RGB", (32, 32), color)
    im.save(buf, format="PNG")
    return buf.getvalue()


def sample_tracks() -> tuple[Track, ...]:
    th = (Thumb("https://example.test/cover.png", 300, 300),)
    return (
        Track(
            video_id="vid1",
            title="Blinding Lights",
            artists=(ArtistRef("The Weeknd", "UC1"),),
            album="After Hours",
            album_id="MPRE1",
            duration=200,
            thumbs=th,
            plays="1.4B",
        ),
        Track(
            video_id="vid2",
            title="Starboy",
            artists=(ArtistRef("The Weeknd", "UC1"),),
            album="Starboy",
            album_id="MPRE2",
            duration=230,
            thumbs=th,
            plays="1.1B",
        ),
        Track(
            video_id="vid3",
            title="Save Your Tears",
            artists=(ArtistRef("The Weeknd", "UC1"),),
            album="After Hours",
            album_id="MPRE1",
            duration=215,
            thumbs=th,
            plays="920M",
        ),
    )


class FakeCatalog(Catalog):
    def __init__(self) -> None:
        self.tracks = sample_tracks()
        self.artist_obj = Artist(
            summary=ArtistSummary(
                "UC1", "The Weeknd", "98.4M monthly listeners", self.tracks[0].thumbs
            ),
            description="Canadian singer, songwriter and producer.",
            listeners="98.4M monthly",
            songs=self.tracks,
            albums=(
                AlbumSummary(
                    "MPRE1",
                    "After Hours",
                    "2020",
                    "Album",
                    (ArtistRef("The Weeknd", "UC1"),),
                    self.tracks[0].thumbs,
                ),
                AlbumSummary(
                    "MPRE2",
                    "Starboy",
                    "2016",
                    "Album",
                    (ArtistRef("The Weeknd", "UC1"),),
                    self.tracks[0].thumbs,
                ),
            ),
            singles=(
                AlbumSummary(
                    "MPRE3",
                    "Take My Breath",
                    "2021",
                    "Single",
                    (ArtistRef("The Weeknd", "UC1"),),
                    self.tracks[0].thumbs,
                ),
            ),
            playlists=(
                PlaylistSummary(
                    "PL1", "The Weeknd Essentials", "YouTube Music", "30", self.tracks[0].thumbs
                ),
            ),
            related=(ArtistSummary("UC2", "Dua Lipa", "71M", self.tracks[0].thumbs),),
            albums_params="ALB_PARAMS",
            singles_params="SNG_PARAMS",
            radio_id="RD1",
        )
        self.album_obj = Album(
            summary=self.artist_obj.albums[0],
            tracks=self.tracks,
            description="Fourth studio album.",
            duration_text="56 min",
        )
        self.playlist_obj = Playlist(
            summary=PlaylistSummary(
                "PL_COMM", "Lofi Beats to Study", "@chillhop_fan", "120", self.tracks[0].thumbs
            ),
            tracks=self.tracks,
            description="Relaxed beats to study and focus.",
            views="1.2M",
            duration_text="6 h 12 min",
            year="2026",
            total=120,
        )

    async def home_feed(self, region: str = "ID") -> HomeFeed:
        return HomeFeed(
            featured=self.artist_obj.albums,
            artists=(self.artist_obj.summary, ArtistSummary("UC2", "Dua Lipa", "71.2M")),
            playlists=(self.playlist_obj.summary,),
        )

    async def search(self, kind: str, query: str, limit: int = 30) -> tuple[SearchItem, ...]:
        if kind == "artists":
            return (self.artist_obj.summary,)
        if kind == "albums":
            return self.artist_obj.albums
        if kind in ("playlists", "community"):
            return (self.playlist_obj.summary,)
        return (self.artist_obj.summary, *self.tracks)

    async def suggestions(self, query: str) -> list[str]:
        return [f"{query} official", f"{query} blinding lights"]

    async def artist(self, browse_id: str) -> Artist:
        return self.artist_obj

    async def artist_releases(
        self, browse_id: str, params: str | None, kind: str = "Album"
    ) -> tuple[AlbumSummary, ...]:
        return self.artist_obj.albums if kind == "Album" else self.artist_obj.singles

    async def album(self, browse_id: str) -> Album:
        return self.album_obj

    async def playlist(self, playlist_id: str, limit: int = 200) -> Playlist:
        if limit > 50:
            extra = Track("vid4", "Quiet Rain", (ArtistRef("Idealism"),), duration=175)
            return Playlist(
                summary=self.playlist_obj.summary,
                tracks=(*self.tracks, extra),
                description=self.playlist_obj.description,
                views=self.playlist_obj.views,
                duration_text=self.playlist_obj.duration_text,
                year=self.playlist_obj.year,
                total=4,
            )
        return self.playlist_obj

    async def radio(
        self, video_id: str | None = None, playlist_id: str | None = None, limit: int = 25
    ) -> tuple[Track, ...]:
        return self.tracks

    async def yt_lyrics(self, video_id: str) -> Lyrics | None:
        return Lyrics(
            (LyricLine(0.0, "Fallback line one"), LyricLine(5.0, "Fallback line two")),
            synced=True,
            source="YouTube Music",
        )
