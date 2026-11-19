"""Decoupled UI messages for navigation, playback, and settings."""

from __future__ import annotations

from collections.abc import Sequence
from typing import Any

from textual.message import Message

from musick.domain import Track


class OpenArtist(Message):
    def __init__(self, browse_id: str, title: str = "") -> None:
        super().__init__()
        self.browse_id = browse_id
        self.title = title


class OpenAlbum(Message):
    def __init__(self, browse_id: str, title: str = "") -> None:
        super().__init__()
        self.browse_id = browse_id
        self.title = title


class OpenPlaylist(Message):
    def __init__(self, browse_id: str, title: str = "") -> None:
        super().__init__()
        self.browse_id = browse_id
        self.title = title


class OpenTrackAlbum(Message):
    def __init__(self, track: Track) -> None:
        super().__init__()
        self.track = track


class OpenTrackArtist(Message):
    def __init__(self, track: Track) -> None:
        super().__init__()
        self.track = track


class PlayTracks(Message):
    def __init__(self, tracks: Sequence[Track], start: int = 0, shuffle: bool = False) -> None:
        super().__init__()
        self.tracks = list(tracks)
        self.start = start
        self.shuffle = shuffle


class QueueTracks(Message):
    def __init__(self, tracks: Sequence[Track]) -> None:
        super().__init__()
        self.tracks = list(tracks)


class PlayNextTracks(Message):
    def __init__(self, tracks: Sequence[Track]) -> None:
        super().__init__()
        self.tracks = list(tracks)


class PlayEntityDirect(Message):
    def __init__(self, item: Any, shuffle: bool = False) -> None:
        super().__init__()
        self.item = item
        self.shuffle = shuffle


class QueueEntityDirect(Message):
    def __init__(self, item: Any) -> None:
        super().__init__()
        self.item = item


class PlayNextEntityDirect(Message):
    def __init__(self, item: Any) -> None:
        super().__init__()
        self.item = item


class RefreshFooter(Message):
    pass


class StartRadio(Message):
    def __init__(self, video_id: str | None = None, playlist_id: str | None = None) -> None:
        super().__init__()
        self.video_id = video_id
        self.playlist_id = playlist_id


class SaveBookmark(Message):
    def __init__(self, item: Any) -> None:
        super().__init__()
        self.item = item


class NotifyStatus(Message):
    def __init__(self, text: str) -> None:
        super().__init__()
        self.text = text


class SettingsUpdated(Message):
    pass
