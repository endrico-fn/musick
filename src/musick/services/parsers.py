"""Pure functions turning ytmusicapi dicts into domain models. Lenient by design."""

from __future__ import annotations

from dataclasses import replace
from typing import Any

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


def _thumbs(d: dict[str, Any]) -> tuple[Thumb, ...]:
    raw = d.get("thumbnails") or d.get("thumbnail") or []
    return tuple(
        Thumb(t["url"], int(t.get("width") or 0), int(t.get("height") or 0))
        for t in raw
        if isinstance(t, dict) and t.get("url")
    )


def _artists(d: dict[str, Any]) -> tuple[ArtistRef, ...]:
    raw = d.get("artists")
    if isinstance(raw, dict):
        raw = [raw]
    if not raw and d.get("artist"):
        a = d["artist"]
        raw = [{"name": a}] if isinstance(a, str) else [a]
    return tuple(
        ArtistRef(a["name"], a.get("id"))
        for a in raw or []
        if isinstance(a, dict) and a.get("name")
    )


def _duration(d: dict[str, Any]) -> int | None:
    if d.get("duration_seconds"):
        return int(d["duration_seconds"])
    text = d.get("duration") or d.get("length")
    if isinstance(text, str) and ":" in text:
        try:
            secs = 0
            for part in text.split(":"):
                secs = secs * 60 + int(part)
        except ValueError:
            return None
        return secs
    return None


def _clean_plays(raw: Any) -> str | None:
    if raw is None:
        return None
    s = str(raw).strip()
    if not s:
        return None
    for suffix in (" views", " view", " plays", " play"):
        if s.lower().endswith(suffix):
            s = s[: -len(suffix)].strip()
            break
    return s or None


def parse_track(
    d: dict[str, Any],
    default_album: tuple[str | None, str | None] = (None, None),
    *,
    default_video_type: str | None = None,
) -> Track | None:
    vid = d.get("videoId")
    if not vid:
        return None
    album = d.get("album")
    album_name = (
        album.get("name")
        if isinstance(album, dict)
        else (album if isinstance(album, str) else None)
    )
    album_id = album.get("id") if isinstance(album, dict) else None
    vtype = d.get("videoType") or None
    if not vtype:
        rt = str(d.get("resultType") or "").lower()
        if rt == "song":
            vtype = "MUSIC_VIDEO_TYPE_ATV"
        elif rt == "video":
            vtype = "MUSIC_VIDEO_TYPE_VIDEO"
        else:
            vtype = default_video_type
    return Track(
        video_id=vid,
        title=d.get("title") or "Untitled",
        artists=_artists(d),
        album=album_name or default_album[0],
        album_id=album_id or default_album[1],
        duration=_duration(d),
        thumbs=_thumbs(d),
        plays=_clean_plays(d.get("views")),
        video_type=vtype,
    )


def parse_tracks(items: list[dict[str, Any]] | None, **kw) -> tuple[Track, ...]:
    return tuple(t for t in (parse_track(i, **kw) for i in items or []) if t)


def parse_album_summary(d: dict[str, Any]) -> AlbumSummary | None:
    bid = d.get("browseId")
    if not bid:
        return None
    kind = d.get("type") or d.get("resultType") or "Album"
    return AlbumSummary(
        browse_id=bid,
        title=d.get("title") or "Untitled",
        year=str(d["year"]) if d.get("year") else None,
        kind=str(kind).title() if str(kind).islower() else str(kind),
        artists=_artists(d),
        thumbs=_thumbs(d),
    )


def parse_artist_summary(d: dict[str, Any]) -> ArtistSummary | None:
    bid = d.get("browseId") or d.get("channelId")
    name = d.get("artist") or d.get("title") or d.get("name")
    raw_artists = d.get("artists")
    if isinstance(raw_artists, list) and raw_artists and isinstance(raw_artists[0], dict):
        first = raw_artists[0]
        bid = bid or first.get("id")
        name = name or first.get("name")
    if not bid or not name:
        return None
    return ArtistSummary(bid, name, d.get("subscribers") or d.get("category"), _thumbs(d))


