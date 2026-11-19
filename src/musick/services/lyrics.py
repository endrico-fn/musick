"""Synced & plain lyrics service: LRC/VTT/JSON3, LRCLIB, YouTube Music & Captions."""

from __future__ import annotations

import asyncio
import html
import json
import logging
import os
import re
from bisect import bisect_right
from collections import OrderedDict
from collections.abc import Sequence
from pathlib import Path
from typing import Any

import httpx

from musick.config import Settings
from musick.domain import LyricLine, Lyrics, Track, clean_media_title
from musick.paths import LYRICS_CACHE_LIMITS, lyrics_cache_dir, prune_cache_dir

from .catalog import CATALOG_ERRORS, Catalog
from .process import communicate_capped
from .resolver import ytdlp_command

logger = logging.getLogger(__name__)

MAX_MEM_LYRICS = 128

_TS_RE = re.compile(r"\[(\d{1,3}):(\d{2})(?:[.:](\d{1,3}))?\]")
_OFFSET_RE = re.compile(r"^\[offset:([+-]?\d+)\]$", re.IGNORECASE)
_META_RE = re.compile(r"^\[[a-zA-Z]+:.*\]$")
_VTT_TS_RE = re.compile(r"(?:(\d{1,2}):)?(\d{2}):(\d{2})[.,](\d{3})\s*-->")
_TAG_RE = re.compile(r"<[^>]+>")
_PUNCT_ONLY_RE = re.compile(r"^[\s·•♪♫\-_=~.,!?…()\[\]\"']+$")
_WORD_RE = re.compile(r"[a-zA-Z]{2,}")

LRCLIB_BASE = "https://lrclib.net/api"
TRANSLATE_ENDPOINT = "https://translate.googleapis.com/translate_a/single"
USER_AGENT = "musick/0.1.0 (https://github.com/endrico-fn/musick)"

_ID_STOPWORDS = frozenset(
    {
        "aku",
        "kamu",
        "kau",
        "ku",
        "mu",
        "nya",
        "yang",
        "dan",
        "di",
        "ke",
        "dari",
        "untuk",
        "dengan",
        "tidak",
        "tak",
        "bukan",
        "jangan",
        "kita",
        "kami",
        "mereka",
        "ini",
        "itu",
        "sudah",
        "telah",
        "akan",
        "bisa",
        "dapat",
        "ada",
        "jadi",
        "hanya",
        "saja",
        "juga",
        "lagi",
        "kalau",
        "jika",
        "bila",
        "saat",
        "ketika",
        "karena",
        "atau",
        "tapi",
        "tetapi",
        "selalu",
        "pernah",
        "cinta",
        "hati",
        "rindu",
        "malam",
        "hari",
        "hidup",
        "dalam",
        "semua",
        "sendiri",
        "bersama",
        "mungkin",
        "ingin",
    }
)


def _should_skip_translation(text: str) -> bool:
    clean = text.strip()
    if not clean or _PUNCT_ONLY_RE.match(clean):
        return True
    return clean.startswith("[") and clean.endswith("]")


def _looks_indonesian(lines: Sequence[LyricLine]) -> bool:
    tokens: list[str] = []
    for ln in lines:
        tokens.extend(m.group(0).lower() for m in _WORD_RE.finditer(ln.text))
    if len(tokens) < 6:
        return False
    id_hits = sum(1 for tok in tokens if tok in _ID_STOPWORDS)
    return (id_hits / len(tokens)) >= 0.30


def _parse_translate_payload(data: Any) -> tuple[str, str]:
    if not isinstance(data, list) or not data:
        return ("", "")
    chunks = data[0]
    src_lang = str(data[2]) if len(data) > 2 and isinstance(data[2], str) else ""
    if not isinstance(chunks, list):
        return ("", src_lang)
    parts = [str(seg[0]) for seg in chunks if isinstance(seg, list) and seg and seg[0] is not None]
    return ("".join(parts), src_lang)


