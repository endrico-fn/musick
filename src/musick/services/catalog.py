"""Catalog service: async facade over ytmusicapi (anonymous, no account cookies)."""

from __future__ import annotations

import asyncio
import logging
import threading
import time
from collections import OrderedDict
from collections.abc import Callable
from dataclasses import replace
from typing import Any, Protocol

from musick.domain import (
    Album,
    AlbumSummary,
    Artist,
    ArtistRef,
    HomeFeed,
    Lyrics,
    Playlist,
    PlaylistSummary,
    SearchItem,
    Track,
    search_item_key,
)

from . import parsers

logger = logging.getLogger(__name__)


class CatalogError(RuntimeError):
    """Raised when an upstream catalog API or network call fails."""


CATALOG_ERRORS: tuple[type[Exception], ...] = (
    CatalogError,
    OSError,
    TimeoutError,
    ValueError,
    RuntimeError,
)

SEARCH_KINDS: dict[str, str | None] = {
    "all": None,
    "songs": "songs",
    "videos": "videos",
    "albums": "albums",
    "artists": "artists",
    "playlists": "playlists",
    "community": "community_playlists",
}


class Catalog(Protocol):
    async def home_feed(self, region: str = "ID") -> HomeFeed: ...
    async def search(self, kind: str, query: str, limit: int = 30) -> tuple[SearchItem, ...]: ...
    async def suggestions(self, query: str) -> list[str]: ...
    async def artist(self, browse_id: str) -> Artist: ...
    async def artist_releases(
        self, browse_id: str, params: str | None, kind: str
    ) -> tuple[AlbumSummary, ...]: ...
    async def album(self, browse_id: str) -> Album: ...
    async def playlist(self, playlist_id: str, limit: int = 200) -> Playlist: ...
    async def radio(
        self, video_id: str | None = None, playlist_id: str | None = None, limit: int = 25
    ) -> tuple[Track, ...]: ...
    async def yt_lyrics(self, video_id: str) -> Lyrics | None: ...


class TTLCache:
    """Tiny LRU+TTL cache for catalog responses (memory only)."""

    def __init__(
        self, ttl: float = 600.0, max_items: int = 128, clock: Callable[[], float] = time.monotonic
    ):
        self._ttl, self._max, self._clock = ttl, max_items, clock
        self._d: OrderedDict[Any, tuple[float, Any]] = OrderedDict()

    def get(self, key):
        hit = self._d.get(key)
        if not hit:
            return None
        if self._clock() - hit[0] > self._ttl:
            del self._d[key]
            return None
        self._d.move_to_end(key)
        return hit[1]

    def put(self, key, value) -> None:
        self._d[key] = (self._clock(), value)
        self._d.move_to_end(key)
        while len(self._d) > self._max:
            self._d.popitem(last=False)


