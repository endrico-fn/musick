"""Shared base class and layout/task helpers for all main views."""

from __future__ import annotations

import asyncio
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

from rich.text import Text
from textual.containers import VerticalScroll
from textual.widget import Widget
from textual.widgets import Static

from musick.domain import Track
from musick.player.controller import PlaybackState
from musick.ui.messages import NotifyStatus, StartRadio
from musick.ui.theme import ACCENT, DIM, IconSet

if TYPE_CHECKING:
    from musick.player.controller import PlayerEngine
from musick.ui.widgets import (
    SHIMMER_CLOCK,
    CoverWidget,
    Skeleton,
    dot_frame,
    fit_square_dims,
)


def playing_index_for(tracks: Sequence[Track], playing_vid: str | None) -> int | None:
    """Return index of currently playing video_id in `tracks`, or None."""
    if not playing_vid:
        return None
    for i, t in enumerate(tracks):
        if t.video_id == playing_vid:
            return i
    return None


def cancel_tasks(*tasks: asyncio.Task[Any] | None) -> None:
    """Safely cancel any active asyncio tasks."""
    for task in tasks:
        if task is not None and not task.done():
            task.cancel()


def compute_side_panel_geometry(
    view_w: int,
    view_h: int,
    *,
    min_bottom_h: int,
    min_side_w: int = 26,
    max_side_cap: int = 64,
    width_ratio: float = 0.34,
) -> tuple[int, int, int, int]:
    """Compute (side_w, top_panel_h, cov_w, cov_h) so a 1:1 cover fits cleanly to the border."""
    w = max(40, int(view_w))
    h = max(12, int(view_h))
    max_side_w = max(min_side_w, min(max_side_cap, round(w * width_ratio)))
    max_top_h = max(6, h - min_bottom_h)
    max_cov_h = max(4, max_top_h - 2)
    max_cov_w = max(8, max_side_w - 4)
    cov_w, cov_h = fit_square_dims(max_cov_w, max_cov_h)
    side_w = max(min_side_w, min(max_side_w, cov_w + 4))
    top_panel_h = cov_h + 2
    return side_w, top_panel_h, cov_w, cov_h


def compute_hero_cover_dims(
    view_w: int,
    view_h: int,
    *,
    min_body_h: int = 8,
    min_cov_h: int = 4,
    max_cov_h: int = 10,
    max_width_ratio: float = 0.28,
    min_panel_w: int = 14,
) -> tuple[int, int, int]:
    """Compute (cover_panel_w, cov_w, cov_h) for a Hero header above full-width content."""
    w = max(48, int(view_w))
    h = max(12, int(view_h))
    avail_cov_h = max(min_cov_h, min(max_cov_h, h - min_body_h - 2, round(h * 0.34)))
    max_cov_w = max(8, round(w * max_width_ratio) - 4)
    cov_w, cov_h = fit_square_dims(max_cov_w, avail_cov_h, min_w=8, min_h=min_cov_h)
    panel_w = max(min_panel_w, cov_w + 4) if w >= 56 else cov_w + 4
    return panel_w, cov_w, cov_h


@dataclass(frozen=True, slots=True)
class HeroIds:
    hero: str
    head_panel: str
    info_panel: str
    body_panel: str
    head_skel: str
    info_skel: str
    body_skel: str
    cover: str
    head_real: str
    title: str
    subtitle: str


