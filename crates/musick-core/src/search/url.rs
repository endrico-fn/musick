use std::sync::LazyLock;

use regex::Regex;

use crate::domain::SearchCategory;

const PREFIX_MAP: &[(&str, SearchCategory)] = &[
    ("community:", SearchCategory::Community),
    ("playlist:", SearchCategory::Playlists),
    ("artist:", SearchCategory::Artists),
    ("album:", SearchCategory::Albums),
    ("video:", SearchCategory::Videos),
    ("comm:", SearchCategory::Community),
    ("song:", SearchCategory::Songs),
    ("vid:", SearchCategory::Videos),
    ("al:", SearchCategory::Albums),
    ("pl:", SearchCategory::Playlists),
    ("a:", SearchCategory::Artists),
    ("c:", SearchCategory::Community),
    ("v:", SearchCategory::Videos),
    ("s:", SearchCategory::Songs),
];

static SCHEMELESS_YT_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:(?:www|m|music)\.)?(?:youtube\.com|youtu\.be)/").ok());

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YoutubeTarget {
    Track(String),
    Playlist(String),
    Album(String),
    Artist(String),
    Handle(String),
}

pub fn parse_query_prefix(raw: &str, default_kind: SearchCategory) -> (SearchCategory, String) {
    let stripped = raw.trim_start();
    let low = stripped.to_ascii_lowercase();
    for &(prefix, cat) in PREFIX_MAP {
        if low.starts_with(prefix) {
            return (cat, stripped[prefix.len()..].trim().to_string());
        }
    }
    (default_kind, raw.trim().to_string())
}

pub fn extract_youtube_target(raw: &str) -> Option<YoutubeTarget> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains(' ') {
        return None;
    }

    let candidate = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else if SCHEMELESS_YT_RE
        .as_ref()
        .is_some_and(|re| re.is_match(trimmed))
    {
        format!("https://{trimmed}")
    } else {
        return None;
    };

    let parsed = url::Url::parse(&candidate).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if !host.contains("youtube.com") && !host.contains("youtu.be") {
        return None;
    }

    if host.contains("youtu.be") {
        let vid = parsed.path().trim_matches('/').split('/').next()?;
        if !vid.is_empty() {
            return Some(YoutubeTarget::Track(vid.to_string()));
        }
    }

    for (k, v) in parsed.query_pairs() {
        if k == "v" && !v.is_empty() {
            return Some(YoutubeTarget::Track(v.into_owned()));
        }
    }
    for (k, v) in parsed.query_pairs() {
        if k == "list" && !v.is_empty() {
            return Some(YoutubeTarget::Playlist(v.into_owned()));
        }
    }

    let parts: Vec<&str> = parsed.path().split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 && matches!(parts[0], "shorts" | "live") {
        return Some(YoutubeTarget::Track(parts[1].to_string()));
    }
    if parts.len() >= 2 && matches!(parts[0], "channel" | "browse") {
        let bid = parts[1];
        if bid.starts_with("MPRE") {
            return Some(YoutubeTarget::Album(bid.to_string()));
        }
        if bid.starts_with("UC") {
            return Some(YoutubeTarget::Artist(bid.to_string()));
        }
        if bid.starts_with("VL")
            || bid.starts_with("PL")
            || bid.starts_with("OLAK")
            || bid.starts_with("RD")
        {
            return Some(YoutubeTarget::Playlist(bid.to_string()));
        }
    }
    if let Some(first) = parts.first()
        && let Some(handle) = first.strip_prefix('@')
        && !handle.is_empty()
    {
        return Some(YoutubeTarget::Handle(handle.to_string()));
    }
    None
}
