use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::domain::{SearchCategory, Track};

pub const INNERTUBE_BASE_URL: &str = "https://music.youtube.com/youtubei/v1";
pub const YTM_ORIGIN: &str = "https://music.youtube.com";
pub const YTM_REFERER: &str = "https://music.youtube.com/";
pub const YTM_API_KEY: &str = "AIzaSyC9XL3ZjWddXya6X74dJoCTL-WEYFDNX30";
pub const WEB_REMIX_CLIENT_NAME: &str = "WEB_REMIX";
pub const WEB_REMIX_CLIENT_ID: &str = "67";
pub const WEB_REMIX_CLIENT_VERSION: &str = "1.20260101.01.00";
pub const ANDROID_MUSIC_CLIENT_NAME: &str = "ANDROID_MUSIC";
pub const ANDROID_MUSIC_CLIENT_ID: &str = "21";
pub const ANDROID_MUSIC_CLIENT_VERSION: &str = "7.29.52";
pub const WEB_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:136.0) Gecko/20100101 Firefox/136.0";
pub const ANDROID_MUSIC_USER_AGENT: &str =
    "com.google.android.apps.youtube.music/7.29.52 (Linux; U; Android 14) gzip";

/// return a fresh `1.YYYYMMDD.01.00` client version for yesterday's utc date
pub fn web_remix_client_version() -> String {
    let Ok(dur) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return WEB_REMIX_CLIENT_VERSION.to_string();
    };
    let days = (dur.as_secs().saturating_sub(86_400) / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("1.{year:04}{m:02}{d:02}.01.00")
}

/// verify a telemetry or auth redirect url points strictly to an https google/youtube host
pub fn is_trusted_google_url(raw: &str) -> bool {
    let Ok(parsed) = url::Url::parse(raw) else {
        return false;
    };
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    matches!(
        host.as_str(),
        "youtube.com" | "google.com" | "googleapis.com"
    ) || host.ends_with(".youtube.com")
        || host.ends_with(".google.com")
        || host.ends_with(".googleapis.com")
}

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
            "clientVersion": web_remix_client_version(),
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