def parse_playlist_summary(d: dict[str, Any]) -> PlaylistSummary | None:
    bid = d.get("browseId") or d.get("playlistId")
    if not bid:
        return None
    author = d.get("author")
    if isinstance(author, list):
        author = ", ".join(a.get("name", "") for a in author if isinstance(a, dict)) or None
    elif isinstance(author, dict):
        author = author.get("name")
    count = d.get("itemCount") or d.get("count")
    return PlaylistSummary(
        bid, d.get("title") or "Untitled", author, str(count) if count else None, _thumbs(d)
    )


_KIND_BY_RESULT = {
    "song": "track",
    "video": "track",
    "album": "album",
    "single": "album",
    "ep": "album",
    "artist": "artist",
    "playlist": "playlist",
}


def parse_search(items: list[dict[str, Any]] | None) -> tuple[SearchItem, ...]:
    out: list[SearchItem] = []
    top_artist: ArtistRef | None = None
    for idx, d in enumerate(items or []):
        kind = _KIND_BY_RESULT.get(str(d.get("resultType", "")).lower())
        item: SearchItem | None = None
        if kind == "track":
            t = parse_track(d)
            if t is not None:
                if not t.artists and top_artist is not None and idx <= 4:
                    t = replace(t, artists=(top_artist,))
                item = t
        elif kind == "album":
            item = parse_album_summary(d)
            if item is not None:
                top_artist = None
        elif kind == "artist":
            item = parse_artist_summary(d)
            if (
                isinstance(item, ArtistSummary)
                and idx == 0
                and str(d.get("category") or "").lower() == "top result"
            ):
                top_artist = ArtistRef(item.name, item.browse_id)
            else:
                top_artist = None
        elif kind == "playlist":
            item = parse_playlist_summary(d)
            top_artist = None
        if item:
            out.append(item)
    return tuple(out)


def _section(d: dict[str, Any], key: str) -> dict[str, Any]:
    v = d.get(key)
    return v if isinstance(v, dict) else {}


def parse_artist(browse_id: str, d: dict[str, Any]) -> Artist:
    summary = ArtistSummary(
        browse_id or d.get("channelId") or "", d.get("name") or "Unknown artist", None, _thumbs(d)
    )
    songs_sec = _section(d, "songs")
    songs = parse_tracks(songs_sec.get("results"))
    songs_playlist_id = (
        songs_sec.get("browseId") if isinstance(songs_sec.get("browseId"), str) else None
    )
    videos_sec = _section(d, "videos")
    videos = parse_tracks(
        videos_sec.get("results"), default_video_type="MUSIC_VIDEO_TYPE_OMV"
    )
    videos_playlist_id = (
        videos_sec.get("browseId") if isinstance(videos_sec.get("browseId"), str) else None
    )
    albums_sec = _section(d, "albums")
    albums = tuple(
        a for a in (parse_album_summary(x) for x in albums_sec.get("results") or []) if a
    )
    singles_sec = _section(d, "singles")
    singles = tuple(
        a
        for a in (
            parse_album_summary({**x, "type": x.get("type") or "Single"})
            for x in singles_sec.get("results") or []
        )
        if a
    )
    playlists: list[PlaylistSummary] = []
    seen_pl: set[str] = set()
    for x in _section(d, "playlists").get("results") or []:
        p = parse_playlist_summary(x)
        if p:
            key = p.browse_id.removeprefix("VL")
            if key not in seen_pl:
                seen_pl.add(key)
                playlists.append(p)
    related = tuple(
        a
        for a in (parse_artist_summary(x) for x in _section(d, "related").get("results") or [])
        if a
    )
    raw_listeners = d.get("monthlyListeners") or d.get("subscribers") or d.get("views")
    subscribers = str(d["subscribers"]).strip() if d.get("subscribers") else None
    views = _clean_plays(d.get("views"))
    albums_browse_id = (
        albums_sec.get("browseId") if isinstance(albums_sec.get("browseId"), str) else None
    )
    singles_browse_id = (
        singles_sec.get("browseId") if isinstance(singles_sec.get("browseId"), str) else None
    )
    shuffle_id = d.get("shuffleId") if isinstance(d.get("shuffleId"), str) else None
    return Artist(
        summary=summary,
        description=d.get("description"),
        listeners=str(raw_listeners) if raw_listeners else None,
        songs=songs,
        albums=albums,
        singles=singles,
        playlists=tuple(playlists),
        related=related,
        albums_params=albums_sec.get("params"),
        singles_params=singles_sec.get("params"),
        radio_id=d.get("radioId"),
        songs_playlist_id=songs_playlist_id,
        videos=videos,
        subscribers=subscribers,
        views=views,
        albums_browse_id=albums_browse_id,
        singles_browse_id=singles_browse_id,
        videos_playlist_id=videos_playlist_id,
        shuffle_id=shuffle_id,
    )


