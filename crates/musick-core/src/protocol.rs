use serde::{Deserialize, Serialize};

use crate::config::Settings;
use crate::domain::{Lyrics, RepeatMode, Track};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PositionTick {
    pub position_secs: f64,
    pub duration_secs: f64,
    pub buffering: bool,
    pub active_lyric_idx: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackSnapshot {
    pub track: Option<Track>,
    pub playing: bool,
    pub paused: bool,
    pub resolving: bool,
    pub restored: bool,
    pub volume: u8,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub autoplay_active: bool,
    pub radio_active: bool,
    pub normalize_audio: bool,
    pub upcoming_count: usize,
    pub show_translation: bool,
    pub loading_lyrics: bool,
    pub translating_lyrics: bool,
    pub lyrics_offset_secs: f64,
    pub audio_info: String,
    pub error: Option<String>,
    pub track_rev: u64,
    pub queue_rev: u64,
    pub lyrics_rev: u64,
}

impl Default for PlaybackSnapshot {
    fn default() -> Self {
        Self {
            track: None,
            playing: false,
            paused: false,
            resolving: false,
            restored: false,
            volume: 80,
            muted: false,
            shuffle: false,
            repeat: RepeatMode::Off,
            autoplay_active: false,
            radio_active: false,
            normalize_audio: false,
            upcoming_count: 0,
            show_translation: false,
            loading_lyrics: false,
            translating_lyrics: false,
            lyrics_offset_secs: 0.0,
            audio_info: String::new(),
            error: None,
            track_rev: 0,
            queue_rev: 0,
            lyrics_rev: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct QueueSnapshot {
    pub tracks: Vec<Track>,
    pub index: Option<usize>,
    pub priority_count: usize,
    pub can_undo: bool,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum AppCommand {
    PlayTracks {
        tracks: Vec<Track>,
        start_index: usize,
        autoplay: bool,
    },
    PlayNext {
        tracks: Vec<Track>,
    },
    AppendTracks {
        tracks: Vec<Track>,
    },
    StartRadio {
        video_id: Option<String>,
        playlist_id: Option<String>,
        seed_track: Option<Track>,
    },
    RestoreTrack {
        track: Track,
    },
    RestoreSession {
        tracks: Vec<Track>,
        index: usize,
        position_secs: f64,
        shuffle: bool,
        repeat: RepeatMode,
    },
    StopPlayback,
    TogglePause,
    ToggleMute,
    Next,
    Previous,
    SeekRelative {
        seconds: f64,
    },
    SeekAbsolute {
        seconds: f64,
    },
    AdjustLyricsOffset {
        delta_secs: f64,
    },
    SetVolume {
        volume: u8,
    },
    AdjustVolume {
        delta: i16,
    },
    ToggleShuffle,
    CycleRepeat,
    JumpToQueueIndex {
        index: usize,
    },
    MoveQueueItem {
        from: usize,
        to: usize,
    },
    RemoveQueueItem {
        index: usize,
    },
    PromoteQueueItemToNext {
        index: usize,
    },
    UndoQueue,
    DeduplicateQueue,
    ClearUpcomingQueue,
    ClearQueue,
    ToggleLyricsTranslation,
    ApplySettings {
        settings: Settings,
    },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum PlayerEvent {
    Snapshot(Box<PlaybackSnapshot>),
    Queue(QueueSnapshot),
    Lyrics {
        track_video_id: Option<String>,
        lyrics: Lyrics,
    },
    Seeked {
        position_secs: f64,
    },
    StatusToast(String),
}