class BaseView(VerticalScroll):
    """Scrollable content view inside the persistent shell."""

    DEFAULT_CSS = """
    BaseView {
        height: 1fr;
        background: transparent;
        scrollbar-size: 1 1;
    }
    """

    tab_id: str = "home"
    footer_hints: tuple[tuple[str, str], ...] = (
        ("/", "search"),
        ("Tab", "focus"),
        ("Enter", "open"),
        ("Space", "play/pause"),
        ("q", "quit"),
    )

    def __init__(self, **kw) -> None:
        super().__init__(**kw)
        self._dot_subs: dict[Widget, object] = {}
        self._saved_focus: Widget | None = None
        self._ui_ready: bool = False
        self._applied_settings_ver: int = 0

    def on_mount(self) -> None:
        self._ui_ready = True

    @property
    def ui_ready(self) -> bool:
        return self._ui_ready or self.is_mounted

    def save_focus(self, widget: Widget | None) -> None:
        self._saved_focus = widget

    def mark_settings_applied(self, version: int) -> None:
        self._applied_settings_ver = version

    def sync_settings_if_needed(
        self, version: int, icons: IconSet, cover_size: str, protocol: str
    ) -> None:
        if self._applied_settings_ver != version:
            self.on_settings_changed(icons, cover_size, protocol)
            self._applied_settings_ver = version

    def restore_focus(self) -> None:
        """Restore focus to the previously focused widget in this view when returning."""
        target = self._saved_focus
        if target is not None and target.is_mounted and target.display and target.can_focus:
            target.focus()
            return
        for w in self.query("*"):
            if w.can_focus and w.display:
                w.focus()
                return

    def on_player_state(self, state: PlaybackState) -> None:
        """Override in views that highlight the currently playing track."""

    def on_settings_changed(self, icons: IconSet, cover_size: str, protocol: str) -> None:
        """Override in views that adapt to updated settings."""

    def on_view_activated(self) -> None:
        """Called when this view becomes the active displayed view."""

    def on_view_deactivated(self) -> None:
        """Called when this view is hidden or removed."""

    def handle_input_escape(self) -> bool:
        """Handle Escape while an Input inside this view is focused. Return True if consumed."""
        return False

    def handle_escape(self) -> bool:
        """Handle Escape while focus is outside Input. Return True if consumed."""
        return False

    def handle_slash(self) -> bool:
        """Handle '/' shortcut inside this view. Return True if consumed."""
        return False

    def set_optional_text(
        self,
        widget_id: str,
        text: str,
        style: str = DIM,
        *,
        prefix: str = "",
    ) -> None:
        w = self.query_one(widget_id, Static)
        clean = text.strip()
        w.display = bool(clean)
        w.update(Text(f"{prefix}{clean}", style=style) if clean else "")

    def begin_hero_loading(self, ids: HeroIds) -> None:
        self.query_one(ids.cover).display = False
        self.query_one(ids.head_real).display = False
        self.bind_panel_loader(self.query_one(ids.head_panel), True)
        self.bind_panel_loader(self.query_one(ids.body_panel), True)

    def reveal_hero_header(self, ids: HeroIds, *, body_loading: bool = False) -> None:
        self.bind_panel_loader(self.query_one(ids.head_panel), False)
        self.bind_panel_loader(self.query_one(ids.body_panel), body_loading)
        self.query_one(ids.head_skel).display = False
        self.query_one(ids.info_skel).display = False
        self.query_one(ids.body_skel).display = False
        self.query_one(ids.cover).display = True
        self.query_one(ids.head_real).display = True

    def show_hero_error(
        self, ids: HeroIds, fallback_title: str, exc: Exception, *, populate_text: bool = True
    ) -> None:
        if not self.is_mounted:
            return
        self.reveal_hero_header(ids, body_loading=False)
        if populate_text:
            self.query_one(ids.title, Static).update(Text(fallback_title, style=f"bold {ACCENT}"))
            self.query_one(ids.subtitle, Static).update(Text(f"Failed to load: {exc}", style=DIM))

    def apply_hero_header(
        self,
        hero_id: str,
        head_panel_id: str,
        cover_id: str,
        skel_id: str | None = None,
        *,
        min_body_h: int = 8,
        max_cov_h: int = 10,
    ) -> tuple[int, tuple[int, int]]:
        """Size a horizontal Hero header (left square cover panel + right wide info panel)."""
        w = max(60, self.size.width or 96)
        h = max(12, self.size.height or 32)
        panel_w, cw, ch = compute_hero_cover_dims(
            w, h, min_body_h=min_body_h, max_cov_h=max_cov_h
        )
        hero_h = ch + 2
        self.query_one(hero_id).styles.height = hero_h
        head_panel = self.query_one(head_panel_id)
        head_panel.styles.width = panel_w
        head_panel.styles.height = hero_h
        self.query_one(cover_id, CoverWidget).set_exact_size(cw, ch)
        if skel_id:
            self.query_one(skel_id, Skeleton).set_cover_dims(cw, ch)
        return panel_w, (cw, ch)

    def bind_panel_loader(self, panel: Widget, active: bool, label: str = "Loading") -> None:
        old = self._dot_subs.pop(panel, None)
        if old is not None and callable(old):
            old()
        if not active:
            panel.border_subtitle = ""
            return

        def update_sub(tick: int) -> None:
            if panel.is_mounted and self.display:
                panel.border_subtitle = f"{label} {dot_frame(tick, SHIMMER_CLOCK.reduced_motion)}"

        update_sub(SHIMMER_CLOCK.tick)
        self._dot_subs[panel] = SHIMMER_CLOCK.subscribe(update_sub)

    def clear_panel_loaders(self) -> None:
        for unsub in self._dot_subs.values():
            if callable(unsub):
                unsub()
        self._dot_subs.clear()

    def on_unmount(self) -> None:
        self._ui_ready = False
        self.clear_panel_loaders()


