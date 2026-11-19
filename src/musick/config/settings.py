"""User settings: typed dataclasses, TOML persistence. Privacy-first defaults."""

from __future__ import annotations

import tomllib
from dataclasses import asdict, dataclass, field, fields, is_dataclass
from pathlib import Path

from musick.paths import atomic_write_text

COVER_SIZES = ("S", "M", "L")
ICON_MODES = ("unicode", "nerd")
IMAGE_PROTOCOLS = ("auto", "tgp", "sixel", "halfcell")
REGIONS = ("ID", "US", "GB", "JP", "KR", "ZZ")
LYRICS_SOURCES = ("auto", "youtube", "lrclib")
CAVA_PALETTES = ("teal", "emerald", "aurora", "sunset", "mono")
CAVA_SMOOTHINGS = ("smooth", "balanced", "crisp")
CAVA_AUDIO_SOURCES = ("auto", "monitor", "off")
AUDIO_QUALITIES = ("best", "balanced", "saver")


@dataclass
class PrivacySettings:
    save_history: bool = False
    cache_images: bool = False
    cache_lyrics: bool = False
    remote_suggestions: bool = True
    # Account cookies are never used. Not configurable on purpose.


@dataclass
class DisplaySettings:
    cover_size: str = "M"
    icons: str = "unicode"
    image_protocol: str = "auto"
    region: str = "ID"
    reduced_motion: bool = False
    cava_enabled: bool = True
    cava_palette: str = "teal"
    cava_smoothing: str = "smooth"
    cava_audio_source: str = "auto"


@dataclass
class PlayerSettings:
    prefetch: bool = True
    gapless: bool = True
    autoplay: bool = True
    normalize_audio: bool = False
    audio_quality: str = "best"
    cache_secs: int = 30
    volume: int = 80
    use_mpv_config: bool = False


@dataclass
class LyricsSettings:
    enabled: bool = True
    synced_only: bool = False
    source: str = "auto"
    translate_to_id: bool = False


@dataclass
class Settings:
    privacy: PrivacySettings = field(default_factory=PrivacySettings)
    display: DisplaySettings = field(default_factory=DisplaySettings)
    player: PlayerSettings = field(default_factory=PlayerSettings)
    lyrics: LyricsSettings = field(default_factory=LyricsSettings)

    # ── validation ─────────────────────────────────────────────
    def normalize(self) -> Settings:
        if self.display.cover_size not in COVER_SIZES:
            self.display.cover_size = "M"
        if self.display.icons not in ICON_MODES:
            self.display.icons = "unicode"
        if self.display.image_protocol not in IMAGE_PROTOCOLS:
            self.display.image_protocol = "auto"
        if self.display.region not in REGIONS:
            self.display.region = "ID"
        if self.display.cava_audio_source not in CAVA_AUDIO_SOURCES:
            self.display.cava_audio_source = "auto"
        if self.display.cava_smoothing not in CAVA_SMOOTHINGS:
            self.display.cava_smoothing = "smooth"
        if self.display.cava_palette not in CAVA_PALETTES:
            self.display.cava_palette = "teal"
        if self.player.audio_quality not in AUDIO_QUALITIES:
            self.player.audio_quality = "best"
        if self.lyrics.source not in LYRICS_SOURCES:
            self.lyrics.source = "auto"
        self.player.cache_secs = max(5, min(300, int(self.player.cache_secs)))
        self.player.volume = max(0, min(130, int(self.player.volume)))
        return self


def _merge(section, data: dict) -> None:
    """Copy known keys with matching types from data into a dataclass instance."""
    for f in fields(section):
        if f.name not in data:
            continue
        cur = getattr(section, f.name)
        val = data[f.name]
        if isinstance(cur, bool):
            ok = isinstance(val, bool)
        elif isinstance(cur, int):
            ok = isinstance(val, int) and not isinstance(val, bool)
        else:
            ok = isinstance(val, type(cur))
        if ok:
            setattr(section, f.name, val)


def loads(text: str) -> Settings:
    try:
        data = tomllib.loads(text)
    except tomllib.TOMLDecodeError:
        return Settings()
    s = Settings()
    for f in fields(s):
        sec = getattr(s, f.name)
        if is_dataclass(sec) and isinstance(data.get(f.name), dict):
            _merge(sec, data[f.name])
    return s.normalize()


def _toml_value(v) -> str:
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    return '"' + str(v).replace("\\", "\\\\").replace('"', '\\"') + '"'


def dumps(s: Settings) -> str:
    out: list[str] = []
    for name, sec in asdict(s).items():
        out.append(f"[{name}]")
        out.extend(f"{k} = {_toml_value(v)}" for k, v in sec.items())
        out.append("")
    return "\n".join(out)


def load(path: Path) -> Settings:
    try:
        return loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return Settings()
    except OSError:
        return Settings()


def save(s: Settings, path: Path) -> None:
    atomic_write_text(path, dumps(s.normalize()))
