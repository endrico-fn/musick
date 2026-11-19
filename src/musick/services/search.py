"""Smart search, prefix parsing, URL lookup, fuzzy local history, and result ranking."""

from __future__ import annotations

import contextlib
import json
import logging
import math
import re
import unicodedata
import urllib.parse
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from rapidfuzz import fuzz

from musick.config import Settings
from musick.domain import (
    AlbumSummary,
    ArtistRef,
    ArtistSummary,
    PlaylistSummary,
    SearchItem,
    Thumb,
    Track,
    clean_media_title,
    search_item_key,
)
from musick.paths import atomic_write_text, history_file

from .catalog import CATALOG_ERRORS, Catalog

logger = logging.getLogger(__name__)

SearchCategory = Literal["all", "songs", "videos", "albums", "artists", "playlists", "community"]

_PREFIX_MAP: dict[str, SearchCategory] = {
    "a:": "artists",
    "artist:": "artists",
    "al:": "albums",
    "album:": "albums",
    "pl:": "playlists",
    "playlist:": "playlists",
    "c:": "community",
    "comm:": "community",
    "community:": "community",
    "v:": "videos",
    "video:": "videos",
    "vid:": "videos",
    "s:": "songs",
    "song:": "songs",
}

_STRONG_NOISE_TOKENS: tuple[str, ...] = (
    "slowed",
    "reverb",
    "sped up",
    "speed up",
    "nightcore",
    "tiktok",
    "mashup",
    "8d audio",
    "bass boosted",
    "karaoke",
    "instrumental",
    "1 hour",
    "reaction",
)

_PAREN_WITHOUT_RE = re.compile(r"[\(\[][^\)\]]*\bwithout\b[^\)\]]*[\)\]]", re.IGNORECASE)

_MILD_VARIANT_TOKENS: tuple[str, ...] = (
    "remix",
    "cover",
    "live",
    "acoustic",
    "lyric video",
    "official video",
    "music video",
    "lyrics",
)


_STRONG_NOISE_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = tuple(
    (tok.strip(), re.compile(r"\b" + re.escape(tok.strip()) + r"\b", re.IGNORECASE))
    for tok in _STRONG_NOISE_TOKENS
)

_MILD_VARIANT_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = tuple(
    (tok.strip(), re.compile(r"\b" + re.escape(tok.strip()) + r"\b", re.IGNORECASE))
    for tok in _MILD_VARIANT_TOKENS
)


def _norm_text(text: str) -> str:
    decomposed = unicodedata.normalize("NFKD", text.lower())
    folded = "".join(ch for ch in decomposed if not unicodedata.combining(ch))
    cleaned = re.sub(r"[^\w\s]", " ", folded)
    return " ".join(cleaned.split())


def _clean_song_title(title: str) -> str:
    stripped = clean_media_title(title, strip_feat=True)
    return _norm_text(stripped) or _norm_text(title)


_SCHEMELESS_YT_RE = re.compile(r"^(?:(?:www|m|music)\.)?(?:youtube\.com|youtu\.be)/", re.IGNORECASE)


def extract_youtube_target(raw: str) -> tuple[str, str] | None:
    """Detect direct YouTube/YTM URLs and return ('track'|'playlist'|'album'|'artist', id)."""
    s = raw.strip()
    if not s or " " in s:
        return None
    if not s.startswith(("http://", "https://")):
        if not _SCHEMELESS_YT_RE.match(s):
            return None
        s = "https://" + s
    try:
        parsed = urllib.parse.urlparse(s)
    except ValueError:
        return None
    host = parsed.netloc.lower()
    if not any(d in host for d in ("youtube.com", "youtu.be")):
        return None
    qs = urllib.parse.parse_qs(parsed.query)
    if "youtu.be" in host:
        vid = parsed.path.strip("/").split("/")[0]
        if vid:
            return ("track", vid)
    if v := qs.get("v"):
        return ("track", v[0])
    if lst := qs.get("list"):
        return ("playlist", lst[0])
    parts = [p for p in parsed.path.split("/") if p]
    if len(parts) >= 2 and parts[0] in ("shorts", "live"):
        return ("track", parts[1])
    if len(parts) >= 2 and parts[0] in ("channel", "browse"):
        bid = parts[1]
        if bid.startswith("MPRE"):
            return ("album", bid)
        if bid.startswith("UC"):
            return ("artist", bid)
        if bid.startswith(("VL", "PL", "OLAK", "RD")):
            return ("playlist", bid)
    return None


