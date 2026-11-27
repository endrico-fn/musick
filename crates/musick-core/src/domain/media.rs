use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Thumb {
    pub url: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

static GOOG_SIZE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"=w\d+-h\d+(-[a-zA-Z0-9-]+)?$").ok());

/// pick the smallest thumbnail >= `px`, else the largest; upscale google cdn urls
pub fn best_thumb_url(thumbs: &[Thumb], px: u32) -> Option<String> {
    if thumbs.is_empty() {
        return None;
    }

    let sized: Vec<&Thumb> = thumbs.iter().filter(|t| t.width > 0).collect();
    let pick = if !sized.is_empty() {
        sized
            .iter()
            .copied()
            .filter(|t| t.width >= px)
            .min_by_key(|t| t.width)
            .or_else(|| sized.iter().copied().max_by_key(|t| t.width))?
    } else {
        thumbs.last()?
    };

    let mut url = pick.url.clone();
    if (url.contains("googleusercontent.com") || url.contains("ggpht.com"))
        && let Some(re) = GOOG_SIZE_RE.as_ref()
        && let Some(caps) = re.captures(&url)
    {
        let flags = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let has_crop = flags.split('-').any(|f| f == "p");
        let crop = if has_crop { "-p" } else { "" };
        let replacement = format!("=w{px}-h{px}{crop}-l90-rj");
        url = re.replace(&url, replacement.as_str()).into_owned();
    }
    Some(url)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtistRef {
    pub name: String,
    #[serde(default)]
    pub id: Option<String>,
}

pub fn format_duration(seconds: Option<u32>) -> String {
    let Some(secs) = seconds else {
        return "--:--".to_string();
    };
    if secs == 0 {
        return "--:--".to_string();
    }
    format_duration_allow_zero(secs)
}

pub fn format_duration_allow_zero(seconds: u32) -> String {
    let m = seconds / 60;
    let s = seconds % 60;
    let h = m / 60;
    let rem_m = m % 60;
    if h > 0 {
        format!("{h}:{rem_m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Track {
    pub video_id: String,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<ArtistRef>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub album_id: Option<String>,
    #[serde(default)]
    pub duration: Option<u32>,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
    #[serde(default)]
    pub plays: Option<String>,
    #[serde(default)]
    pub video_type: Option<String>,
    #[serde(default)]
    pub set_video_id: Option<String>,
    #[serde(default = "default_true")]
    pub is_available: bool,
}

impl Track {
    pub fn minimal(video_id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            video_id: video_id.into(),
            title: title.into(),
            artists: Vec::new(),
            album: None,
            album_id: None,
            duration: None,
            thumbs: Vec::new(),
            plays: None,
            video_type: None,
            set_video_id: None,
            is_available: true,
        }
    }

    pub fn artist_names(&self) -> String {
        if self.artists.is_empty() {
            "Unknown artist".to_string()
        } else {
            self.artists
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    pub fn duration_text(&self) -> String {
        format_duration(self.duration)
    }

    pub fn url(&self) -> String {
        format!("https://music.youtube.com/watch?v={}", self.video_id)
    }

    pub fn cover_url(&self) -> Option<String> {
        best_thumb_url(&self.thumbs, 300)
    }

    pub fn is_video(&self) -> bool {
        matches!(
            self.video_type.as_deref(),
            Some("MUSIC_VIDEO_TYPE_OMV" | "MUSIC_VIDEO_TYPE_UGC" | "MUSIC_VIDEO_TYPE_VIDEO")
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AlbumSummary {
    pub browse_id: String,
    pub title: String,
    #[serde(default)]
    pub year: Option<String>,
    #[serde(default = "default_album_kind")]
    pub kind: String,
    #[serde(default)]
    pub artists: Vec<ArtistRef>,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
}

fn default_album_kind() -> String {
    "Album".to_string()
}

impl AlbumSummary {
    pub fn subtitle(&self) -> String {
        let mut parts = Vec::new();
        if let Some(y) = self.year.as_deref().filter(|s| !s.is_empty()) {
            parts.push(y);
        }
        if !self.kind.is_empty() {
            parts.push(self.kind.as_str());
        }
        parts.join(" · ")
    }

    pub fn cover_url(&self) -> Option<String> {
        best_thumb_url(&self.thumbs, 300)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Album {
    pub summary: AlbumSummary,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub duration_text: Option<String>,
    #[serde(default)]
    pub audio_playlist_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtistSummary {
    pub browse_id: String,
    pub name: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
}

impl ArtistSummary {
    pub fn cover_url(&self) -> Option<String> {
        best_thumb_url(&self.thumbs, 300)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlaylistSummary {
    pub browse_id: String,
    pub title: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub count: Option<String>,
    #[serde(default)]
    pub thumbs: Vec<Thumb>,
}

impl PlaylistSummary {
    pub fn cover_url(&self) -> Option<String> {
        best_thumb_url(&self.thumbs, 300)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    pub summary: PlaylistSummary,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub views: Option<String>,
    #[serde(default)]
    pub duration_text: Option<String>,
    #[serde(default)]
    pub year: Option<String>,
    #[serde(default)]
    pub total: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artist {
    pub summary: ArtistSummary,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub listeners: Option<String>,
    #[serde(default)]
    pub songs: Vec<Track>,
    #[serde(default)]
    pub albums: Vec<AlbumSummary>,
    #[serde(default)]
    pub singles: Vec<AlbumSummary>,
    #[serde(default)]
    pub playlists: Vec<PlaylistSummary>,
    #[serde(default)]
    pub related: Vec<ArtistSummary>,
    #[serde(default)]
    pub albums_params: Option<String>,
    #[serde(default)]
    pub singles_params: Option<String>,
    #[serde(default)]
    pub radio_id: Option<String>,
    #[serde(default)]
    pub songs_playlist_id: Option<String>,
    #[serde(default)]
    pub videos: Vec<Track>,
    #[serde(default)]
    pub subscribers: Option<String>,
    #[serde(default)]
    pub views: Option<String>,
    #[serde(default)]
    pub albums_browse_id: Option<String>,
    #[serde(default)]
    pub singles_browse_id: Option<String>,
    #[serde(default)]
    pub videos_playlist_id: Option<String>,
    #[serde(default)]
    pub shuffle_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "item_kind", rename_all = "snake_case")]
pub enum SearchItem {
    Track(Track),
    Album(AlbumSummary),
    Artist(ArtistSummary),
    Playlist(PlaylistSummary),
}

impl SearchItem {
    pub fn dedup_key(&self) -> (&'static str, &str) {
        search_item_key(self)
    }

    pub fn title(&self) -> &str {
        match self {
            Self::Track(t) => &t.title,
            Self::Album(a) => &a.title,
            Self::Artist(a) => &a.name,
            Self::Playlist(p) => &p.title,
        }
    }

    pub fn cover_url(&self) -> Option<String> {
        match self {
            Self::Track(t) => t.cover_url(),
            Self::Album(a) => a.cover_url(),
            Self::Artist(a) => a.cover_url(),
            Self::Playlist(p) => p.cover_url(),
        }
    }
}

pub fn is_valid_video_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn search_item_key(item: &SearchItem) -> (&'static str, &str) {
    match item {
        SearchItem::Track(t) => ("t", t.video_id.as_str()),
        SearchItem::Album(a) => ("al", a.browse_id.as_str()),
        SearchItem::Artist(a) => ("ar", a.browse_id.as_str()),
        SearchItem::Playlist(p) => ("pl", p.browse_id.as_str()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchCategory {
    #[default]
    All,
    Songs,
    Videos,
    Albums,
    Artists,
    Playlists,
    Community,
}

impl SearchCategory {
    pub const ALL: &'static [Self] = &[
        Self::All,
        Self::Songs,
        Self::Videos,
        Self::Albums,
        Self::Artists,
        Self::Playlists,
        Self::Community,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Songs => "songs",
            Self::Videos => "videos",
            Self::Albums => "albums",
            Self::Artists => "artists",
            Self::Playlists => "playlists",
            Self::Community => "community",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Songs => "Songs",
            Self::Videos => "Videos",
            Self::Albums => "Albums",
            Self::Artists => "Artists",
            Self::Playlists => "Playlists",
            Self::Community => "Community",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Playlists => "Lists",
            Self::Community => "Comm.",
            other => other.label(),
        }
    }
}

static TITLE_NOISE_FEAT_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\s*[\(\[](?:feat\.?|ft\.?|with\s+[^\)\]]+|official\s*(?:music\s*)?video|official\s*audio|lyric\s*video|lyrics|audio|hd|4k|mv|visualizer|explicit|remaster(?:ed)?(?:\s*\d{4})?)[^\)\]]*[\)\]]",
    )
    .ok()
});

static TITLE_NOISE_CORE_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\s*[\(\[](?:official\s*(?:music\s*)?video|official\s*audio|lyric\s*video|lyrics|audio|hd|4k|mv|visualizer|explicit|remaster(?:ed)?(?:\s*\d{4})?)[^\)\]]*[\)\]]",
    )
    .ok()
});

pub fn clean_media_title(title: &str, strip_feat: bool) -> String {
    let pat = if strip_feat {
        TITLE_NOISE_FEAT_RE.as_ref()
    } else {
        TITLE_NOISE_CORE_RE.as_ref()
    };
    let cleaned = match pat {
        Some(re) => re.replace_all(title, "").trim().to_string(),
        None => title.trim().to_string(),
    };
    if cleaned.is_empty() {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            title.to_string()
        } else {
            trimmed.to_string()
        }
    } else {
        cleaned
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DiscoverFeed {
    /// personalized or regional mixes songs list
    #[serde(default)]
    pub mixes: Vec<Track>,
    /// new releases items
    #[serde(default)]
    pub new_releases: Vec<SearchItem>,
    #[serde(default)]
    pub featured: Vec<SearchItem>,
    #[serde(default)]
    pub artists: Vec<ArtistSummary>,
    #[serde(default)]
    pub playlists: Vec<PlaylistSummary>,
}
