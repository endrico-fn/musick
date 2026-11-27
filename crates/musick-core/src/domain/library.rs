use serde::{Deserialize, Serialize};

use crate::domain::Track;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub tracks: Vec<Track>,
    pub index: usize,
    pub position_secs: f64,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistSource {
    #[default]
    Local,
    YtmSynced,
}

impl PlaylistSource {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::YtmSynced => "ytm_synced",
        }
    }

    pub fn from_db_str(raw: &str) -> Self {
        match raw {
            "ytm_synced" => Self::YtmSynced,
            _ => Self::Local,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserPlaylist {
    pub id: String,
    pub account_key: String,
    pub remote_id: Option<String>,
    pub title: String,
    pub description: String,
    pub source: PlaylistSource,
    pub thumb_url: Option<String>,
    pub remote_etag: Option<String>,
    pub track_count: usize,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_synced_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SavedCollectionKind {
    Album,
    Artist,
    Playlist,
}

impl SavedCollectionKind {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Album => "album",
            Self::Artist => "artist",
            Self::Playlist => "playlist",
        }
    }

    pub fn from_db_str(raw: &str) -> Option<Self> {
        match raw {
            "album" => Some(Self::Album),
            "artist" => Some(Self::Artist),
            "playlist" => Some(Self::Playlist),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedCollection {
    pub account_key: String,
    pub kind: SavedCollectionKind,
    pub browse_id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub thumb_url: Option<String>,
    pub extra_json: String,
    pub saved_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

impl RepeatMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::All => "all",
            Self::One => "one",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueRole {
    Played,
    Playing,
    Next,
    Queued,
}