def parse_lrc(raw: str, source: str = "LRCLIB") -> Lyrics:
    """Parse LRC text into sorted LyricLine entries. Supports multi-tag lines & [offset:]."""
    if not raw or not raw.strip():
        return Lyrics()

    offset_sec = 0.0
    entries: list[LyricLine] = []
    plain_lines: list[str] = []

    for raw_line in raw.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        off_m = _OFFSET_RE.match(line)
        if off_m:
            offset_sec = int(off_m.group(1)) / 1000.0
            continue
        matches = list(_TS_RE.finditer(line))
        if not matches:
            if not _META_RE.match(line):
                plain_lines.append(line)
            continue
        text = _TS_RE.sub("", line).strip()
        for m in matches:
            mins = int(m.group(1))
            secs = int(m.group(2))
            frac = m.group(3)
            ms = int(frac.ljust(3, "0")) if frac else 0
            ts = max(0.0, mins * 60.0 + secs + ms / 1000.0 + offset_sec)
            entries.append(LyricLine(ts, text))

    if entries:
        entries.sort(key=lambda e: e.time)
        return Lyrics(tuple(entries), synced=True, source=source)

    if plain_lines:
        return Lyrics(tuple(LyricLine(0.0, t) for t in plain_lines), synced=False, source=source)

    return Lyrics()


def parse_vtt_captions(raw: str, source: str = "YouTube Captions") -> Lyrics:
    """Parse WebVTT/SRT subtitle text into synced LyricLine entries."""
    if not raw or not raw.strip():
        return Lyrics()
    entries: list[LyricLine] = []
    cur_time: float | None = None
    buf: list[str] = []

    def _flush() -> None:
        nonlocal cur_time, buf
        if cur_time is not None and buf:
            text = " ".join(buf).strip()
            text = html.unescape(_TAG_RE.sub("", text)).strip()
            if text and (not entries or entries[-1].text != text):
                entries.append(LyricLine(cur_time, text))
        cur_time = None
        buf = []

    for raw_ln in raw.splitlines():
        ln = raw_ln.strip()
        if not ln:
            _flush()
            continue
        if ln.startswith(("WEBVTT", "Kind:", "Language:", "NOTE")):
            continue
        m = _VTT_TS_RE.match(ln)
        if m:
            _flush()
            hrs = int(m.group(1) or 0)
            mins = int(m.group(2))
            secs = int(m.group(3))
            ms = int(m.group(4))
            cur_time = hrs * 3600.0 + mins * 60.0 + secs + ms / 1000.0
            continue
        if cur_time is None and ln.isdigit():
            continue
        if cur_time is not None:
            buf.append(ln)
    _flush()
    return Lyrics(tuple(entries), synced=True, source=source) if entries else Lyrics()


def parse_json3_captions(data: dict[str, Any], source: str = "YouTube Captions") -> Lyrics:
    """Parse YouTube json3 timedtext payload into synced LyricLine entries."""
    events = data.get("events") if isinstance(data, dict) else None
    if not isinstance(events, list):
        return Lyrics()
    entries: list[LyricLine] = []
    for ev in events:
        if not isinstance(ev, dict):
            continue
        segs = ev.get("segs")
        if not isinstance(segs, list):
            continue
        text = "".join(str(s.get("utf8", "")) for s in segs if isinstance(s, dict))
        text = " ".join(html.unescape(text).split()).strip()
        if not text:
            continue
        t_sec = max(0.0, float(ev.get("tStartMs", 0)) / 1000.0)
        if not entries or entries[-1].text != text:
            entries.append(LyricLine(t_sec, text))
    return Lyrics(tuple(entries), synced=True, source=source) if entries else Lyrics()


def active_line_index(lyrics: Lyrics, position: float) -> int | None:
    """Return the index of the active lyric line at playback `position` (seconds)."""
    if not lyrics.lines or not lyrics.synced:
        return None
    times = lyrics.times
    if len(times) != len(lyrics.lines):
        times = tuple(line.time for line in lyrics.lines)
    idx = bisect_right(times, position + 0.05) - 1
    return idx if idx >= 0 else None


def _cache_path(video_id: str) -> Path:
    safe = re.sub(r"[^a-zA-Z0-9_-]", "_", video_id)
    return lyrics_cache_dir() / f"{safe}.json"