_PLAYS_SUFFIXES = (" plays", " play", " views", " view")


def _dig(obj: Any, *keys: str) -> Any:
    cur = obj
    for key in keys:
        if not isinstance(cur, dict):
            return None
        cur = cur.get(key)
    return cur


def _iter_renderers(obj: Any, key: str):
    if isinstance(obj, dict):
        if key in obj and isinstance(obj[key], dict):
            yield obj[key]
            return
        for val in obj.values():
            yield from _iter_renderers(val, key)
    elif isinstance(obj, list):
        for val in obj:
            yield from _iter_renderers(val, key)


def extract_plays_from_browse(raw_browse: Any) -> dict[str, str]:
    """Extract per-track shorthand play/view counts from raw Innertube browse JSON."""
    out: dict[str, str] = {}
    for item in _iter_renderers(raw_browse, "musicResponsiveListItemRenderer"):
        vid = _dig(item, "playlistItemData", "videoId")
        flex_cols = item.get("flexColumns") or []
        if not vid and flex_cols and isinstance(flex_cols[0], dict):
            runs0 = _dig(
                flex_cols[0],
                "musicResponsiveListItemFlexColumnRenderer",
                "text",
                "runs",
            )
            if isinstance(runs0, list) and runs0 and isinstance(runs0[0], dict):
                vid = _dig(runs0[0], "navigationEndpoint", "watchEndpoint", "videoId")
        if not vid:
            vid = _dig(
                item,
                "overlay",
                "musicItemThumbnailOverlayRenderer",
                "content",
                "musicPlayButtonRenderer",
                "playNavigationEndpoint",
                "watchEndpoint",
                "videoId",
            )
        if not isinstance(vid, str) or not vid:
            continue
        for fc in flex_cols:
            if not isinstance(fc, dict):
                continue
            runs = _dig(fc, "musicResponsiveListItemFlexColumnRenderer", "text", "runs") or []
            txt = "".join(str(r.get("text", "")) for r in runs if isinstance(r, dict)).strip()
            if txt.lower().endswith(_PLAYS_SUFFIXES):
                cleaned = _clean_plays(txt)
                if cleaned:
                    out[vid] = cleaned
                    break
    return out


def _browse_sections(raw_browse: Any) -> list[dict[str, Any]]:
    if not isinstance(raw_browse, dict):
        return []
    contents = raw_browse.get("contents")
    if not isinstance(contents, dict):
        return []
    for root_key in ("singleColumnBrowseResultsRenderer", "twoColumnBrowseResultsRenderer"):
        tabs = _dig(contents, root_key, "tabs")
        if isinstance(tabs, list) and tabs and isinstance(tabs[0], dict):
            sl = _dig(tabs[0], "tabRenderer", "content", "sectionListRenderer", "contents")
            if isinstance(sl, list):
                return [s for s in sl if isinstance(s, dict)]
    return []


