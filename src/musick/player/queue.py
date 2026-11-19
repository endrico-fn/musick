"""Two-tier playback queue: history, active track, priority Next Up, context, and undo."""

from __future__ import annotations

import contextlib
import random
from collections.abc import Iterable
from typing import Literal

from musick.domain import Track, format_duration

RepeatMode = Literal["off", "all", "one"]
QueueRole = Literal["played", "playing", "next", "queued"]
_REPEAT_ORDER: tuple[RepeatMode, ...] = ("off", "all", "one")


class PlayQueue:
    """Two-tier playback queue preserving priority 'Next Up' tracks over context tracks."""

    MAX_UNDO = 12

    def __init__(self, rng: random.Random | None = None) -> None:
        self._rng = rng or random.Random()
        self._items: list[Track] = []
        self._original: list[Track] = []
        self.index: int = -1
        self.shuffle: bool = False
        self.repeat: RepeatMode = "off"
        self._priority_count: int = 0
        self._undo_stack: list[tuple[list[Track], list[Track], int, int]] = []

    def _snapshot(self) -> None:
        if not self._items:
            return
        self._undo_stack.append(
            (list(self._items), list(self._original), self.index, self._priority_count)
        )
        if len(self._undo_stack) > self.MAX_UNDO:
            self._undo_stack.pop(0)

    def can_undo(self) -> bool:
        return bool(self._undo_stack)

    def undo(self) -> bool:
        """Restore previous queue state from undo stack."""
        if not self._undo_stack:
            return False
        items, orig, idx, prio = self._undo_stack.pop()
        self._items = items
        self._original = orig
        self.index = idx
        self._priority_count = prio
        return True

    @property
    def tracks(self) -> tuple[Track, ...]:
        return tuple(self._items)

    @property
    def priority_count(self) -> int:
        if self.index < 0 or self.index >= len(self._items):
            return 0
        avail = max(0, len(self._items) - self.index - 1)
        return min(self._priority_count, avail)

    @property
    def upcoming_tracks(self) -> tuple[Track, ...]:
        if self.index < 0:
            return tuple(self._items)
        return tuple(self._items[self.index + 1 :])

    @property
    def remaining_seconds(self) -> int:
        return sum(t.duration for t in self.upcoming_tracks if t.duration and t.duration > 0)

    def role_at(self, idx: int) -> QueueRole:
        if idx == self.index:
            return "playing"
        if 0 <= idx < self.index:
            return "played"
        if self.index >= 0 and self.index < idx <= self.index + self.priority_count:
            return "next"
        return "queued"

    def summary_text(self) -> str:
        total = len(self._items)
        if not total:
            return "Empty"
        up_count = len(self.upcoming_tracks)
        rem = format_duration(self.remaining_seconds) if self.remaining_seconds > 0 else ""
        parts: list[str] = []
        if up_count:
            lbl = f"{up_count} up next"
            if self.priority_count:
                lbl += f" ({self.priority_count} prioritized)"
            parts.append(lbl)
        else:
            parts.append(f"{total} track" + ("s" if total != 1 else ""))
        if rem:
            parts.append(rem)
        if self.shuffle:
            parts.append("Shuffle")
        if self.repeat != "off":
            parts.append(f"Repeat:{self.repeat}")
        return " · ".join(parts)

    def __len__(self) -> int:
        return len(self._items)

    @property
    def current(self) -> Track | None:
        if 0 <= self.index < len(self._items):
            return self._items[self.index]
        return None

    def set_tracks(self, tracks: Iterable[Track], start: int = 0) -> Track | None:
        items = list(tracks)
        self._original = list(items)
        self._priority_count = 0
        if not items:
            self._items = []
            self.index = -1
            return None
        start = max(0, min(start, len(items) - 1))
        if self.shuffle:
            chosen = items[start]
            rest = items[:start] + items[start + 1 :]
            self._rng.shuffle(rest)
            self._items = [chosen, *rest]
            self.index = 0
        else:
            self._items = items
            self.index = start
        return self.current

    def append(self, tracks: Iterable[Track]) -> None:
        added = list(tracks)
        if not added:
            return
        self._original.extend(added)
        if self.shuffle:
            pool = list(added)
            self._rng.shuffle(pool)
            self._items.extend(pool)
        else:
            self._items.extend(added)
        if self.index < 0 and self._items:
            self.index = 0

    def play_next(self, track: Track) -> None:
        """Insert track into immediate Next Up priority slot (after current track)."""
        self.play_next_many([track])

    def play_next_many(self, tracks: Iterable[Track]) -> None:
        added = list(tracks)
        if not added:
            return
        if not self._items:
            self.set_tracks(added, 0)
            self._priority_count = max(0, len(added) - 1)
            return
        pos = max(0, self.index + 1)
        for offset, trk in enumerate(added):
            self._items.insert(pos + offset, trk)
            self._original.append(trk)
        self._priority_count += len(added)

    def remove(self, idx: int) -> None:
        if not (0 <= idx < len(self._items)):
            return
        self._snapshot()
        prio_end = self.index + self.priority_count
        removed = self._items.pop(idx)
        with contextlib.suppress(ValueError):
            self._original.remove(removed)
        if self.index >= 0 and self.index < idx <= prio_end:
            self._priority_count = max(0, self._priority_count - 1)
        if not self._items:
            self.index = -1
            self._priority_count = 0
        elif idx < self.index:
            self.index -= 1
        elif idx == self.index and self.index >= len(self._items):
            self.index = len(self._items) - 1
            self._priority_count = 0

    def move_to_next(self, idx: int) -> bool:
        """Promote any queued track right after the active track as immediate Next Up."""
        if not (0 <= idx < len(self._items)) or self.index < 0 or idx == self.index:
            return False
        dst = self.index + 1 if idx > self.index else self.index
        return self.move(idx, dst)

    def deduplicate(self) -> int:
        """Remove duplicate upcoming tracks (by video_id) while preserving order and active."""
        if not self._items:
            return 0
        self._snapshot()
        seen: set[str] = set()
        if self.current is not None:
            seen.add(self.current.video_id)
        head = self._items[: self.index + 1] if self.index >= 0 else []
        prio_end = self.index + self.priority_count
        new_items = list(head)
        new_prio = 0
        removed = 0
        for i, trk in enumerate(self._items[len(head) :], start=len(head)):
            if trk.video_id in seen:
                removed += 1
                with contextlib.suppress(ValueError):
                    self._original.remove(trk)
                continue
            seen.add(trk.video_id)
            new_items.append(trk)
            if self.index >= 0 and i <= prio_end:
                new_prio += 1
        self._items = new_items
        self._priority_count = new_prio
        return removed

    def clear_upcoming(self) -> int:
        """Clear upcoming tracks while leaving played history and active track intact."""
        if self.index < 0 or self.index + 1 >= len(self._items):
            return 0
        self._snapshot()
        removed = len(self._items) - (self.index + 1)
        kept = self._items[: self.index + 1]
        self._items = kept
        if not self.shuffle:
            self._original = list(kept)
        else:
            kept_set = set(kept)
            self._original = [t for t in self._original if t in kept_set]
        self._priority_count = 0
        return removed

    def move(self, src: int, dst: int) -> bool:
        n = len(self._items)
        if not (0 <= src < n and 0 <= dst < n) or src == dst:
            return False
        prio_end = self.index + self.priority_count
        was_src_prio = self.index >= 0 and self.index < src <= prio_end
        item = self._items.pop(src)
        self._items.insert(dst, item)
        if self.index == src:
            self.index = dst
            self._priority_count = 0
        elif src < self.index <= dst:
            self.index -= 1
        elif dst <= self.index < src:
            self.index += 1

        if self.index >= 0 and src != self.index:
            if was_src_prio and (dst <= self.index or dst > prio_end):
                self._priority_count = max(0, self._priority_count - 1)
            elif not was_src_prio and (self.index < dst <= self.index + self.priority_count + 1):
                self._priority_count += 1
        if not self.shuffle and 0 <= src < len(self._original) and 0 <= dst < len(self._original):
            self._original = list(self._items)
        return True

    def clear(self) -> None:
        self._snapshot()
        self._items.clear()
        self._original.clear()
        self.index = -1
        self._priority_count = 0

    def jump(self, idx: int) -> Track | None:
        if 0 <= idx < len(self._items):
            prio_end = self.index + self.priority_count
            if self.index >= 0 and self.index < idx <= prio_end:
                self._priority_count = prio_end - idx
            else:
                self._priority_count = 0
            self.index = idx
            return self.current
        return None

    def peek_next(self) -> Track | None:
        if not self._items:
            return None
        if self.repeat == "one":
            return self.current
        if self.index + 1 < len(self._items):
            return self._items[self.index + 1]
        if self.repeat == "all":
            return self._items[0]
        return None

    def advance(self, manual: bool = False) -> Track | None:
        if not self._items:
            return None
        if self.repeat == "one" and not manual:
            return self.current
        if self.index + 1 < len(self._items):
            self.index += 1
            if self._priority_count > 0:
                self._priority_count -= 1
            return self.current
        if self.repeat == "all":
            self.index = 0
            self._priority_count = 0
            return self.current
        return None

    def retreat(self) -> Track | None:
        if not self._items:
            return None
        if self.index - 1 >= 0:
            self.index -= 1
            self._priority_count = 0
            return self.current
        if self.repeat == "all":
            self.index = len(self._items) - 1
            self._priority_count = 0
            return self.current
        self.index = 0
        return self.current

    def toggle_shuffle(self) -> bool:
        self.shuffle = not self.shuffle
        cur = self.current
        if not self._items:
            return self.shuffle
        if self.shuffle:
            if cur is not None:
                prio_n = self.priority_count
                prio_slice = self._items[self.index + 1 : self.index + 1 + prio_n]
                rest = [
                    t
                    for i, t in enumerate(self._items)
                    if i != self.index and not (self.index < i <= self.index + prio_n)
                ]
                self._rng.shuffle(rest)
                self._items = [cur, *prio_slice, *rest]
                self.index = 0
            else:
                self._rng.shuffle(self._items)
        else:
            self._items = list(self._original)
            self._priority_count = 0
            if cur is not None:
                for i, t in enumerate(self._items):
                    if t.video_id == cur.video_id:
                        self.index = i
                        break
        return self.shuffle

    def cycle_repeat(self) -> RepeatMode:
        idx = _REPEAT_ORDER.index(self.repeat)
        self.repeat = _REPEAT_ORDER[(idx + 1) % len(_REPEAT_ORDER)]
        return self.repeat
