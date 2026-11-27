use serde_json::Value;

use crate::domain::{ArtistRef, LyricLine, Lyrics, Track};
use crate::innertube::endpoints::{PlaybackTracking, RadioBatch};
use crate::innertube::extract::{
    extract_navigation_browse_id, extract_navigation_page_type, extract_thumbs,
    extract_watch_video_type, find_all_by_key, get_array, get_path, get_str, parse_duration_str,
    runs_text, runs_vec,
};

use super::{extract_continuation_token, is_separator_run, looks_like_year};

pub fn parse_watch_playlist(raw: &Value) -> (RadioBatch, Option<String>, Option<String>) {
    let mut tracks = Vec::new();
    let mut playlist_id = None;
    let mut continuation = None;

    let panel = get_path(
        raw,
        &[
            "contents",
            "singleColumnMusicWatchNextResultsRenderer",
            "tabbedRenderer",
            "watchNextTabbedResultsRenderer",
            "tabs",
            "0",
            "tabRenderer",
            "content",
            "musicQueueRenderer",
            "content",
            "playlistPanelRenderer",
        ],
    )
    .or_else(|| get_path(raw, &["continuationContents", "playlistPanelContinuation"]));

    if let Some(p) = panel {
        playlist_id = p
            .get("playlistId")
            .and_then(Value::as_str)
            .map(str::to_string);
        continuation = extract_continuation_token(p);
        if let Some(contents) = get_array(p, &["contents"]) {
            for c in contents {
                if let Some(r) = c.get("playlistPanelVideoRenderer").or_else(|| {
                    get_path(
                        c,
                        &[
                            "playlistPanelVideoWrapperRenderer",
                            "primaryRenderer",
                            "playlistPanelVideoRenderer",
                        ],
                    )
                }) && let Some(tr) = parse_playlist_panel_video(r)
                {
                    tracks.push(tr);
                }
            }
        }
    }

    let mut lyrics_id = None;
    let mut related_id = None;
    if let Some(tabs) = get_array(
        raw,
        &[
            "contents",
            "singleColumnMusicWatchNextResultsRenderer",
            "tabbedRenderer",
            "watchNextTabbedResultsRenderer",
            "tabs",
        ],
    ) {
        for tab in tabs {
            let Some(tr) = tab.get("tabRenderer") else {
                continue;
            };
            let tab_title = tr
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            let bid = get_str(tr, &["endpoint", "browseEndpoint", "browseId"]);
            if let Some(b) = bid {
                if tab_title.contains("lyric") || b.starts_with("MPLYt") {
                    lyrics_id = Some(b.to_string());
                } else if tab_title.contains("related") || b.starts_with("MPTRt") {
                    related_id = Some(b.to_string());
                }
            }
        }
    }

    (
        RadioBatch {
            tracks,
            continuation,
            playlist_id,
        },
        lyrics_id,
        related_id,
    )
}

fn parse_playlist_panel_video(r: &Value) -> Option<Track> {
    let video_id = r.get("videoId")?.as_str()?.to_string();
    let title = r.get("title").and_then(runs_text)?;
    let thumbs = extract_thumbs(r);
    let duration = r
        .get("lengthText")
        .and_then(runs_text)
        .and_then(|s| parse_duration_str(&s));
    let video_type = extract_watch_video_type(r);
    let set_video_id = r
        .get("playlistSetVideoId")
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut artists = Vec::new();
    let mut album = None;
    let mut album_id = None;
    if let Some(byline) = r.get("longBylineText").or_else(|| r.get("shortBylineText")) {
        for run in runs_vec(byline) {
            let txt = run.get("text").and_then(Value::as_str).unwrap_or("").trim();
            if is_separator_run(txt) || looks_like_year(txt) {
                continue;
            }
            if let Some(bid) = extract_navigation_browse_id(run) {
                let pt = extract_navigation_page_type(run).unwrap_or_default();
                if pt == "MUSIC_PAGE_TYPE_ALBUM" || bid.starts_with("MPRE") {
                    album = Some(txt.to_string());
                    album_id = Some(bid);
                } else {
                    artists.push(ArtistRef {
                        name: txt.to_string(),
                        id: Some(bid),
                    });
                }
            } else if artists.is_empty() {
                artists.push(ArtistRef {
                    name: txt.to_string(),
                    id: None,
                });
            }
        }
    }

    Some(Track {
        video_id,
        title,
        artists,
        album,
        album_id,
        duration,
        thumbs,
        plays: None,
        video_type,
        set_video_id,
        is_available: true,
    })
}

pub fn parse_yt_lyrics(raw: &Value) -> Option<Lyrics> {
    let mut timed_lists = Vec::new();
    find_all_by_key(raw, "timedLyricsData", &mut timed_lists);
    for list_val in timed_lists {
        let Some(arr) = list_val.as_array() else {
            continue;
        };
        let mut lines = Vec::new();
        for entry in arr {
            let Some(text) = entry.get("lyricLine").and_then(Value::as_str) else {
                continue;
            };
            let start_ms = get_path(entry, &["cueRange", "startTimeMilliseconds"])
                .and_then(|v| {
                    v.as_u64()
                        .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
                })
                .unwrap_or(0);
            lines.push(LyricLine::new(start_ms as f64 / 1000.0, text.trim()));
        }
        if !lines.is_empty() {
            lines.sort_by(|a, b| a.time.total_cmp(&b.time));
            return Some(Lyrics {
                lines,
                synced: true,
                source: "YouTube Music".to_string(),
                translation_lang: String::new(),
            });
        }
    }

    let mut shelves = Vec::new();
    find_all_by_key(raw, "musicDescriptionShelfRenderer", &mut shelves);
    for shelf in shelves {
        if let Some(text) = shelf.get("description").and_then(runs_text) {
            let lines: Vec<LyricLine> = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(|l| LyricLine::new(0.0, l))
                .collect();
            if !lines.is_empty() {
                return Some(Lyrics {
                    lines,
                    synced: false,
                    source: "YouTube Music".to_string(),
                    translation_lang: String::new(),
                });
            }
        }
    }

    None
}

pub fn parse_player_tracking(video_id: &str, raw: &Value) -> Option<PlaybackTracking> {
    let tracking = raw.get("playbackTracking")?;
    let playback_url = get_str(tracking, &["videostatsPlaybackUrl", "baseUrl"]).map(str::to_string);
    let watchtime_url =
        get_str(tracking, &["videostatsWatchtimeUrl", "baseUrl"]).map(str::to_string);
    if playback_url.is_none() && watchtime_url.is_none() {
        return None;
    }
    let length_secs = get_str(raw, &["videoDetails", "lengthSeconds"])
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    Some(PlaybackTracking {
        video_id: video_id.to_string(),
        playback_url,
        watchtime_url,
        length_secs,
    })
}