def extract_artist_playlists_from_browse(raw_browse: Any) -> tuple[PlaylistSummary, ...]:
    """Extract artist playlists & 'Featured on' carousels from raw Innertube artist browse JSON."""
    out: list[PlaylistSummary] = []
    seen: set[str] = set()
    for sec in _browse_sections(raw_browse):
        carousel = sec.get("musicCarouselShelfRenderer")
        if not isinstance(carousel, dict):
            continue
        hdr_runs = (
            _dig(
                carousel,
                "header",
                "musicCarouselShelfBasicHeaderRenderer",
                "title",
                "runs",
            )
            or []
        )
        hdr_title = "".join(str(r.get("text", "")) for r in hdr_runs if isinstance(r, dict)).strip()
        hdr_low = hdr_title.lower()
        is_playlist_shelf = (
            hdr_low == "featured on"
            or hdr_low.startswith("playlists by")
            or "playlist" in hdr_low
        )
        if not is_playlist_shelf:
            continue
        for entry in carousel.get("contents") or []:
            if not isinstance(entry, dict):
                continue
            item = entry.get("musicTwoRowItemRenderer")
            if not isinstance(item, dict):
                continue
            title_runs = _dig(item, "title", "runs") or []
            title = "".join(
                str(r.get("text", "")) for r in title_runs if isinstance(r, dict)
            ).strip()
            browse_id = (
                _dig(title_runs[0], "navigationEndpoint", "browseEndpoint", "browseId")
                if title_runs and isinstance(title_runs[0], dict)
                else None
            )
            playlist_id = _dig(
                item,
                "thumbnailOverlay",
                "musicItemThumbnailOverlayRenderer",
                "content",
                "musicPlayButtonRenderer",
                "playNavigationEndpoint",
                "watchPlaylistEndpoint",
                "playlistId",
            )
            bid = browse_id or playlist_id
            if not isinstance(bid, str) or not bid:
                continue
            dedup = bid.removeprefix("VL")
            if dedup in seen:
                continue
            seen.add(dedup)

            sub_runs = _dig(item, "subtitle", "runs") or []
            tokens = [
                tok
                for r in sub_runs
                if isinstance(r, dict)
                for tok in (str(r.get("text", "")).strip(),)
                if tok and tok not in ("•", "·") and tok.lower() != "playlist"
            ]
            author: str | None = None
            count: str | None = None
            if len(tokens) >= 2:
                author = tokens[0]
                count = " · ".join(tokens[1:])
            elif len(tokens) == 1:
                tok = tokens[0]
                if any(w in tok.lower() for w in ("view", "track", "song")):
                    count = tok
                else:
                    author = tok
            thumb_list = (
                _dig(item, "thumbnailRenderer", "musicThumbnailRenderer", "thumbnail", "thumbnails")
                or []
            )
            thumbs = _thumbs({"thumbnails": thumb_list})
            out.append(PlaylistSummary(bid, title or "Untitled", author, count, thumbs))
    return tuple(out)


def parse_album(browse_id: str, d: dict[str, Any]) -> Album:
    summary = AlbumSummary(
        browse_id=browse_id,
        title=d.get("title") or "Untitled",
        year=str(d["year"]) if d.get("year") else None,
        kind=d.get("type") or "Album",
        artists=_artists(d),
        thumbs=_thumbs(d),
    )
    tracks = parse_tracks(d.get("tracks"), default_album=(summary.title, browse_id))
    # album tracks carry no thumbs: inherit the album's so Now Playing can show a cover
    tracks = tuple(
        t
        if t.thumbs
        else replace(t, artists=t.artists or summary.artists, thumbs=summary.thumbs)
        for t in tracks
    )
    return Album(summary, tracks, d.get("description"), d.get("duration"))