class LyricsService:
    def __init__(
        self,
        settings: Settings,
        catalog: Catalog | None = None,
        http: httpx.AsyncClient | None = None,
    ) -> None:
        self._settings = settings
        self._catalog = catalog
        self._http = http
        self._owns_http = http is None
        self._mem: OrderedDict[str, Lyrics] = OrderedDict()
        self._inflight: dict[str, asyncio.Task[Lyrics]] = {}
        self._disk_writes = 0

    def _client(self) -> httpx.AsyncClient:
        if self._http is None:
            self._http = httpx.AsyncClient(
                timeout=8.0,
                headers={"User-Agent": USER_AGENT},
                follow_redirects=True,
                http2=True,
            )
        return self._http

    async def aclose(self) -> None:
        if self._owns_http and self._http is not None:
            await self._http.aclose()
            self._http = None

    def clear_cache(self) -> None:
        self._mem.clear()

    def _remember(self, video_id: str, lyrics: Lyrics) -> None:
        self._mem[video_id] = lyrics
        self._mem.move_to_end(video_id)
        while len(self._mem) > MAX_MEM_LYRICS:
            self._mem.popitem(last=False)

    def _read_disk(self, video_id: str, source_mode: str | None = None) -> Lyrics | None:
        if not self._settings.privacy.cache_lyrics:
            return None
        mode = source_mode or self._settings.lyrics.source
        path = _cache_path(video_id)
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
            cached_mode = data.get("mode")
            if cached_mode and cached_mode != mode:
                return None
            lines = tuple(
                LyricLine(float(x["t"]), str(x["s"]), str(x.get("tr", "")))
                for x in data.get("lines", [])
            )
            return Lyrics(
                lines,
                synced=bool(data.get("synced")),
                source=str(data.get("source", "")),
                translation_lang=str(data.get("translation_lang", "")),
            )
        except (OSError, ValueError, KeyError, TypeError):
            return None

    def _write_disk(
        self, video_id: str, lyrics: Lyrics, *, source_mode: str | None = None, prune: bool = False
    ) -> None:
        if not self._settings.privacy.cache_lyrics or not lyrics:
            return
        mode = source_mode or self._settings.lyrics.source
        path = _cache_path(video_id)
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            payload = {
                "mode": mode,
                "synced": lyrics.synced,
                "source": lyrics.source,
                "translation_lang": lyrics.translation_lang,
                "lines": [
                    {
                        "t": ln.time,
                        "s": ln.text,
                        **({"tr": ln.translation} if ln.translation else {}),
                    }
                    for ln in lyrics.lines
                ],
            }
            path.write_text(json.dumps(payload), encoding="utf-8")
            if prune:
                max_bytes, max_files = LYRICS_CACHE_LIMITS
                prune_cache_dir(
                    lyrics_cache_dir(),
                    max_bytes=max_bytes,
                    max_files=max_files,
                )
        except OSError:
            pass

    async def _save_disk(self, video_id: str, lyrics: Lyrics) -> None:
        if not self._settings.privacy.cache_lyrics or not lyrics:
            return
        self._disk_writes += 1
        await asyncio.to_thread(
            self._write_disk,
            video_id,
            lyrics,
            source_mode=self._settings.lyrics.source,
            prune=(self._disk_writes % 64 == 0),
        )

    async def fetch(self, track: Track) -> Lyrics:
        if not self._settings.lyrics.enabled:
            return Lyrics()
        vid = track.video_id
        if vid in self._mem:
            self._mem.move_to_end(vid)
            lyr = self._mem[vid]
        else:
            disk = (
                await asyncio.to_thread(self._read_disk, vid, self._settings.lyrics.source)
                if self._settings.privacy.cache_lyrics
                else None
            )
            if disk is not None:
                self._remember(vid, disk)
                lyr = disk
            else:
                task = self._inflight.get(vid)
                if task is None:
                    task = asyncio.create_task(self._load(track))
                    self._inflight[vid] = task
                    task.add_done_callback(
                        lambda t, k=vid: (
                            self._inflight.pop(k, None) if self._inflight.get(k) is t else None
                        )
                    )
                lyr = await asyncio.shield(task)

        if lyr and self._settings.lyrics.translate_to_id and lyr.translation_lang != "id":
            lyr = await self.translate_lyrics(lyr, video_id=vid, target_lang="id")

        if not lyr:
            return Lyrics()
        return lyr if (lyr.synced or not self._settings.lyrics.synced_only) else Lyrics()

    async def _translate_chunk(
        self, lines: Sequence[str], target_lang: str
    ) -> tuple[list[str], str]:
        client = self._client()
        client.cookies.clear()
        resp = await client.get(
            TRANSLATE_ENDPOINT,
            params=[
                ("client", "gtx"),
                ("sl", "auto"),
                ("tl", target_lang),
                ("dt", "t"),
                ("q", "\n".join(lines)),
            ],
        )
        if resp.status_code != 200:
            raise httpx.HTTPError(f"Translate HTTP {resp.status_code}")
        joined, src_lang = _parse_translate_payload(resp.json())
        parts = joined.splitlines() if joined else []
        return parts, src_lang

    async def _translate_texts(
        self, texts: Sequence[str], target_lang: str = "id"
    ) -> tuple[dict[str, str], bool]:
        unique = [t for t in dict.fromkeys(texts) if not _should_skip_translation(t)]
        if not unique:
            return {}, True
        if self._http is None and "PYTEST_CURRENT_TEST" in os.environ:
            return {}, False

        batches: list[list[str]] = []
        cur: list[str] = []
        cur_chars = 0
        for item in unique:
            if cur and (len(cur) >= 25 or cur_chars + len(item) + 1 > 1200):
                batches.append(cur)
                cur = [item]
                cur_chars = len(item)
            else:
                cur.append(item)
                cur_chars += len(item) + 1
        if cur:
            batches.append(cur)

        mapping: dict[str, str] = {}
        try:
            for batch in batches:
                parts, src_lang = await self._translate_chunk(batch, target_lang)
                if src_lang.lower().split("-")[0] == target_lang.lower():
                    return {}, True
                if len(parts) == len(batch):
                    pairs = zip(batch, parts, strict=True)
                else:
                    # Fallback to per-line calls if batch newline segmentation shifted
                    pairs = []
                    for orig in batch:
                        single, _ = await self._translate_chunk([orig], target_lang)
                        pairs.append((orig, " ".join(single) if single else ""))
                for orig, tr in pairs:
                    clean_tr = tr.strip()
                    if clean_tr and clean_tr.casefold() != orig.strip().casefold():
                        mapping[orig] = clean_tr
        except (httpx.HTTPError, ValueError, RuntimeError):
            return {}, False
        return mapping, True

    async def translate_lyrics(
        self,
        lyrics: Lyrics,
        *,
        video_id: str | None = None,
        target_lang: str = "id",
    ) -> Lyrics:
        if not lyrics:
            return lyrics
        if lyrics.translation_lang == target_lang and (
            lyrics.has_translations or _looks_indonesian(lyrics.lines)
        ):
            return lyrics
        if target_lang == "id" and _looks_indonesian(lyrics.lines):
            updated = Lyrics(
                lyrics.lines,
                synced=lyrics.synced,
                source=lyrics.source,
                translation_lang=target_lang,
            )
        else:
            tr_map, ok = await self._translate_texts(
                [ln.text for ln in lyrics.lines], target_lang=target_lang
            )
            if not ok:
                return lyrics
            new_lines = tuple(
                LyricLine(ln.time, ln.text, tr_map.get(ln.text, "")) for ln in lyrics.lines
            )
            updated = Lyrics(
                new_lines, synced=lyrics.synced, source=lyrics.source, translation_lang=target_lang
            )

        if video_id:
            self._remember(video_id, updated)
            await self._save_disk(video_id, updated)
        return updated

    async def _load(self, track: Track) -> Lyrics:
        mode = self._settings.lyrics.source
        lyr = Lyrics()

        if mode == "youtube":
            lyr = await self._fetch_yt_music(track)
            if not lyr or not lyr.synced:
                cap = await self._fetch_yt_captions(track)
                if cap and (not lyr or cap.synced):
                    lyr = cap
            if not lyr or not lyr.synced:
                lrc = await self._fetch_lrclib(track)
                if lrc and (not lyr or (lrc.synced and not lyr.synced)):
                    lyr = lrc
        elif mode == "lrclib":
            lyr = await self._fetch_lrclib(track)
        else:
            # Auto: LRCLIB first; upgrade plain lyrics if YTM or YouTube captions are synced.
            lyr = await self._fetch_lrclib(track)
            if not lyr or not lyr.synced:
                yt_lyr = await self._fetch_yt_music(track)
                if yt_lyr and (not lyr or yt_lyr.synced):
                    lyr = yt_lyr
            if not lyr or not lyr.synced:
                cap = await self._fetch_yt_captions(track)
                if cap and (not lyr or cap.synced):
                    lyr = cap

        self._remember(track.video_id, lyr)
        if lyr:
            await self._save_disk(track.video_id, lyr)
        return lyr

    async def _fetch_yt_music(self, track: Track) -> Lyrics:
        if self._catalog is None:
            return Lyrics()
        try:
            yt_lyr = await self._catalog.yt_lyrics(track.video_id)
            return yt_lyr or Lyrics()
        except CATALOG_ERRORS:
            logger.debug("YouTube Music lyrics fetch failed for %s", track.video_id, exc_info=True)
            return Lyrics()

    async def _fetch_yt_captions(self, track: Track) -> Lyrics:
        if "PYTEST_CURRENT_TEST" in os.environ:
            return Lyrics()
        target = f"https://www.youtube.com/watch?v={track.video_id}"
        try:
            proc = await asyncio.create_subprocess_exec(
                *ytdlp_command(),
                "--no-cookies",
                "--no-playlist",
                "--no-warnings",
                "--quiet",
                "--skip-download",
                "--print",
                "%(subtitles)j",
                target,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
            stdout, _ = await communicate_capped(proc, timeout=12.0)
        except (OSError, TimeoutError, ValueError):
            logger.debug("yt-dlp captions probe failed for %s", track.video_id, exc_info=True)
            return Lyrics()
        if proc.returncode != 0 or not stdout:
            return Lyrics()
        try:
            subs = json.loads(stdout.decode("utf-8", errors="ignore").strip())
        except ValueError:
            return Lyrics()
        if not isinstance(subs, dict) or not subs:
            return Lyrics()

        reg_lang = self._settings.display.region.lower()
        pref_langs = tuple(
            dict.fromkeys(
                [
                    *([] if reg_lang in ("zz", "") else [reg_lang]),
                    "en",
                    "en-US",
                    "en-GB",
                    "id",
                    "orig",
                ]
            )
        )
        lang_order = [k for k in pref_langs if k in subs] + [
            k for k in subs if k not in pref_langs and "live_chat" not in k.lower()
        ]
        client = self._client()
        for lang in lang_order:
            tracks = subs.get(lang)
            if not isinstance(tracks, list):
                continue
            pick = None
            for ext in ("json3", "vtt", "srt"):
                pick = next(
                    (
                        t
                        for t in tracks
                        if isinstance(t, dict) and t.get("ext") == ext and t.get("url")
                    ),
                    None,
                )
                if pick:
                    break
            if not pick:
                continue
            try:
                client.cookies.clear()
                resp = await client.get(str(pick["url"]))
                if resp.status_code != 200 or not resp.text:
                    continue
                if pick.get("ext") == "json3":
                    parsed = parse_json3_captions(resp.json())
                else:
                    parsed = parse_vtt_captions(resp.text)
                if parsed:
                    return parsed
            except (httpx.HTTPError, OSError, ValueError, KeyError, TypeError):
                logger.debug(
                    "Caption track parse failed for %s (%s)", track.video_id, lang, exc_info=True
                )
                continue
        return Lyrics()

    async def _fetch_lrclib(self, track: Track) -> Lyrics:
        artist = track.artists[0].name if track.artists else ""
        title = clean_media_title(track.title, strip_feat=False)
        if not title:
            return Lyrics()

        client = self._client()
        params: dict[str, Any] = {"track_name": title}
        if artist and artist != "Unknown artist":
            params["artist_name"] = artist
        if track.album:
            params["album_name"] = track.album
        if track.duration and track.duration > 0:
            params["duration"] = track.duration

        data = await self._get_json(client, f"{LRCLIB_BASE}/get", params)
        if not data:
            results = await self._get_json(
                client,
                f"{LRCLIB_BASE}/search",
                {"track_name": title, "artist_name": artist} if artist else {"q": title},
            )
            if isinstance(results, list) and results:
                data = next(
                    (r for r in results if isinstance(r, dict) and r.get("syncedLyrics")),
                    results[0],
                )

        if not isinstance(data, dict):
            return Lyrics()
        synced_raw = data.get("syncedLyrics")
        if isinstance(synced_raw, str) and synced_raw.strip():
            parsed = parse_lrc(synced_raw, source="LRCLIB")
            if parsed:
                return parsed
        plain_raw = data.get("plainLyrics")
        if isinstance(plain_raw, str) and plain_raw.strip():
            lines = tuple(LyricLine(0.0, ln.strip()) for ln in plain_raw.splitlines() if ln.strip())
            if lines:
                return Lyrics(lines, synced=False, source="LRCLIB")
        return Lyrics()

    @staticmethod
    async def _get_json(client: httpx.AsyncClient, url: str, params: dict[str, Any]) -> Any:
        try:
            client.cookies.clear()
            resp = await client.get(url, params=params)
            if resp.status_code == 200:
                return resp.json()
        except (httpx.HTTPError, ValueError):
            return None
        return None