def _parse_play_count(plays: str | None) -> float:
    """Convert shorthand display counts ('2.9B', '337M', '45K') into numeric scale."""
    if not plays:
        return 0.0
    s = plays.strip().lower().replace(",", "").replace(" ", "")
    mult = 1.0
    for suffix, factor in (
        ("rb", 1e3),
        ("jt", 1e6),
        ("b", 1e9),
        ("m", 1e6),
        ("k", 1e3),
    ):
        if s.endswith(suffix):
            s = s[: -len(suffix)]
            mult = factor
            break
    try:
        return max(0.0, float(s) * mult)
    except ValueError:
        return 0.0


def _play_bonus(item: SearchItem) -> float:
    """Small logarithmic bonus so mainstream official hits outrank 0-play uploads."""
    plays = getattr(item, "plays", None) or getattr(item, "subtitle", None)
    count = _parse_play_count(plays)
    if count < 100.0:
        return 0.0
    return min(10.0, math.log10(count) * 1.0)


_CONTENT_STOPWORDS = frozenset(
    {
        "i",
        "m",
        "im",
        "s",
        "t",
        "re",
        "ve",
        "ll",
        "d",
        "a",
        "an",
        "the",
        "you",
        "me",
        "my",
        "your",
        "to",
        "in",
        "on",
        "of",
        "is",
        "it",
        "and",
        "or",
        "for",
        "that",
        "this",
    }
)


def _meaningful_tokens(q_norm: str) -> set[str]:
    toks = q_norm.split()
    if len(toks) < 3:
        return set(toks)
    content = {t for t in toks if t not in _CONTENT_STOPWORDS}
    return content or set(toks)


def _has_requested_variant(q_norm: str) -> bool:
    return any(
        re.search(r"\b" + re.escape(tok) + r"\b", q_norm) is not None
        for tok, _ in (*_STRONG_NOISE_PATTERNS, *_MILD_VARIANT_PATTERNS)
    )


def _has_strong_noise(raw_lower: str, q_norm: str) -> bool:
    return any(
        pat.search(raw_lower) and not re.search(r"\b" + re.escape(tok) + r"\b", q_norm)
        for tok, pat in _STRONG_NOISE_PATTERNS
    ) or (_PAREN_WITHOUT_RE.search(raw_lower) is not None and not re.search(r"\bwithout\b", q_norm))


def _mild_variant_hits(raw_lower: str, q_norm: str) -> int:
    return sum(
        1
        for tok, pat in _MILD_VARIANT_PATTERNS
        if pat.search(raw_lower) and not re.search(r"\b" + re.escape(tok) + r"\b", q_norm)
    )


def _item_lexical_overlap(item: SearchItem, q_norm: str, q_tokens: set[str]) -> float:
    """Return 0.0..1.0 indicating how strongly item matches query text directly."""
    if not q_norm or not q_tokens:
        return 0.0
    if isinstance(item, ArtistSummary):
        a_norm = _norm_text(item.name)
        if a_norm == q_norm or q_norm in a_norm:
            return 1.0
        toks = set(a_norm.split())
        return len(q_tokens & toks) / len(q_tokens) if toks else 0.0
    if isinstance(item, AlbumSummary):
        al_norm = _norm_text(item.title)
        art_norm = _norm_text(" ".join(a.name for a in item.artists))
        toks = set(f"{art_norm} {al_norm}".split())
        if al_norm == q_norm or q_norm in al_norm:
            return 1.0
        return len(q_tokens & toks) / len(q_tokens) if toks else 0.0
    if isinstance(item, PlaylistSummary):
        toks = set(_norm_text(item.title).split())
        return len(q_tokens & toks) / len(q_tokens) if toks else 0.0
    if isinstance(item, Track):
        clean_title = _clean_song_title(item.title)
        raw_norm = _norm_text(item.title)
        art_norm = _norm_text(item.artist_names if item.artists else "")
        if clean_title == q_norm or q_norm in clean_title or (art_norm and art_norm == q_norm):
            return 1.0
        toks = set(f"{art_norm} {raw_norm}".split())
        return len(q_tokens & toks) / len(q_tokens) if toks else 0.0
    return 0.0


