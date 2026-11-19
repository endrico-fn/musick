"""Shared Textual scroll helper for custom cursor-driven widgets inside scrollable containers."""

from __future__ import annotations

import contextlib

from textual.app import ScreenStackError
from textual.geometry import Region
from textual.widget import NoScreen, Widget


def scroll_child_region(widget: Widget, region: Region, *, animate: bool = False) -> None:
    """Scroll ancestor containers so `region` (in `widget` local coordinates) is visible."""
    if not widget.is_mounted or not widget.size:
        return
    cur: Widget = widget
    target = Region(
        cur.virtual_region.x + region.x,
        cur.virtual_region.y + region.y,
        max(1, region.width),
        max(1, region.height),
    )
    with contextlib.suppress(NoScreen, ScreenStackError):
        while isinstance(cur.parent, Widget):
            container = cur.parent
            offset = container.scroll_to_region(target, animate=animate)
            target = (
                target.translate(-offset)
                .translate(container.styles.margin.top_left)
                .translate(container.styles.border.spacing.top_left)
                .translate(container.virtual_region_with_margin.offset)
            )
            cur = container
            if getattr(container, "is_screen", False):
                break
