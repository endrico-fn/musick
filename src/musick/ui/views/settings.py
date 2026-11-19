"""Settings page: Privacy (no-cookie default locked), Display + Region, Player, and Lyrics."""

from __future__ import annotations

from rich.text import Text
from textual.app import ComposeResult
from textual.containers import Horizontal, Vertical
from textual.events import Click, Key
from textual.message import Message
from textual.widget import Widget
from textual.widgets import Button, Static

from musick.config import (
    AUDIO_QUALITIES,
    CAVA_AUDIO_SOURCES,
    CAVA_PALETTES,
    CAVA_SMOOTHINGS,
    COVER_SIZES,
    ICON_MODES,
    IMAGE_PROTOCOLS,
    LYRICS_SOURCES,
    REGIONS,
    Settings,
)
from musick.config import (
    save as save_settings,
)
from musick.paths import clear_local_data, settings_file
from musick.ui.messages import NotifyStatus, SettingsUpdated
from musick.ui.text_utils import trunc_cells
from musick.ui.theme import ACCENT, DIM, TEXT, IconSet, icons_for

from .base import BaseView

_REGION_NAMES: dict[str, str] = {
    "ID": "ID",
    "US": "US",
    "GB": "GB",
    "JP": "JP",
    "KR": "KR",
    "ZZ": "Global",
}


_TOGGLE_BINDINGS: dict[str, tuple[str, str]] = {
    "tog-history": ("privacy", "save_history"),
    "tog-cache-img": ("privacy", "cache_images"),
    "tog-cache-lyr": ("privacy", "cache_lyrics"),
    "tog-remote-sugg": ("privacy", "remote_suggestions"),
    "tog-cava-on": ("display", "cava_enabled"),
    "tog-reduced-motion": ("display", "reduced_motion"),
    "tog-autoplay": ("player", "autoplay"),
    "tog-normalize": ("player", "normalize_audio"),
    "tog-prefetch": ("player", "prefetch"),
    "tog-gapless": ("player", "gapless"),
    "tog-mpv-cfg": ("player", "use_mpv_config"),
    "tog-lyr-on": ("lyrics", "enabled"),
    "tog-lyr-synced": ("lyrics", "synced_only"),
    "tog-lyr-translate": ("lyrics", "translate_to_id"),
}

_BUTTON_GROUPS: tuple[tuple[str, str, str, tuple[str, ...]], ...] = (
    ("cov-", "display", "cover_size", COVER_SIZES),
    ("reg-", "display", "region", REGIONS),
    ("proto-", "display", "image_protocol", IMAGE_PROTOCOLS),
    ("icon-", "display", "icons", ICON_MODES),
    ("cvpal-", "display", "cava_palette", CAVA_PALETTES),
    ("cvsm-", "display", "cava_smoothing", CAVA_SMOOTHINGS),
    ("csrc-", "display", "cava_audio_source", CAVA_AUDIO_SOURCES),
    ("qual-", "player", "audio_quality", AUDIO_QUALITIES),
    ("lyrsrc-", "lyrics", "source", LYRICS_SOURCES),
)


class SettingToggle(Widget):
    """Left-aligned, zero-background checkbox row (only focused state gets cursor bg)."""

    can_focus = True

    class Toggled(Message):
        def __init__(self, toggle: SettingToggle) -> None:
            super().__init__()
            self.toggle = toggle

    def __init__(self, label: str, value: bool = False, icons: IconSet | None = None, **kw) -> None:
        super().__init__(**kw)
        self.label = label
        self.value = value
        self.icons = icons or icons_for("unicode")

    def set_state(self, value: bool, icons: IconSet | None = None) -> None:
        self.value = value
        if icons is not None:
            self.icons = icons
        self.refresh()

    def press(self) -> None:
        self.post_message(self.Toggled(self))

    def render(self) -> Text:
        w = max(12, self.size.width or 40)
        mark = self.icons.check_on if self.value else self.icons.check_off
        prefix_len = len(mark) + 1
        avail = max(4, w - prefix_len)
        label = trunc_cells(self.label, avail)
        txt = Text(no_wrap=True, overflow="ignore")
        txt.append(f"{mark} ", style=f"bold {ACCENT}" if self.value else DIM)
        txt.append(label, style=TEXT)
        return txt

    def on_key(self, event: Key) -> None:
        if event.key in ("enter", "space"):
            self.press()
            event.stop()

    def on_click(self, event: Click) -> None:
        self.focus()
        self.press()


