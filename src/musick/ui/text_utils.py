"""Display-cell-aware text truncation, alignment, and two-column row splitting."""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass

from rich.cells import cell_len


def trunc_cells(text: str, width: int) -> str:
    """Truncate text to at most `width` terminal display cells, appending ellipsis if needed."""
    if width <= 0:
        return ""
    if cell_len(text) <= width:
        return text
    if width == 1:
        for ch in text:
            return ch if cell_len(ch) <= 1 else " "
        return ""
    out: list[str] = []
    used = 0
    limit = width - 1
    for ch in text:
        cw = cell_len(ch)
        if used + cw > limit:
            break
        out.append(ch)
        used += cw
    return "".join(out) + "…"


def align_cells(text: str, width: int, align: str = "l") -> str:
    """Truncate and pad text to exactly `width` display cells."""
    s = trunc_cells(text, width)
    pad = max(0, width - cell_len(s))
    return (" " * pad + s) if align == "r" else (s + " " * pad)


def split_row_cells(
    left: str,
    right: str,
    total_w: int,
    *,
    gutter: int = 2,
    right_cap_ratio: float = 0.4,
) -> tuple[str, str, int]:
    """Fit left and right strings into total_w (excluding `gutter`) using display cells."""
    avail = max(4, total_w - gutter)
    if not right:
        l_str = trunc_cells(left, avail)
        return l_str, "", max(0, avail - cell_len(l_str))
    r_max = max(4, int(avail * right_cap_ratio))
    r_str = trunc_cells(right, r_max)
    l_max = max(3, avail - cell_len(r_str) - 2)
    l_str = trunc_cells(left, l_max)
    gap = max(1, avail - cell_len(l_str) - cell_len(r_str))
    return l_str, r_str, gap


def _hard_break_word(word: str, max_w: int) -> list[str]:
    chunks: list[str] = []
    cur: list[str] = []
    cw_sum = 0
    for ch in word:
        chw = cell_len(ch)
        if cw_sum + chw > max_w and cur:
            chunks.append("".join(cur))
            cur = [ch]
            cw_sum = chw
        else:
            cur.append(ch)
            cw_sum += chw
    if cur:
        chunks.append("".join(cur))
    return chunks or [word[:max_w]]


def wrap_cells(text: str, width: int) -> list[str]:
    """Word-wrap text so every returned line fits within `width` display cells (CJK/emoji safe)."""
    max_w = max(4, width)
    if cell_len(text) <= max_w:
        return [text]
    lines: list[str] = []
    cur: list[str] = []
    cur_w = 0
    for word in text.split(" "):
        ww = cell_len(word)
        if not cur:
            if ww <= max_w:
                cur.append(word)
                cur_w = ww
            else:
                broken = _hard_break_word(word, max_w)
                lines.extend(broken[:-1])
                cur = [broken[-1]]
                cur_w = cell_len(broken[-1])
        elif cur_w + 1 + ww <= max_w:
            cur.append(" " + word)
            cur_w += 1 + ww
        else:
            lines.append("".join(cur))
            if ww <= max_w:
                cur = [word]
                cur_w = ww
            else:
                broken = _hard_break_word(word, max_w)
                lines.extend(broken[:-1])
                cur = [broken[-1]]
                cur_w = cell_len(broken[-1])
    if cur:
        lines.append("".join(cur))
    return lines or [text[:max_w]]


@dataclass(frozen=True, slots=True)
class WrappedLyricRow:
    orig_idx: int
    text: str
    is_translation: bool = False
    is_spacer: bool = False


def wrap_bilingual_lyric_lines(
    lines: Sequence[object],
    width: int,
    *,
    show_translation: bool = False,
    blank_between: bool = False,
    pad_left: int = 0,
) -> list[WrappedLyricRow]:
    """Wrap LyricLine items with optional Indonesian translation lines and optional stanza gap."""
    wrap_w = max(8, width - max(0, pad_left))
    rows: list[WrappedLyricRow] = []
    has_tr = show_translation and any(
        (getattr(ln, "translation", "") or "").strip() for ln in lines
    )
    last_i = len(lines) - 1

    for idx, line in enumerate(lines):
        raw = (getattr(line, "text", "") or "···").strip() or "···"
        rows.extend(WrappedLyricRow(idx, part) for part in wrap_cells(raw, wrap_w))

        if has_tr:
            tr = (getattr(line, "translation", "") or "").strip()
            if tr and tr.casefold() != raw.casefold():
                rows.extend(
                    WrappedLyricRow(idx, part, is_translation=True)
                    for part in wrap_cells(tr, wrap_w)
                )
            if blank_between and idx < last_i:
                rows.append(WrappedLyricRow(-1, "", is_spacer=True))
    return rows


def format_lyric_row(
    row: WrappedLyricRow,
    *,
    pad_left: int = 0,
    is_act: bool = False,
    is_past: bool = False,
) -> tuple[str, str]:
    """Return (text, style) for lyrics lines with active playback highlight only."""
    from musick.ui.theme import ACCENT, DIM, TEXT, TR_ACTIVE, TR_PAST, TR_UPCOMING

    if row.is_spacer:
        return ("", "")

    txt = f"{' ' * max(0, pad_left)}{row.text}"
    if row.is_translation:
        if is_act:
            return (txt, TR_ACTIVE)
        if is_past:
            return (txt, TR_PAST)
        return (txt, TR_UPCOMING)

    if is_act:
        return (txt, f"bold {ACCENT}")
    if is_past:
        return (txt, DIM)
    return (txt, TEXT)