def _is_semantic_result_set(items: Sequence[SearchItem], q_norm: str) -> bool:
    """Detect YTMusic semantic/lyric queries whose top hits don't literally match the title."""
    if _has_requested_variant(q_norm):
        return False
    q_tokens = _meaningful_tokens(q_norm)
    if len(q_tokens) < 2:
        return False
    clean_head: list[SearchItem] = []
    for it in items:
        if isinstance(it, Track) and _has_strong_noise(it.title.lower(), q_norm):
            continue
        clean_head.append(it)
        if len(clean_head) >= 4:
            break
    if not clean_head:
        return False
    overlaps = [_item_lexical_overlap(it, q_norm, q_tokens) for it in clean_head]
    # Top clean YTM results lack query tokens -> matched semantically / via lyrics
    return max(overlaps[:3]) < 0.5


def score_search_item(
    item: SearchItem,
    query: str,
    idx: int,
    *,
    kind: SearchCategory = "all",
    semantic: bool = False,
) -> float:
    """Score search items for official quality while respecting YTMusic semantic/lyric rank."""
    q_norm = _norm_text(query)
    if not q_norm:
        return float(-idx)

    if semantic:
        # Preserve YouTube Music lyric/semantic order while favoring ATV & demoting spam
        score = max(0.0, 90.0 - idx * 3.0) + _play_bonus(item)
        if isinstance(item, Track):
            raw_lower = item.title.lower()
            if item.video_type == "MUSIC_VIDEO_TYPE_ATV":
                score += 10.0
            elif item.album:
                score += 4.0
            elif item.video_type in ("MUSIC_VIDEO_TYPE_OMV", "MUSIC_VIDEO_TYPE_VIDEO"):
                score -= 4.0

            if _has_strong_noise(raw_lower, q_norm):
                score -= 100.0
            else:
                mild = _mild_variant_hits(raw_lower, q_norm)
                if mild == 1:
                    score -= 28.0
                elif mild >= 2:
                    score -= 42.0
        elif isinstance(item, ArtistSummary):
            score += 4.0
        elif isinstance(item, AlbumSummary):
            score -= 8.0
        elif isinstance(item, PlaylistSummary):
            score -= 6.0
        return score

    # Direct title / artist lookup scoring with strong positional anchor & popularity signal
    score = max(0.0, 18.0 - idx * 0.55) + _play_bonus(item)

    if isinstance(item, ArtistSummary):
        a_norm = _norm_text(item.name)
        ratio = fuzz.WRatio(q_norm, a_norm) if a_norm else 0.0
        if a_norm == q_norm:
            score += 88.0
        elif a_norm.startswith(q_norm) or q_norm in a_norm:
            score += 46.0
        elif ratio >= 75:
            score += ratio * 0.42
        elif kind == "all":
            score += 10.0
        return score

    if isinstance(item, AlbumSummary):
        al_norm = _norm_text(item.title)
        art_norm = _norm_text(" ".join(a.name for a in item.artists))
        combo = f"{art_norm} {al_norm}".strip()
        if al_norm == q_norm:
            score += 50.0
        else:
            r = max(fuzz.WRatio(q_norm, al_norm), fuzz.token_set_ratio(q_norm, combo))
            if r >= 70:
                score += r * 0.34
        if kind == "all":
            score -= 6.0
        return score

    if isinstance(item, PlaylistSummary):
        pl_norm = _norm_text(item.title)
        r = fuzz.WRatio(q_norm, pl_norm) if pl_norm else 0.0
        if pl_norm == q_norm:
            score += 36.0
        elif r >= 70:
            score += r * 0.28
        if kind == "all":
            score -= 6.0
        return score

    if isinstance(item, Track):
        raw_lower = item.title.lower()
        clean_title = _clean_song_title(item.title)
        art_norm = _norm_text(item.artist_names if item.artists else "")
        combo = f"{art_norm} {clean_title}".strip()

        q_tokens = set(q_norm.split())
        title_tokens = set(clean_title.split())
        combo_tokens = set(combo.split())
        matched = False
        if clean_title == q_norm:
            score += 55.0
            matched = True
        elif clean_title.startswith(q_norm) or (q_tokens and q_tokens.issubset(title_tokens)):
            score += 38.0
            matched = True
        elif art_norm and q_norm == art_norm:
            score += 14.0
            matched = True
        elif combo_tokens and q_tokens.issubset(combo_tokens):
            score += 34.0
            matched = True
        elif q_norm in clean_title:
            score += 18.0
            matched = True
        else:
            r = max(
                fuzz.token_sort_ratio(q_norm, clean_title), fuzz.token_sort_ratio(q_norm, combo)
            )
            if r >= 60:
                score += r * 0.35
                matched = True

        if matched:
            if item.video_type == "MUSIC_VIDEO_TYPE_ATV":
                score += 30.0
            elif item.album:
                score += 22.0
            elif item.video_type == "MUSIC_VIDEO_TYPE_OMV":
                score += 4.0
            else:
                score -= 10.0

        for tok, pat in (*_STRONG_NOISE_PATTERNS, *_MILD_VARIANT_PATTERNS):
            if re.search(r"\b" + re.escape(tok) + r"\b", q_norm) and pat.search(raw_lower):
                score += 26.0

        if _has_strong_noise(raw_lower, q_norm):
            score -= 60.0
        else:
            mild_hits = _mild_variant_hits(raw_lower, q_norm)
            if mild_hits == 1:
                score -= 26.0
            elif mild_hits >= 2:
                score -= 40.0

        return score

    return score