class YTMusicCatalog:
    def __init__(self, client: Any | None = None, region: str = "ID"):
        self._client = client
        self._injected = client is not None
        self._region = region
        self._region_gen = 0
        self._cache = TTLCache()
        self._yt_lock = threading.RLock()
        self._tls = threading.local()

    def clear_cache(self) -> None:
        self._cache = TTLCache()

    def set_region(self, region: str) -> None:
        with self._yt_lock:
            if region != self._region:
                self._region = region
                self._region_gen += 1
                if not self._injected:
                    self._client = None
                self._cache = TTLCache()

    def _yt(self, region: str | None = None):
        if self._injected:
            return self._client
        target = region or self._region
        with self._yt_lock:
            gen = self._region_gen
        cached = getattr(self._tls, "entry", None)
        if cached is not None and cached[0] == gen and cached[1] == target:
            return cached[2]
        from ytmusicapi import (
            YTMusic,  # imported lazily: heavy import, network-free ctor
        )

        loc = "" if target == "ZZ" else target
        client = YTMusic(language="en", location=loc)
        self._tls.entry = (gen, target, client)
        return client

    async def _call(self, key, fn: Callable[[], Any]) -> Any:
        full_key = (self._region, key)
        cached = self._cache.get(full_key)
        if cached is not None:
            return cached

        def _locked() -> Any:
            from requests.exceptions import RequestException
            from ytmusicapi.exceptions import YTMusicError

            try:
                if self._injected:
                    with self._yt_lock:
                        return fn()
                return fn()
            except (
                YTMusicError,
                RequestException,
                OSError,
                ValueError,
                LookupError,
                TypeError,
            ) as exc:
                raise CatalogError(str(exc)) from exc

        result = await asyncio.to_thread(_locked)
        self._cache.put(full_key, result)
        return result

    async def search(self, kind: str, query: str, limit: int = 30) -> tuple[SearchItem, ...]:
        if kind == "all":
            all_res, song_res = await asyncio.gather(
                self._call(
                    ("search", "all", query, limit),
                    lambda: self._yt().search(query, filter=None, limit=limit),
                ),
                self._call(
                    ("search", "songs", query, min(20, limit)),
                    lambda: self._yt().search(query, filter="songs", limit=min(20, limit)),
                ),
                return_exceptions=True,
            )
            for res in (all_res, song_res):
                if isinstance(res, BaseException) and not isinstance(res, Exception):
                    raise res
            if isinstance(all_res, Exception) and isinstance(song_res, Exception):
                raise all_res
            parsed_all = parsers.parse_search([] if isinstance(all_res, Exception) else all_res)
            parsed_songs = parsers.parse_search([] if isinstance(song_res, Exception) else song_res)
            song_by_vid = {it.video_id: it for it in parsed_songs if isinstance(it, Track)}
            merged: list[SearchItem] = []
            seen: set[tuple[str, str]] = set()
            # Prefer official song metadata when the same videoId appears in both "all" and "songs"
            for it in (*parsed_all, *parsed_songs):
                if isinstance(it, Track) and it.video_id in song_by_vid:
                    s_it = song_by_vid[it.video_id]
                    if (not it.album and s_it.album) or s_it.video_type == "MUSIC_VIDEO_TYPE_ATV":
                        it = replace(
                            it,
                            title=s_it.title or it.title,
                            artists=s_it.artists or it.artists,
                            album=s_it.album or it.album,
                            album_id=s_it.album_id or it.album_id,
                            duration=s_it.duration or it.duration,
                            thumbs=s_it.thumbs or it.thumbs,
                            plays=it.plays or s_it.plays,
                            video_type=s_it.video_type or it.video_type,
                        )
                k = search_item_key(it)
                if k in seen:
                    continue
                seen.add(k)
                merged.append(it)
            return tuple(merged)

        flt = SEARCH_KINDS.get(kind)
        raw = await self._call(
            ("search", kind, query, limit),
            lambda: self._yt().search(query, filter=flt, limit=limit),
        )
        return parsers.parse_search(raw)

    async def suggestions(self, query: str) -> list[str]:
        if not query.strip():
            return []
        raw = await self._call(("sugg", query), lambda: self._yt().get_search_suggestions(query))
        return [s for s in raw if isinstance(s, str)]

    async def artist(self, browse_id: str) -> Artist:
        raw = await self._call(("artist", browse_id), lambda: self._yt().get_artist(browse_id))
        art = parsers.parse_artist(browse_id, raw)

        async def _expand_songs() -> Playlist | None:
            if not art.songs_playlist_id:
                return None
            try:
                return await self.playlist(art.songs_playlist_id, limit=150)
            except CATALOG_ERRORS:
                logger.debug(
                    "Failed expanding artist top songs playlist %s",
                    art.songs_playlist_id,
                    exc_info=True,
                )
                return None

        async def _expand_videos() -> Playlist | None:
            if not art.videos_playlist_id:
                return None
            try:
                return await self.playlist(art.videos_playlist_id, limit=100)
            except CATALOG_ERRORS:
                logger.debug(
                    "Failed expanding artist videos playlist %s",
                    art.videos_playlist_id,
                    exc_info=True,
                )
                return None

        async def _expand_releases(
            rel_bid: str | None, params: str | None, kind: str
        ) -> tuple[AlbumSummary, ...]:
            if not params:
                return ()
            try:
                return await self.artist_releases(rel_bid or browse_id, params, kind)
            except CATALOG_ERRORS:
                logger.debug(
                    "Failed expanding artist %s releases for %s",
                    kind,
                    browse_id,
                    exc_info=True,
                )
                return ()

        async def _fetch_browse_extras() -> tuple[dict[str, str], tuple[PlaylistSummary, ...]]:
            if self._injected:
                return {}, ()

            async def _safe_raw(cache_key: tuple[str, str], req_id: str) -> Any:
                try:
                    return await self._call(
                        cache_key, lambda: self._yt()._send_request("browse", {"browseId": req_id})
                    )
                except CATALOG_ERRORS:
                    logger.debug("Raw browse fetch failed for %s", req_id, exc_info=True)
                    return None

            clean_bid = browse_id.removeprefix("MPLA")
            tasks: list[Any] = [_safe_raw(("artist_raw", clean_bid), clean_bid)]
            if art.songs_playlist_id:
                pl_bid = (
                    art.songs_playlist_id
                    if art.songs_playlist_id.startswith(("VL", "MP"))
                    else f"VL{art.songs_playlist_id}"
                )
                tasks.append(_safe_raw(("playlist_raw", pl_bid), pl_bid))
            raw_pages = await asyncio.gather(*tasks)
            artist_raw = raw_pages[0]
            pl_raw = raw_pages[1] if len(raw_pages) > 1 else None

            plays_map: dict[str, str] = {}
            if pl_raw is not None:
                plays_map.update(parsers.extract_plays_from_browse(pl_raw))
            if artist_raw is not None:
                plays_map.update(parsers.extract_plays_from_browse(artist_raw))
                extra_playlists = parsers.extract_artist_playlists_from_browse(artist_raw)
            else:
                extra_playlists = ()
            return plays_map, extra_playlists

        songs_pl, videos_pl, full_albums, full_singles, (plays_map, extra_playlists) = (
            await asyncio.gather(
                _expand_songs(),
                _expand_videos(),
                _expand_releases(art.albums_browse_id, art.albums_params, "Album"),
                _expand_releases(art.singles_browse_id, art.singles_params, "Single"),
                _fetch_browse_extras(),
            )
        )

        fallback_artists = (ArtistRef(art.summary.name, browse_id),) if art.summary else ()

        if songs_pl and songs_pl.tracks:
            orig_by_vid = {t.video_id: t for t in art.songs}
            seen_vids: set[str] = set()
            enriched_songs: list[Track] = []
            for t in songs_pl.tracks:
                seen_vids.add(t.video_id)
                orig = orig_by_vid.get(t.video_id)
                enriched_songs.append(
                    replace(
                        t,
                        artists=t.artists or (orig.artists if orig else ()) or fallback_artists,
                        album=t.album or (orig.album if orig else None),
                        album_id=t.album_id or (orig.album_id if orig else None),
                        duration=t.duration or (orig.duration if orig else None),
                        thumbs=t.thumbs or (orig.thumbs if orig else ()),
                        plays=plays_map.get(t.video_id)
                        or t.plays
                        or (orig.plays if orig else None),
                        video_type=t.video_type or (orig.video_type if orig else None),
                    )
                )
            enriched_songs.extend(
                replace(
                    orig,
                    artists=orig.artists or fallback_artists,
                    plays=plays_map.get(orig.video_id) or orig.plays,
                )
                for orig in art.songs
                if orig.video_id not in seen_vids
            )
            final_songs = tuple(enriched_songs)
        else:
            final_songs = tuple(
                replace(
                    orig,
                    artists=orig.artists or fallback_artists,
                    plays=plays_map.get(orig.video_id) or orig.plays,
                )
                for orig in art.songs
            )

        if videos_pl and videos_pl.tracks:
            orig_vid_by_id = {t.video_id: t for t in art.videos}
            seen_mv: set[str] = set()
            enriched_videos: list[Track] = []
            for t in videos_pl.tracks:
                seen_mv.add(t.video_id)
                orig = orig_vid_by_id.get(t.video_id)
                enriched_videos.append(
                    replace(
                        t,
                        artists=t.artists or (orig.artists if orig else ()) or fallback_artists,
                        duration=t.duration or (orig.duration if orig else None),
                        thumbs=t.thumbs or (orig.thumbs if orig else ()),
                        plays=(orig.plays if orig else None) or t.plays,
                        video_type=t.video_type
                        or (orig.video_type if orig else None)
                        or "MUSIC_VIDEO_TYPE_OMV",
                    )
                )
            enriched_videos.extend(orig for orig in art.videos if orig.video_id not in seen_mv)
            final_videos = tuple(enriched_videos)
        else:
            final_videos = art.videos

        final_albums = tuple(
            a if a.artists else replace(a, artists=fallback_artists)
            for a in (full_albums or art.albums)
        )
        final_singles = tuple(
            a if a.artists else replace(a, artists=fallback_artists)
            for a in (full_singles or art.singles)
        )

        merged_playlists: list[PlaylistSummary] = []
        seen_pl: set[str] = set()
        for pl in (*art.playlists, *extra_playlists):
            key = pl.browse_id.removeprefix("VL")
            if key not in seen_pl:
                seen_pl.add(key)
                merged_playlists.append(pl)

        return replace(
            art,
            songs=final_songs,
            videos=final_videos,
            albums=final_albums,
            singles=final_singles,
            playlists=tuple(merged_playlists),
        )

    async def artist_releases(
        self, browse_id: str, params: str | None, kind: str = "Album"
    ) -> tuple[AlbumSummary, ...]:
        if not params:
            return ()
        raw = await self._call(
            ("releases", browse_id, params), lambda: self._yt().get_artist_albums(browse_id, params)
        )
        items = [
            parsers.parse_album_summary({**x, "type": x.get("type") or kind}) for x in raw or []
        ]
        return tuple(i for i in items if i)

    async def album(self, browse_id: str) -> Album:
        raw = await self._call(("album", browse_id), lambda: self._yt().get_album(browse_id))
        return parsers.parse_album(browse_id, raw)

    async def playlist(self, playlist_id: str, limit: int = 200) -> Playlist:
        pid = playlist_id.removeprefix("VL")
        raw = await self._call(
            ("playlist", pid, limit), lambda: self._yt().get_playlist(pid, limit=limit)
        )
        return parsers.parse_playlist(pid, raw)

    async def home_feed(self, region: str = "ID") -> HomeFeed:
        async def _safe_explore() -> dict[str, Any]:
            try:
                return await self._call(("explore", region), lambda: self._yt(region).get_explore())
            except CATALOG_ERRORS:
                logger.debug("Explore feed unavailable for region %s", region, exc_info=True)
                return {}

        async def _safe_charts() -> dict[str, Any]:
            try:
                return await self._call(
                    ("charts", region), lambda: self._yt(region).get_charts(country=region)
                )
            except CATALOG_ERRORS:
                logger.debug("Charts feed unavailable for region %s", region, exc_info=True)
                return {}

        explore, charts = await asyncio.gather(_safe_explore(), _safe_charts())
        return parsers.parse_home_feed(explore, charts)

    async def radio(
        self, video_id: str | None = None, playlist_id: str | None = None, limit: int = 25
    ) -> tuple[Track, ...]:
        if not video_id and not playlist_id:
            return ()
        raw = await self._call(
            ("watch", video_id, playlist_id, limit),
            lambda: self._yt().get_watch_playlist(
                videoId=video_id, playlistId=playlist_id, limit=limit, radio=True
            ),
        )
        return parsers.parse_tracks(raw.get("tracks") if isinstance(raw, dict) else None)

    async def yt_lyrics(self, video_id: str) -> Lyrics | None:
        raw = await self._call(
            ("watch_lyrics", video_id),
            lambda: self._yt().get_watch_playlist(videoId=video_id, limit=1),
        )
        lyrics_id = raw.get("lyrics") if isinstance(raw, dict) else None
        if not isinstance(lyrics_id, str) or not lyrics_id:
            return None

        def _get_timed() -> Any:
            # ytmusicapi.as_mobile() temporarily mutates client context; serialize shared client.
            if self._injected:
                with self._yt_lock:
                    return self._yt().get_lyrics(lyrics_id, timestamps=True)
            return self._yt().get_lyrics(lyrics_id, timestamps=True)

        try:
            data = await self._call(("yt_lyrics", lyrics_id), _get_timed)
        except CATALOG_ERRORS:
            logger.debug(
                "Timed YTM lyrics unavailable for %s, falling back to plain",
                lyrics_id,
                exc_info=True,
            )
            data = await self._call(
                ("yt_lyrics_plain", lyrics_id),
                lambda: self._yt().get_lyrics(lyrics_id, timestamps=False),
            )
        return parsers.parse_yt_lyrics(data if isinstance(data, dict) else None)
