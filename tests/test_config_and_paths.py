"""Tests for config settings, TOML persistence, and privacy cleanup."""

from pathlib import Path

from musick.config import Settings, dumps, load, loads, save
from musick.paths import (
    cache_dir,
    clear_local_data,
    config_dir,
    data_dir,
    settings_file,
)


def test_privacy_first_defaults() -> None:
    s = Settings()
    assert s.privacy.save_history is False
    assert s.privacy.cache_images is False
    assert s.privacy.cache_lyrics is False
    assert s.privacy.remote_suggestions is True
    assert s.display.cover_size == "M"
    assert s.display.icons == "unicode"
    assert s.display.image_protocol == "auto"
    assert s.display.region == "ID"
    assert s.display.reduced_motion is False
    assert s.display.cava_enabled is True
    assert s.display.cava_palette == "teal"
    assert s.display.cava_smoothing == "smooth"
    assert s.display.cava_audio_source == "auto"
    assert s.player.prefetch is True
    assert s.player.gapless is True
    assert s.player.autoplay is True
    assert s.player.normalize_audio is False
    assert s.player.audio_quality == "best"
    assert s.player.use_mpv_config is False
    assert s.lyrics.source == "auto"


def test_toml_roundtrip_and_normalization(tmp_path: Path) -> None:
    s = Settings()
    s.privacy.save_history = True
    s.display.cover_size = "L"
    s.display.icons = "nerd"
    s.display.image_protocol = "tgp"
    s.display.cava_palette = "aurora"
    s.display.cava_smoothing = "balanced"
    s.player.volume = 95
    s.player.cache_secs = 60
    s.player.autoplay = False
    s.player.normalize_audio = True
    s.player.audio_quality = "balanced"

    path = tmp_path / "settings.toml"
    save(s, path)
    loaded = load(path)
    assert loaded == s

    # Invalid values normalize cleanly without crashing
    corrupt = loads(
        """
[display]
cover_size = "HUGE"
icons = "emoji"
image_protocol = "bad"
cava_palette = "neon"
cava_smoothing = "ultra"
cava_audio_source = "fifo"
[player]
volume = 999
cache_secs = 1
audio_quality = "lossless"
[lyrics]
source = "invalid"
"""
    )
    assert corrupt.display.cover_size == "M"
    assert corrupt.display.icons == "unicode"
    assert corrupt.display.image_protocol == "auto"
    assert corrupt.display.cava_palette == "teal"
    assert corrupt.display.cava_smoothing == "smooth"
    assert corrupt.display.cava_audio_source == "auto"
    assert corrupt.player.volume == 130
    assert corrupt.player.cache_secs == 5
    assert corrupt.player.audio_quality == "best"
    assert corrupt.lyrics.source == "auto"
    assert dumps(corrupt).startswith("[privacy]")


def test_clear_local_data_preserves_settings() -> None:
    s = Settings()
    save(s, settings_file())
    (cache_dir() / "images").mkdir(parents=True, exist_ok=True)
    (cache_dir() / "images" / "x.png").write_bytes(b"123")
    (data_dir()).mkdir(parents=True, exist_ok=True)
    (data_dir() / "history.json").write_text("{}", encoding="utf-8")

    removed = clear_local_data(keep_settings=True)
    assert len(removed) == 2
    assert not cache_dir().exists()
    assert not data_dir().exists()
    assert settings_file().exists()
    assert config_dir().exists()