def rank_search_items(
    items: Sequence[SearchItem], query: str, *, kind: SearchCategory = "all"
) -> tuple[SearchItem, ...]:
    """Deduplicate and re-rank results for official quality and YouTube Music lyric parity."""
    deduped: list[SearchItem] = []
    seen: set[tuple[str, str]] = set()
    for it in items:
        k = search_item_key(it)
        if k in seen:
            continue
        seen.add(k)
        deduped.append(it)

    if kind not in ("all", "songs") or not query.strip():
        return tuple(deduped)

    q_norm = _norm_text(query)
    semantic = _is_semantic_result_set(deduped, q_norm)
    scored = sorted(
        enumerate(deduped),
        key=lambda pair: (
            -score_search_item(pair[1], query, pair[0], kind=kind, semantic=semantic),
            pair[0],
        ),
    )
    return tuple(it for _, it in scored)


@dataclass(frozen=True, slots=True)
class Suggestion:
    text: str
    kind_hint: str = ""
    from_history: bool = False


def parse_query_prefix(
    raw: str, default_kind: SearchCategory = "all"
) -> tuple[SearchCategory, str]:
    """Extract category prefix like 'a: the weeknd' -> ('artists', 'the weeknd')."""
    stripped = raw.lstrip()
    low = stripped.lower()
    for prefix, cat in _PREFIX_MAP.items():
        if low.startswith(prefix):
            return cat, stripped[len(prefix) :].strip()
    return default_kind, raw.strip()


def _track_to_dict(t: Track) -> dict:
    return {
        "video_id": t.video_id,
        "title": t.title,
        "artists": [{"name": a.name, "id": a.id} for a in t.artists],
        "album": t.album,
        "album_id": t.album_id,
        "duration": t.duration,
        "thumbs": [{"url": th.url, "w": th.width, "h": th.height} for th in t.thumbs],
    }


def _track_from_dict(d: dict) -> Track | None:
    vid = d.get("video_id")
    if not vid:
        return None
    artists = tuple(
        ArtistRef(a["name"], a.get("id"))
        for a in d.get("artists", [])
        if isinstance(a, dict) and a.get("name")
    )
    thumbs = tuple(
        Thumb(th["url"], int(th.get("w") or 0), int(th.get("h") or 0))
        for th in d.get("thumbs", [])
        if isinstance(th, dict) and th.get("url")
    )
    return Track(
        video_id=vid,
        title=str(d.get("title") or "Untitled"),
        artists=artists,
        album=d.get("album"),
        album_id=d.get("album_id"),
        duration=d.get("duration"),
        thumbs=thumbs,
    )


