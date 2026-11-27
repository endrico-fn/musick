use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::domain::{SearchCategory, Track};

pub const INNERTUBE_BASE_URL: &str = "https://music.youtube.com/youtubei/v1";
pub const WEB_REMIX_CLIENT_NAME: &str = "WEB_REMIX";
pub const WEB_REMIX_CLIENT_VERSION: &str = "1.20260101.01.00";
pub const ANDROID_MUSIC_CLIENT_NAME: &str = "ANDROID_MUSIC";
pub const ANDROID_MUSIC_CLIENT_VERSION: &str = "7.29.52";
pub const WEB_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:136.0) Gecko/20100101 Firefox/136.0";
pub const ANDROID_MUSIC_USER_AGENT: &str =
    "com.google.android.apps.youtube.music/7.29.52 (Linux; U; Android 14) gzip";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    WebRemix,
    AndroidMusic,
}

pub fn search_filter_params(category: SearchCategory) -> Option<&'static str> {
    match category {
        SearchCategory::All => None,
        SearchCategory::Songs => Some("EgWKAQIIAWoMEAMQBBAJEAoQBRAV"),
        SearchCategory::Videos => Some("EgWKAQIQAWoMEAMQBBAJEAoQBRAV"),
        SearchCategory::Albums => Some("EgWKAQIYAWoMEAMQBBAJEAoQBRAV"),
        SearchCategory::Artists => Some("EgWKAQIgAWoMEAMQBBAJEAoQBRAV"),
        SearchCategory::Playlists => Some("EgeKAQQoADgBagwQAxAEEAkQChAFEBU="),
        SearchCategory::Community => Some("EgeKAQQoAEABagwQAxAEEAkQChAFEBU="),
    }
}

pub fn build_context(
    client_kind: ClientKind,
    region: &str,
    visitor_data: Option<&str>,
    on_behalf_of_user: Option<&str>,
) -> Value {
    let gl = if region.is_empty() || region == "ZZ" {
        "US"
    } else {
        region
    };

    let mut client = match client_kind {
        ClientKind::WebRemix => json!({
            "clientName": WEB_REMIX_CLIENT_NAME,
            "clientVersion": WEB_REMIX_CLIENT_VERSION,
            "hl": "en",
            "gl": gl,
        }),
        ClientKind::AndroidMusic => json!({
            "clientName": ANDROID_MUSIC_CLIENT_NAME,
            "clientVersion": ANDROID_MUSIC_CLIENT_VERSION,
            "androidSdkVersion": 34,
            "osName": "Android",
            "osVersion": "14",
            "hl": "en",
            "gl": gl,
        }),
    };

    if let Some(vd) = visitor_data.filter(|s| !s.trim().is_empty())
        && let Some(obj) = client.as_object_mut()
    {
        obj.insert("visitorData".to_string(), json!(vd.trim()));
    }

    let mut user = json!({});
    if let Some(page_id) = on_behalf_of_user.filter(|s| !s.trim().is_empty())
        && let Some(obj) = user.as_object_mut()
    {
        obj.insert("onBehalfOfUser".to_string(), json!(page_id.trim()));
    }

    json!({
        "client": client,
        "user": user,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RadioBatch {
    pub tracks: Vec<Track>,
    pub continuation: Option<String>,
    pub playlist_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackTracking {
    pub video_id: String,
    pub playback_url: Option<String>,
    pub watchtime_url: Option<String>,
    pub length_secs: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlaybackTelemetryEvent {
    Start,
    Watchtime {
        elapsed_secs: f64,
        duration_secs: f64,
        playing: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SongRating {
    Like,
    Dislike,
    Indifferent,
}

impl SongRating {
    pub fn endpoint(&self) -> &'static str {
        match self {
            Self::Like => "like/like",
            Self::Dislike => "like/dislike",
            Self::Indifferent => "like/removelike",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaylistEditAction {
    AddVideo {
        video_id: String,
    },
    RemoveVideo {
        video_id: String,
        set_video_id: String,
    },
    MoveVideoBefore {
        set_video_id: String,
        moved_before_set_video_id: Option<String>,
    },
    SetName(String),
    SetDescription(String),
}

impl PlaylistEditAction {
    pub fn to_innertube_json(&self) -> Value {
        match self {
            Self::AddVideo { video_id } => json!({
                "action": "ACTION_ADD_VIDEO",
                "addedVideoId": video_id,
                "dedupeOption": "DEDUPE_OPTION_SKIP",
            }),
            Self::RemoveVideo {
                video_id,
                set_video_id,
            } => json!({
                "action": "ACTION_REMOVE_VIDEO",
                "removedVideoId": video_id,
                "setVideoId": set_video_id,
            }),
            Self::MoveVideoBefore {
                set_video_id,
                moved_before_set_video_id,
            } => {
                if let Some(before) = moved_before_set_video_id {
                    json!({
                        "action": "ACTION_MOVE_VIDEO_BEFORE",
                        "setVideoId": set_video_id,
                        "movedSetVideoIdSuccessor": before,
                    })
                } else {
                    json!({
                        "action": "ACTION_MOVE_VIDEO_BEFORE",
                        "setVideoId": set_video_id,
                    })
                }
            }
            Self::SetName(name) => json!({
                "action": "ACTION_SET_PLAYLIST_NAME",
                "playlistName": name,
            }),
            Self::SetDescription(desc) => json!({
                "action": "ACTION_SET_PLAYLIST_DESCRIPTION",
                "playlistDescription": desc,
            }),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistEditResult {
    /// `(video_id, set_video_id)` pairs returned by innertube on `ACTION_ADD_VIDEO`
    pub added_set_video_ids: Vec<(String, String)>,
}