class SettingsView(BaseView):
    tab_id = "settings"
    footer_hints = (
        ("Tab", "next control"),
        ("Enter / Space", "toggle"),
        ("Ctrl+s", "save"),
        ("Bksp", "back"),
    )

    def __init__(self, settings: Settings, on_clear_local_data=None, **kw) -> None:
        super().__init__(**kw)
        self._settings = settings
        self._on_clear_local_data = on_clear_local_data
        self._icons: IconSet = icons_for(settings.display.icons)

    def compose(self) -> ComposeResult:
        with Horizontal(classes="split-row"):
            with Vertical(id="set-priv-panel", classes="panel split-col-left") as pp:
                pp.border_title = "Privacy"
                yield Static("", id="set-no-cookie-lock")
                yield SettingToggle("Save search & play history", id="tog-history")
                yield SettingToggle("Cache cover images on disk", id="tog-cache-img")
                yield SettingToggle("Cache lyrics on disk", id="tog-cache-lyr")
                yield SettingToggle("Remote search suggestions", id="tog-remote-sugg")
                yield Static("")
                yield Button("Clear All Local Data", id="btn-clear-data")

            with Vertical(id="set-disp-panel", classes="panel split-col-right") as dp:
                dp.border_title = "Display, Cava & Region"
                with Horizontal(classes="compact-row"):
                    yield Static(Text("Cover:", style=DIM), classes="inline-label")
                    for s in COVER_SIZES:
                        yield Button(s, id=f"cov-{s}")
                    yield Static(Text("Icons:", style=DIM), classes="inline-label")
                    for m in ICON_MODES:
                        yield Button(m.title(), id=f"icon-{m}")
                with Horizontal(classes="compact-row"):
                    yield Static(Text("Region:", style=DIM), classes="inline-label")
                    for r in REGIONS:
                        yield Button(_REGION_NAMES[r], id=f"reg-{r}")
                with Horizontal(classes="compact-row"):
                    yield Static(Text("Proto:", style=DIM), classes="inline-label")
                    for p in IMAGE_PROTOCOLS:
                        yield Button(p.title(), id=f"proto-{p}")
                yield SettingToggle("Show live Cava spectrum", id="tog-cava-on")
                with Horizontal(classes="compact-row"):
                    yield Static(Text("Source:", style=DIM), classes="inline-label")
                    for src in CAVA_AUDIO_SOURCES:
                        yield Button(src.title(), id=f"csrc-{src}")
                with Horizontal(classes="compact-row"):
                    yield Static(Text("Theme:", style=DIM), classes="inline-label")
                    for pal in CAVA_PALETTES:
                        yield Button(pal.title(), id=f"cvpal-{pal}")
                with Horizontal(classes="compact-row"):
                    yield Static(Text("Motion:", style=DIM), classes="inline-label")
                    for sm in CAVA_SMOOTHINGS:
                        yield Button(sm.title(), id=f"cvsm-{sm}")
                yield SettingToggle(
                    "Reduced motion (static skeletons & Cava)", id="tog-reduced-motion"
                )

        with Horizontal(classes="split-row"):
            with Vertical(id="set-play-panel", classes="panel split-col-left") as plp:
                plp.border_title = "Player & Audio"
                yield SettingToggle("Autoplay radio at queue end", id="tog-autoplay")
                yield SettingToggle("Normalize track volume (dynaud)", id="tog-normalize")
                with Horizontal(classes="compact-row"):
                    yield Static(Text("Quality:", style=DIM), classes="inline-label")
                    for q in AUDIO_QUALITIES:
                        yield Button(q.title(), id=f"qual-{q}")
                yield SettingToggle("Prefetch next track stream", id="tog-prefetch")
                yield SettingToggle("Gapless playback", id="tog-gapless")
                yield SettingToggle("Use system mpv config", id="tog-mpv-cfg")
                with Horizontal(classes="compact-row"):
                    yield Static(Text("Cache:", style=DIM), classes="inline-label")
                    yield Button("-", id="cache-dec")
                    yield Static("", id="val-cache", classes="inline-label")
                    yield Button("+", id="cache-inc")
                    yield Static(Text("Vol:", style=DIM), classes="inline-label")
                    yield Button("-", id="vol-dec")
                    yield Static("", id="val-vol", classes="inline-label")
                    yield Button("+", id="vol-inc")

            with Vertical(id="set-lyr-panel", classes="panel split-col-right") as lp:
                lp.border_title = "Lyrics"
                yield Static(Text("Source priority", style=DIM))
                with Horizontal(classes="compact-row"):
                    for src_key in LYRICS_SOURCES:
                        lbl = (
                            "YouTube"
                            if src_key == "youtube"
                            else ("LRCLIB" if src_key == "lrclib" else "Auto")
                        )
                        yield Button(lbl, id=f"lyrsrc-{src_key}")
                yield Static(Text("Auto: LRCLIB + YouTube Music + Captions", style=DIM))
                yield SettingToggle("Enable lyrics fetching", id="tog-lyr-on")
                yield SettingToggle("Synced lyrics only", id="tog-lyr-synced")
                yield SettingToggle("Translate lyrics to Indonesian [t]", id="tog-lyr-translate")

        with Horizontal(classes="compact-row"):
            yield Button("Save Settings", id="btn-save-settings", classes="-primary")
            yield Button("Reset Defaults", id="btn-reset-settings")
        yield Static("", id="set-status")

    def on_mount(self) -> None:
        super().on_mount()
        self._sync_ui()

    def on_view_activated(self) -> None:
        if self.is_mounted:
            self._sync_ui()

    def on_descendant_focus(self, event) -> None:
        if self.is_mounted and event.widget is not None:
            self.scroll_to_widget(event.widget, animate=False)

    def _sync_ui(self) -> None:
        self._icons = icons_for(self._settings.display.icons)
        s = self._settings

        lock_txt = Text()
        lock_txt.append(f"{self._icons.check_on} ", style=f"bold {ACCENT}")
        lock_txt.append("No cookies (enforced, locked)", style=ACCENT)
        self.query_one("#set-no-cookie-lock", Static).update(lock_txt)

        for tog_id, (sec_name, field_name) in _TOGGLE_BINDINGS.items():
            sec = getattr(s, sec_name)
            self.query_one(f"#{tog_id}", SettingToggle).set_state(
                bool(getattr(sec, field_name)), self._icons
            )

        for prefix, sec_name, field_name, options in _BUTTON_GROUPS:
            cur = getattr(getattr(s, sec_name), field_name)
            for opt in options:
                self.query_one(f"#{prefix}{opt}", Button).set_class(cur == opt, "is-selected")

        self.query_one("#val-cache", Static).update(Text(f" {s.player.cache_secs}s ", style=TEXT))
        self.query_one("#val-vol", Static).update(Text(f" {s.player.volume}% ", style=TEXT))

    def on_setting_toggle_toggled(self, event: SettingToggle.Toggled) -> None:
        bid = event.toggle.id or ""
        binding = _TOGGLE_BINDINGS.get(bid)
        if binding is None:
            return
        sec_name, field_name = binding
        s = self._settings
        sec = getattr(s, sec_name)
        setattr(sec, field_name, not getattr(sec, field_name))
        s.normalize()
        save_settings(s, settings_file())
        self._sync_ui()
        self.post_message(SettingsUpdated())

    def on_button_pressed(self, event: Button.Pressed) -> None:
        bid = event.button.id or ""
        s = self._settings

        handled_group = False
        for prefix, sec_name, field_name, _ in _BUTTON_GROUPS:
            if bid.startswith(prefix):
                setattr(getattr(s, sec_name), field_name, bid.removeprefix(prefix))
                handled_group = True
                break

        if handled_group:
            pass
        elif bid == "cache-dec":
            s.player.cache_secs = max(5, s.player.cache_secs - 5)
        elif bid == "cache-inc":
            s.player.cache_secs = min(300, s.player.cache_secs + 5)
        elif bid == "vol-dec":
            s.player.volume = max(0, s.player.volume - 5)
        elif bid == "vol-inc":
            s.player.volume = min(130, s.player.volume + 5)
        elif bid == "btn-clear-data":
            removed = clear_local_data(keep_settings=True)
            if self._on_clear_local_data:
                self._on_clear_local_data()
            msg = f"Cleared local data ({len(removed)} folders removed)."
            self.query_one("#set-status", Static).update(Text(msg, style=ACCENT))
            self.post_message(NotifyStatus(msg))
            self._sync_ui()
            return
        elif bid == "btn-reset-settings":
            fresh = Settings()
            s.privacy = fresh.privacy
            s.display = fresh.display
            s.player = fresh.player
            s.lyrics = fresh.lyrics
            save_settings(s, settings_file())
            self._sync_ui()
            self.post_message(SettingsUpdated())
            self.query_one("#set-status", Static).update(
                Text("Defaults restored and saved.", style=ACCENT)
            )
            return
        elif bid == "btn-save-settings":
            save_settings(s, settings_file())
            self._sync_ui()
            self.post_message(SettingsUpdated())
            self.query_one("#set-status", Static).update(
                Text(f"Saved to {settings_file()}", style=ACCENT)
            )
            return
        else:
            return

        s.normalize()
        save_settings(s, settings_file())
        self._sync_ui()
        self.post_message(SettingsUpdated())