def parse_playlist(browse_id: str, d: dict[str, Any]) -> Playlist:
    author = d.get("author")
    author_name = author.get("name") if isinstance(author, dict) else author
    summary = PlaylistSummary(
        browse_id=d.get("id") or browse_id,
        title=d.get("title") or "Untitled",
        author=author_name,
        count=str(d["trackCount"]) if d.get("trackCount") else None,
        thumbs=_thumbs(d),
    )
    return Playlist(
        summary=summary,
        tracks=parse_tracks(d.get("tracks")),
        description=d.get("description"),
        views=str(d["views"]) if d.get("views") else None,
        duration_text=d.get("duration"),
        year=str(d["year"]) if d.get("year") else None,
        total=d.get("trackCount"),
    )


def parse_yt_lyrics(d: dict[str, Any] | None) -> Lyrics | None:
    if not d:
        return None
    raw = d.get("lyrics")
    src = str(d.get("source") or "YouTube Music")
    if isinstance(raw, list):
        lines = []
        for entry in raw:
            txt = getattr(entry, "text", None) if not isinstance(entry, dict) else entry.get("text")
            start = (
                getattr(entry, "start_time", None)
                if not isinstance(entry, dict)
                else entry.get("start_time")
            )
            if txt is None:
                continue
            t_sec = float(start) / 1000.0 if start is not None else 0.0
            lines.append(LyricLine(t_sec, str(txt)))
        if lines:
            return Lyrics(tuple(lines), synced=bool(d.get("hasTimestamps")), source=src)
    if isinstance(raw, str) and raw.strip():
        lines = tuple(LyricLine(0.0, ln.strip()) for ln in raw.splitlines())
        return Lyrics(lines, synced=False, source=src)
    return None


def parse_home_feed(
    explore_or_home: dict[str, Any] | list[dict[str, Any]] | None, charts: dict[str, Any] | None
) -> HomeFeed:
    featured: list[AlbumSummary | PlaylistSummary] = []
    artists: list[ArtistSummary] = []
    playlists: list[PlaylistSummary] = []
    seen_ids: set[str] = set()

    if isinstance(charts, dict):
        for raw_a in charts.get("artists") or []:
            a = parse_artist_summary(raw_a)
            if a and a.browse_id not in seen_ids:
                seen_ids.add(a.browse_id)
                artists.append(a)
        for key in ("videos", "genres"):
            for raw_p in charts.get(key) or []:
                p = parse_playlist_summary(raw_p)
                if p and p.browse_id not in seen_ids:
                    seen_ids.add(p.browse_id)
                    playlists.append(p)

    if isinstance(explore_or_home, dict):
        for raw_rel in explore_or_home.get("new_releases") or []:
            if isinstance(raw_rel, dict):
                al = parse_album_summary(raw_rel)
                if al and al.browse_id not in seen_ids:
                    seen_ids.add(al.browse_id)
                    featured.append(al)
        tr_pl = (explore_or_home.get("trending") or {}).get("playlist")
        if isinstance(tr_pl, str) and tr_pl and tr_pl not in seen_ids:
            seen_ids.add(tr_pl)
            playlists.insert(
                0, PlaylistSummary(tr_pl, "Trending on YouTube Music", "YouTube Music")
            )
    elif isinstance(explore_or_home, list):
        for row in explore_or_home:
            for entry in row.get("contents") or []:
                if not isinstance(entry, dict):
                    continue
                bid = entry.get("browseId") or entry.get("playlistId") or ""
                if not bid or bid in seen_ids:
                    continue
                if entry.get("playlistId") or bid.startswith(("PL", "VL", "RD")):
                    p = parse_playlist_summary(entry)
                    if p:
                        seen_ids.add(bid)
                        if len(featured) < 10:
                            featured.append(p)
                        else:
                            playlists.append(p)
                elif bid.startswith("MPRE"):
                    al = parse_album_summary(entry)
                    if al:
                        seen_ids.add(bid)
                        featured.append(al)
                elif bid.startswith("UC"):
                    ar = parse_artist_summary(entry)
                    if ar:
                        seen_ids.add(bid)
                        artists.append(ar)

    return HomeFeed(
        featured=tuple(featured[:10]),
        artists=tuple(artists[:10]),
        playlists=tuple(playlists[:10]),
    )