def format_footer_hints(hints: tuple[tuple[str, str], ...]) -> str:
    return "  ".join(f"[{ACCENT}]{k}[/] [{DIM}]{v}[/]" for k, v in hints)


def handle_queue_shortcut(
    view: BaseView,
    engine: PlayerEngine,
    key: str,
    cursor: int,
    *,
    set_cursor: Callable[[int], None],
    on_refresh: Callable[[], None],
    current_track: Track | None = None,
) -> bool:
    """Handle queue manipulation keyboard shortcuts shared by HomeView and QueueView."""
    q = engine.queue
    if key == "s":
        engine.toggle_shuffle()
        on_refresh()
        return True
    if key == "r":
        engine.cycle_repeat()
        on_refresh()
        return True
    if key == "c":
        q.clear()
        engine.notify_queue_changed()
        on_refresh()
        return True
    if key == "C":
        q.clear_upcoming()
        engine.notify_queue_changed()
        on_refresh()
        return True
    if key == "D":
        removed = q.deduplicate()
        engine.notify_queue_changed()
        on_refresh()
        view.post_message(
            NotifyStatus(f"Removed {removed} duplicate(s)" if removed else "No duplicates")
        )
        return True
    if key == "z":
        if q.undo():
            engine.notify_queue_changed()
            on_refresh()
            view.post_message(NotifyStatus("Restored queue state"))
        return True
    if key == "T" and q.tracks:
        if q.move_to_next(cursor):
            if q.index >= 0:
                set_cursor(q.index + 1)
            engine.notify_queue_changed()
            on_refresh()
            view.post_message(NotifyStatus("Moved to Next Up"))
        return True
    if key in ("period", "x") and q.index >= 0:
        set_cursor(q.index)
        return True
    if key == "R" and isinstance(current_track, Track):
        view.post_message(StartRadio(video_id=current_track.video_id))
        return True
    if key in ("J", "shift+down") and len(q.tracks) > 1:
        dst = min(len(q.tracks) - 1, cursor + 1)
        if q.move(cursor, dst):
            set_cursor(dst)
            engine.notify_queue_changed()
            on_refresh()
        return True
    if key in ("K", "shift+up") and len(q.tracks) > 1:
        dst = max(0, cursor - 1)
        if q.move(cursor, dst):
            set_cursor(dst)
            engine.notify_queue_changed()
            on_refresh()
        return True
    return False
