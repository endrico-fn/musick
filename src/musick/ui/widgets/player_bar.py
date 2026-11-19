"""Persistent bottom Now Playing control bar (without mini cover album)."""

from __future__ import annotations

from rich.text import Text
from textual.app import ComposeResult
from textual.containers import Container, Horizontal, Vertical
from textual.message import Message
from textual.widgets import Button, Static

from musick.player.controller import PlaybackState
from musick.ui.theme import ACCENT, BORDER, DIM, ERR, TEXT, UNICODE_ICONS, IconSet

from .loading import SHIMMER_CLOCK, dot_frame
from .progress_bar import SeekBar


class NowPlayingBar(Container):
    """Fixed 5-row sharp-bordered playback control bar without mini cover."""

    DEFAULT_CSS = """
    NowPlayingBar {
        height: 5;
    }
    NowPlayingBar > #np-inner {
        layout: vertical;
        height: 3;
    }
    NowPlayingBar #np-title {
        height: 1;
        width: 1fr;
    }
    NowPlayingBar #np-controls {
        layout: horizontal;
        height: 1;
    }
    """

    class ActionTriggered(Message):
        def __init__(self, action: str) -> None:
            super().__init__()
            self.action = action

    def __init__(
        self,
        icons: IconSet = UNICODE_ICONS,
        **kw,
    ) -> None:
        kw.setdefault("id", "now-playing")
        super().__init__(**kw)
        self.border_title = "Player"
        self._icons = icons
        self._state = PlaybackState()
        self._unsub = None
        self._title_key: tuple[str | None, str | None] | None = None
        self._last_volume: int | None = None

    def compose(self) -> ComposeResult:
        with Vertical(id="np-inner"):
            yield Static(Text("Nothing playing  ·  Press / to search", style=DIM), id="np-title")
            with Horizontal(id="np-controls"):
                yield Button(self._icons.prev, id="np-prev")
                yield Button(self._icons.playing, id="np-play", classes="-primary")
                yield Button(self._icons.next, id="np-next")
                yield Button(self._icons.shuffle, id="np-shf")
                yield Button(self._icons.repeat_off, id="np-rpt")
                yield Static("", id="np-spacer")
                yield Static("", id="np-vol")
            yield SeekBar(id="np-seek")

    def on_mount(self) -> None:
        self._render_volume()

    def on_unmount(self) -> None:
        if self._unsub is not None:
            self._unsub()
            self._unsub = None

    def set_icons(self, icons: IconSet) -> None:
        self._icons = icons
        self.query_one("#np-prev", Button).label = icons.prev
        self.query_one("#np-next", Button).label = icons.next
        self.query_one("#np-shf", Button).label = icons.shuffle
        self.apply_state(self._state)

    def apply_state(self, state: PlaybackState) -> None:
        self._state = state
        busy = state.resolving or state.buffering
        self._sync_dot_loader(busy)

        title_w = self.query_one("#np-title", Static)
        title_key = (state.track.video_id if state.track else None, state.error)
        if title_key != self._title_key:
            self._title_key = title_key
            t_txt = Text(overflow="ellipsis", no_wrap=True)
            if state.error:
                t_txt.append(f"Error: {state.error}", style=ERR)
            elif state.track is None:
                t_txt.append("Nothing playing  ·  Press / to search", style=DIM)
            else:
                t_txt.append(state.track.title, style=f"bold {TEXT}")
                t_txt.append(f"  ·  {state.track.artist_names}", style=DIM)
                if state.track.album:
                    t_txt.append(f"  ·  {state.track.album}", style=DIM)
            title_w.update(t_txt)

        play_btn = self.query_one("#np-play", Button)
        new_play_lbl = (
            self._icons.pause if (state.playing and not state.paused) else self._icons.playing
        )
        if str(play_btn.label) != new_play_lbl:
            play_btn.label = new_play_lbl

        shf_btn = self.query_one("#np-shf", Button)
        shf_btn.set_class(state.shuffle, "is-selected")

        rpt_btn = self.query_one("#np-rpt", Button)
        new_rpt = (
            self._icons.repeat_one
            if state.repeat == "one"
            else self._icons.repeat_all
            if state.repeat == "all"
            else self._icons.repeat_off
        )
        if new_rpt != str(rpt_btn.label):
            rpt_btn.label = new_rpt
        rpt_btn.set_class(state.repeat != "off", "is-selected")

        self.query_one("#np-seek", SeekBar).update_progress(state.position, state.duration)
        self._render_volume()

    def _sync_dot_loader(self, busy: bool) -> None:
        if busy and self._unsub is None and self.is_mounted:
            self._unsub = SHIMMER_CLOCK.subscribe(self._on_dot_tick)
        elif not busy and self._unsub is not None:
            self._unsub()
            self._unsub = None
        if not busy:
            if self._state.restored and self._state.track:
                self.border_subtitle = "Last played"
            else:
                badges: list[str] = []
                if self._state.audio_info:
                    badges.append(self._state.audio_info)
                if self._state.normalize_audio:
                    badges.append("Norm")
                if self._state.autoplay_active:
                    badges.append("Radio")
                if self._state.upcoming_count > 0:
                    badges.append(f"+{self._state.upcoming_count} up next")
                self.border_subtitle = " · ".join(badges)
        else:
            self._on_dot_tick(SHIMMER_CLOCK.tick)

    def _on_dot_tick(self, tick: int) -> None:
        dots = dot_frame(tick, SHIMMER_CLOCK.reduced_motion)
        label = "Resolving" if self._state.resolving else "Buffering"
        self.border_subtitle = f"{label} {dots}"

    def _render_volume(self) -> None:
        vol = max(0, min(130, self._state.volume))
        if vol == self._last_volume:
            return
        self._last_volume = vol
        filled = min(10, max(0, round(vol / 10)))
        bar = "█" * filled + "░" * (10 - filled)
        bar_style = "#e0af68" if vol > 100 else ACCENT
        txt = Text(no_wrap=True, overflow="ignore")
        txt.append(f"{self._icons.volume} ", style=DIM)
        txt.append("▕", style=BORDER)
        txt.append(bar, style=bar_style)
        txt.append("▏", style=BORDER)
        txt.append(f"{vol:>4}%", style="#e0af68" if vol > 100 else DIM)
        self.query_one("#np-vol", Static).update(txt)

    def on_button_pressed(self, event: Button.Pressed) -> None:
        bid = event.button.id
        mapping = {
            "np-prev": "prev",
            "np-play": "toggle_pause",
            "np-next": "next",
            "np-shf": "toggle_shuffle",
            "np-rpt": "cycle_repeat",
        }
        if bid in mapping:
            self.post_message(self.ActionTriggered(mapping[bid]))