class HistoryStore:
    """Stores recent queries and tracks in memory; persists only when privacy.save_history=True."""

    MAX_QUERIES = 40
    MAX_TRACKS = 30

    def __init__(self, settings: Settings, path: Path | None = None) -> None:
        self._settings = settings
        self._path = path or history_file()
        self.queries: list[str] = []
        self.recent_tracks: list[Track] = []
        if self._settings.privacy.save_history:
            self._load()

    def _load(self) -> None:
        try:
            data = json.loads(self._path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return
        self.queries = [str(q) for q in data.get("queries", []) if isinstance(q, str)][
            : self.MAX_QUERIES
        ]
        tracks: list[Track] = []
        for raw_t in data.get("tracks", []):
            if isinstance(raw_t, dict):
                t = _track_from_dict(raw_t)
                if t:
                    tracks.append(t)
        self.recent_tracks = tracks[: self.MAX_TRACKS]

    def _save(self) -> None:
        if not self._settings.privacy.save_history:
            return
        try:
            payload = {
                "queries": self.queries[: self.MAX_QUERIES],
                "tracks": [_track_to_dict(t) for t in self.recent_tracks[: self.MAX_TRACKS]],
            }
            atomic_write_text(self._path, json.dumps(payload))
        except OSError:
            pass

    def record_query(self, query: str) -> None:
        q = query.strip()
        if not q:
            return
        # Persisted only when privacy.save_history=True
        if not self._settings.privacy.save_history:
            return
        self.queries = [q] + [x for x in self.queries if x.lower() != q.lower()]
        self.queries = self.queries[: self.MAX_QUERIES]
        self._save()

    def record_track(self, track: Track) -> None:
        # Session Continue Listening stays in RAM during the run; written to disk only if opted-in
        self.recent_tracks = [track] + [
            t for t in self.recent_tracks if t.video_id != track.video_id
        ]
        self.recent_tracks = self.recent_tracks[: self.MAX_TRACKS]
        self._save()

    def flush(self) -> None:
        """Persist in-memory history if privacy.save_history is enabled."""
        self._save()

    def clear(self) -> None:
        self.queries.clear()
        self.recent_tracks.clear()
        if self._path.exists():
            with contextlib.suppress(OSError):
                self._path.unlink()


class SearchService:
    def __init__(self, catalog: Catalog, history: HistoryStore, settings: Settings) -> None:
        self._catalog = catalog
        self._history = history
        self._settings = settings

    async def search(
        self, kind: SearchCategory, query: str, limit: int = 40
    ) -> tuple[SearchItem, ...]:
        target = extract_youtube_target(query)
        if target is not None:
            t_kind, t_id = target
            if t_kind == "track":
                try:
                    tracks = await self._catalog.radio(video_id=t_id, limit=5)
                    exact = next((t for t in tracks if t.video_id == t_id), None)
                    if exact is not None:
                        return (exact, *[t for t in tracks if t.video_id != t_id])
                    if tracks:
                        return (Track(video_id=t_id, title=f"YouTube Track ({t_id})"), *tracks)
                except CATALOG_ERRORS:
                    logger.debug("Track URL radio lookup failed for %s", t_id, exc_info=True)
                return (Track(video_id=t_id, title=f"YouTube Track ({t_id})"),)
            if t_kind == "playlist":
                pl = await self._catalog.playlist(t_id, limit=50)
                return (pl.summary, *pl.tracks[:15])
            if t_kind == "album":
                alb = await self._catalog.album(t_id)
                return (alb.summary, *alb.tracks)
            if t_kind == "artist":
                art = await self._catalog.artist(t_id)
                return (art.summary, *art.songs[:15])

        items = await self._catalog.search(kind, query, limit=limit)
        return rank_search_items(items, query, kind=kind)[:limit]

    def fuzzy_history(self, query: str, limit: int = 5) -> list[Suggestion]:
        if not self._settings.privacy.save_history or not self._history.queries:
            return []
        q = query.strip()
        if not q:
            return [
                Suggestion(text=h, kind_hint="History", from_history=True)
                for h in self._history.queries[:limit]
            ]
        scored: list[tuple[float, str]] = []
        for h in self._history.queries:
            score = fuzz.WRatio(q.lower(), h.lower())
            if score >= 55:
                scored.append((score, h))
        scored.sort(key=lambda item: item[0], reverse=True)
        return [
            Suggestion(text=h, kind_hint="History", from_history=True) for _, h in scored[:limit]
        ]

    async def autocomplete(self, raw_query: str, limit: int = 7) -> list[Suggestion]:
        cat, clean_q = parse_query_prefix(raw_query)
        hist = self.fuzzy_history(clean_q, limit=3)
        if not clean_q or not self._settings.privacy.remote_suggestions:
            return hist[:limit]

        try:
            remote = await self._catalog.suggestions(clean_q)
        except CATALOG_ERRORS:
            logger.debug("Remote autocomplete failed for %r", clean_q, exc_info=True)
            remote = []

        seen = {s.text.lower() for s in hist}
        hint = "" if cat == "all" else cat.rstrip("s").title()
        out = list(hist)
        for item in remote:
            if item.lower() in seen:
                continue
            seen.add(item.lower())
            out.append(Suggestion(text=item, kind_hint=hint, from_history=False))
            if len(out) >= limit:
                break
        return out
