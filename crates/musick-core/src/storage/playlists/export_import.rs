use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::domain::{ArtistRef, PlaylistSource, Track, UserPlaylist};
use crate::error::{CoreError, CoreResult};
use crate::storage::playlists::{
    add_tracks_to_playlist, create_playlist, get_playlist, playlist_tracks,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExportedPlaylist {
    format: String,
    title: String,
    description: String,
    tracks: Vec<Track>,
}

pub fn export_playlist_json(conn: &Connection, playlist_id: &str) -> CoreResult<String> {
    let pl = get_playlist(conn, playlist_id)?
        .ok_or_else(|| CoreError::NotFound(format!("Playlist {playlist_id} not found")))?;
    let tracks = playlist_tracks(conn, playlist_id)?;
    let exported = ExportedPlaylist {
        format: "musick-playlist-v1".to_string(),
        title: pl.title,
        description: pl.description,
        tracks,
    };
    Ok(serde_json::to_string_pretty(&exported)?)
}

pub fn export_playlist_m3u8(conn: &Connection, playlist_id: &str) -> CoreResult<String> {
    let pl = get_playlist(conn, playlist_id)?
        .ok_or_else(|| CoreError::NotFound(format!("Playlist {playlist_id} not found")))?;
    let tracks = playlist_tracks(conn, playlist_id)?;
    let mut out = String::new();
    out.push_str("#EXTM3U\n");
    out.push_str(&format!("#PLAYLIST:{}\n", pl.title.replace('\n', " ")));
    for tr in tracks {
        let dur = tr.duration.map(i64::from).unwrap_or(-1);
        let artist = tr.artist_names().replace('\n', " ");
        let title = tr.title.replace('\n', " ");
        out.push_str(&format!("#EXTINF:{dur},{artist} - {title}\n"));
        out.push_str(&tr.url());
        out.push('\n');
    }
    Ok(out)
}

pub fn import_playlist_json(
    conn: &mut Connection,
    account_key: &str,
    raw: &str,
) -> CoreResult<UserPlaylist> {
    let parsed: ExportedPlaylist = serde_json::from_str(raw)?;
    let pl = create_playlist(
        conn,
        account_key,
        &parsed.title,
        &parsed.description,
        PlaylistSource::Local,
        None,
        None,
    )?;
    add_tracks_to_playlist(conn, &pl.id, &parsed.tracks)?;
    get_playlist(conn, &pl.id)?
        .ok_or_else(|| CoreError::NotFound("Imported playlist disappeared".into()))
}

pub fn import_playlist_m3u8(
    conn: &mut Connection,
    account_key: &str,
    fallback_title: &str,
    raw: &str,
) -> CoreResult<UserPlaylist> {
    let mut title = fallback_title.trim().to_string();
    let mut pending_meta: Option<(Option<u32>, String, Vec<ArtistRef>)> = None;
    let mut tracks = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(pl_name) = trimmed.strip_prefix("#PLAYLIST:") {
            let cleaned = pl_name.trim();
            if !cleaned.is_empty() {
                title = cleaned.to_string();
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("#EXTINF:") {
            if let Some((dur_s, label)) = rest.split_once(',') {
                let dur = dur_s
                    .trim()
                    .parse::<i64>()
                    .ok()
                    .and_then(|d| if d > 0 { u32::try_from(d).ok() } else { None });
                let (artists, song_title) = if let Some((art, t)) = label.split_once(" - ") {
                    let artists = art
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty() && *s != "Unknown artist")
                        .map(|name| ArtistRef {
                            name: name.to_string(),
                            id: None,
                        })
                        .collect();
                    (artists, t.trim().to_string())
                } else {
                    (Vec::new(), label.trim().to_string())
                };
                pending_meta = Some((dur, song_title, artists));
            }
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }

        if let Some(vid) = extract_video_id_from_line(trimmed) {
            let (duration, song_title, artists) = pending_meta
                .take()
                .unwrap_or_else(|| (None, format!("Track ({vid})"), Vec::new()));
            let mut tr = Track::minimal(vid, song_title);
            tr.artists = artists;
            tr.duration = duration;
            tracks.push(tr);
        } else {
            pending_meta = None;
        }
    }

    if title.is_empty() {
        title = "Imported Playlist".to_string();
    }

    let pl = create_playlist(
        conn,
        account_key,
        &title,
        "",
        PlaylistSource::Local,
        None,
        None,
    )?;
    add_tracks_to_playlist(conn, &pl.id, &tracks)?;
    get_playlist(conn, &pl.id)?
        .ok_or_else(|| CoreError::NotFound("Imported playlist disappeared".into()))
}

fn extract_video_id_from_line(line: &str) -> Option<String> {
    if let Ok(parsed) = url::Url::parse(line) {
        let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
        if host.contains("youtu.be") {
            let seg = parsed.path().trim_matches('/').split('/').next()?;
            if !seg.is_empty() {
                return Some(seg.to_string());
            }
        }
        if host.contains("youtube.com") {
            for (k, v) in parsed.query_pairs() {
                if k == "v" && !v.is_empty() {
                    return Some(v.into_owned());
                }
            }
        }
    }
    None
}
