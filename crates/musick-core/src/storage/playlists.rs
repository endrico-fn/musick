use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::now_unix_secs;
use crate::domain::{ArtistRef, PlaylistSource, Thumb, Track, UserPlaylist};
use crate::error::{CoreError, CoreResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxOp {
    pub id: i64,
    pub account_key: String,
    pub op_kind: String,
    pub entity_id: String,
    pub payload_json: String,
    pub created_at: i64,
    pub retry_count: u32,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExportedPlaylist {
    format: String,
    title: String,
    description: String,
    tracks: Vec<Track>,
}

fn now_i64() -> i64 {
    now_unix_secs() as i64
}

fn row_to_user_playlist(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserPlaylist> {
    let source_raw: String = row.get(5)?;
    let track_count: i64 = row.get(11)?;
    Ok(UserPlaylist {
        id: row.get(0)?,
        account_key: row.get(1)?,
        remote_id: row.get(2)?,
        title: row.get(3)?,
        description: row.get(4)?,
        source: PlaylistSource::from_db_str(&source_raw),
        thumb_url: row.get(6)?,
        remote_etag: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        last_synced_at: row.get(10)?,
        track_count: usize::try_from(track_count.max(0)).unwrap_or(0),
    })
}

fn row_to_track(row: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    let video_id: String = row.get(0)?;
    let set_video_id: Option<String> = row.get(1)?;
    let title: String = row.get(2)?;
    let artists_json: String = row.get(3)?;
    let album: Option<String> = row.get(4)?;
    let album_id: Option<String> = row.get(5)?;
    let duration_sec: Option<i64> = row.get(6)?;
    let thumb_url: Option<String> = row.get(7)?;
    let video_type: Option<String> = row.get(8)?;
    let is_available: i64 = row.get(9)?;

    let artists: Vec<ArtistRef> = serde_json::from_str(&artists_json).unwrap_or_default();
    let thumbs = thumb_url
        .into_iter()
        .map(|url| Thumb {
            url,
            width: 300,
            height: 300,
        })
        .collect();

    Ok(Track {
        video_id,
        title,
        artists,
        album,
        album_id,
        duration: duration_sec.and_then(|d| u32::try_from(d).ok()),
        thumbs,
        plays: None,
        video_type,
        set_video_id,
        is_available: is_available != 0,
    })
}

pub fn create_playlist(
    conn: &Connection,
    account_key: &str,
    title: &str,
    description: &str,
    source: PlaylistSource,
    remote_id: Option<&str>,
    thumb_url: Option<&str>,
) -> CoreResult<UserPlaylist> {
    let clean_title = title.trim();
    if clean_title.is_empty() {
        return Err(CoreError::InvalidInput(
            "Playlist title cannot be empty".into(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    let now = now_i64();
    let last_synced = if source == PlaylistSource::YtmSynced {
        Some(now)
    } else {
        None
    };

    conn.execute(
        "INSERT INTO playlists (
            id, account_key, remote_id, title, description, source, thumb_url, remote_etag,
            created_at, updated_at, last_synced_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?8, ?9)",
        params![
            id,
            account_key,
            remote_id,
            clean_title,
            description.trim(),
            source.as_db_str(),
            thumb_url,
            now,
            last_synced
        ],
    )?;

    get_playlist(conn, &id)?
        .ok_or_else(|| CoreError::NotFound(format!("Created playlist {id} not found")))
}

pub fn get_playlist(conn: &Connection, playlist_id: &str) -> CoreResult<Option<UserPlaylist>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.account_key, p.remote_id, p.title, p.description, p.source, p.thumb_url,
                p.remote_etag, p.created_at, p.updated_at, p.last_synced_at,
                (SELECT COUNT(*) FROM playlist_tracks pt WHERE pt.playlist_id = p.id) AS track_count
         FROM playlists p
         WHERE p.id = ?1",
    )?;
    let item = stmt
        .query_row([playlist_id], row_to_user_playlist)
        .optional()?;
    Ok(item)
}

pub fn get_playlist_by_remote_id(
    conn: &Connection,
    account_key: &str,
    remote_id: &str,
) -> CoreResult<Option<UserPlaylist>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.account_key, p.remote_id, p.title, p.description, p.source, p.thumb_url,
                p.remote_etag, p.created_at, p.updated_at, p.last_synced_at,
                (SELECT COUNT(*) FROM playlist_tracks pt WHERE pt.playlist_id = p.id) AS track_count
         FROM playlists p
         WHERE p.account_key = ?1 AND p.remote_id = ?2",
    )?;
    let item = stmt
        .query_row(params![account_key, remote_id], row_to_user_playlist)
        .optional()?;
    Ok(item)
}

pub fn list_playlists(conn: &Connection, account_key: &str) -> CoreResult<Vec<UserPlaylist>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.account_key, p.remote_id, p.title, p.description, p.source, p.thumb_url,
                p.remote_etag, p.created_at, p.updated_at, p.last_synced_at,
                (SELECT COUNT(*) FROM playlist_tracks pt WHERE pt.playlist_id = p.id) AS track_count
         FROM playlists p
         WHERE p.account_key = 'local' OR p.account_key = ?1
         ORDER BY
            CASE WHEN p.source = 'ytm_synced' THEN 0 ELSE 1 END,
            p.updated_at DESC,
            p.title COLLATE NOCASE ASC",
    )?;
    let rows = stmt.query_map([account_key], row_to_user_playlist)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn list_synced_playlists_for_account(
    conn: &Connection,
    account_key: &str,
) -> CoreResult<Vec<UserPlaylist>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.account_key, p.remote_id, p.title, p.description, p.source, p.thumb_url,
                p.remote_etag, p.created_at, p.updated_at, p.last_synced_at,
                (SELECT COUNT(*) FROM playlist_tracks pt WHERE pt.playlist_id = p.id) AS track_count
         FROM playlists p
         WHERE p.account_key = ?1 AND p.source = 'ytm_synced'",
    )?;
    let rows = stmt.query_map([account_key], row_to_user_playlist)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn rename_playlist(
    conn: &Connection,
    playlist_id: &str,
    new_title: &str,
    new_description: Option<&str>,
) -> CoreResult<UserPlaylist> {
    let clean = new_title.trim();
    if clean.is_empty() {
        return Err(CoreError::InvalidInput(
            "Playlist title cannot be empty".into(),
        ));
    }
    let now = now_i64();
    let changed = if let Some(desc) = new_description {
        conn.execute(
            "UPDATE playlists SET title = ?1, description = ?2, updated_at = ?3 WHERE id = ?4",
            params![clean, desc.trim(), now, playlist_id],
        )?
    } else {
        conn.execute(
            "UPDATE playlists SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![clean, now, playlist_id],
        )?
    };
    if changed == 0 {
        return Err(CoreError::NotFound(format!(
            "Playlist {playlist_id} not found"
        )));
    }
    get_playlist(conn, playlist_id)?
        .ok_or_else(|| CoreError::NotFound(format!("Playlist {playlist_id} not found")))
}

pub fn delete_playlist(conn: &Connection, playlist_id: &str) -> CoreResult<bool> {
    let changed = conn.execute("DELETE FROM playlists WHERE id = ?1", [playlist_id])?;
    Ok(changed > 0)
}

pub fn downgrade_playlist_to_local(conn: &Connection, playlist_id: &str) -> CoreResult<()> {
    let now = now_i64();
    conn.execute(
        "UPDATE playlists
         SET account_key = 'local', source = 'local', remote_id = NULL, remote_etag = NULL,
             last_synced_at = NULL, updated_at = ?1
         WHERE id = ?2",
        params![now, playlist_id],
    )?;
    Ok(())
}

pub fn promote_playlist_to_synced(
    conn: &Connection,
    playlist_id: &str,
    account_key: &str,
    remote_id: &str,
) -> CoreResult<()> {
    let now = now_i64();
    conn.execute(
        "UPDATE playlists
         SET account_key = ?1, source = 'ytm_synced', remote_id = ?2,
             last_synced_at = ?3, updated_at = ?3
         WHERE id = ?4",
        params![account_key, remote_id, now, playlist_id],
    )?;
    Ok(())
}

pub fn mark_playlist_synced(
    conn: &Connection,
    playlist_id: &str,
    title: &str,
    description: &str,
    thumb_url: Option<&str>,
) -> CoreResult<()> {
    let now = now_i64();
    conn.execute(
        "UPDATE playlists
         SET title = ?1, description = ?2, thumb_url = COALESCE(?3, thumb_url),
             last_synced_at = ?4, updated_at = ?4
         WHERE id = ?5",
        params![title, description, thumb_url, now, playlist_id],
    )?;
    Ok(())
}

pub fn playlist_tracks(conn: &Connection, playlist_id: &str) -> CoreResult<Vec<Track>> {
    let mut stmt = conn.prepare(
        "SELECT video_id, set_video_id, title, artists_json, album, album_id, duration_sec,
                thumb_url, video_type, is_available
         FROM playlist_tracks
         WHERE playlist_id = ?1
         ORDER BY position ASC, id ASC",
    )?;
    let rows = stmt.query_map([playlist_id], row_to_track)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn add_tracks_to_playlist(
    conn: &mut Connection,
    playlist_id: &str,
    tracks: &[Track],
) -> CoreResult<usize> {
    if tracks.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    let max_pos: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position), -1) FROM playlist_tracks WHERE playlist_id = ?1",
        [playlist_id],
        |row| row.get(0),
    )?;
    let now = now_i64();
    let mut first_thumb: Option<String> = None;

    for (pos, tr) in (max_pos + 1..).zip(tracks.iter()) {
        let artists_json = serde_json::to_string(&tr.artists)?;
        let thumb = tr.cover_url();
        if first_thumb.is_none() {
            first_thumb = thumb.clone();
        }
        tx.execute(
            "INSERT INTO playlist_tracks (
                playlist_id, position, video_id, set_video_id, title, artists_json, album,
                album_id, duration_sec, thumb_url, video_type, is_available, added_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                playlist_id,
                pos,
                tr.video_id,
                tr.set_video_id,
                tr.title,
                artists_json,
                tr.album,
                tr.album_id,
                tr.duration.map(i64::from),
                thumb,
                tr.video_type,
                i64::from(tr.is_available),
                now
            ],
        )?;
    }

    tx.execute(
        "UPDATE playlists
         SET updated_at = ?1, thumb_url = COALESCE(thumb_url, ?2)
         WHERE id = ?3",
        params![now, first_thumb, playlist_id],
    )?;
    tx.commit()?;
    Ok(tracks.len())
}

pub fn replace_playlist_tracks(
    conn: &mut Connection,
    playlist_id: &str,
    tracks: &[Track],
) -> CoreResult<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
        [playlist_id],
    )?;
    let now = now_i64();
    let mut first_thumb: Option<String> = None;

    for (idx, tr) in tracks.iter().enumerate() {
        let artists_json = serde_json::to_string(&tr.artists)?;
        let thumb = tr.cover_url();
        if first_thumb.is_none() {
            first_thumb = thumb.clone();
        }
        tx.execute(
            "INSERT INTO playlist_tracks (
                playlist_id, position, video_id, set_video_id, title, artists_json, album,
                album_id, duration_sec, thumb_url, video_type, is_available, added_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                playlist_id,
                idx as i64,
                tr.video_id,
                tr.set_video_id,
                tr.title,
                artists_json,
                tr.album,
                tr.album_id,
                tr.duration.map(i64::from),
                thumb,
                tr.video_type,
                i64::from(tr.is_available),
                now
            ],
        )?;
    }

    tx.execute(
        "UPDATE playlists
         SET updated_at = ?1, thumb_url = COALESCE(?2, thumb_url)
         WHERE id = ?3",
        params![now, first_thumb, playlist_id],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn remove_track_at(
    conn: &mut Connection,
    playlist_id: &str,
    position: usize,
) -> CoreResult<Option<Track>> {
    let tx = conn.transaction()?;
    let target: Option<(i64, Track)> = {
        let mut stmt = tx.prepare(
            "SELECT id, video_id, set_video_id, title, artists_json, album, album_id, duration_sec,
                    thumb_url, video_type, is_available
             FROM playlist_tracks
             WHERE playlist_id = ?1 AND position = ?2
             LIMIT 1",
        )?;
        stmt.query_row(params![playlist_id, position as i64], |row| {
            let row_id: i64 = row.get(0)?;
            let artists_json: String = row.get(4)?;
            let thumb_url: Option<String> = row.get(8)?;
            let duration_sec: Option<i64> = row.get(7)?;
            let is_available: i64 = row.get(10)?;
            Ok((
                row_id,
                Track {
                    video_id: row.get(1)?,
                    set_video_id: row.get(2)?,
                    title: row.get(3)?,
                    artists: serde_json::from_str(&artists_json).unwrap_or_default(),
                    album: row.get(5)?,
                    album_id: row.get(6)?,
                    duration: duration_sec.and_then(|d| u32::try_from(d).ok()),
                    thumbs: thumb_url
                        .into_iter()
                        .map(|url| Thumb {
                            url,
                            width: 300,
                            height: 300,
                        })
                        .collect(),
                    plays: None,
                    video_type: row.get(9)?,
                    is_available: is_available != 0,
                },
            ))
        })
        .optional()?
    };

    let Some((row_id, track)) = target else {
        return Ok(None);
    };

    tx.execute("DELETE FROM playlist_tracks WHERE id = ?1", [row_id])?;
    reindex_playlist_tracks(&tx, playlist_id)?;
    tx.execute(
        "UPDATE playlists SET updated_at = ?1 WHERE id = ?2",
        params![now_i64(), playlist_id],
    )?;
    tx.commit()?;
    Ok(Some(track))
}

pub fn move_track(
    conn: &mut Connection,
    playlist_id: &str,
    from: usize,
    to: usize,
) -> CoreResult<bool> {
    if from == to {
        return Ok(false);
    }
    let tx = conn.transaction()?;
    let ids: Vec<i64> = {
        let mut stmt = tx.prepare(
            "SELECT id FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position ASC, id ASC",
        )?;
        let mapped = stmt.query_map([playlist_id], |r| r.get::<_, i64>(0))?;
        let mut v = Vec::new();
        for item in mapped {
            v.push(item?);
        }
        v
    };

    if from >= ids.len() || to >= ids.len() {
        return Ok(false);
    }

    let mut reordered = ids;
    let moved = reordered.remove(from);
    reordered.insert(to, moved);

    for (pos, row_id) in reordered.iter().enumerate() {
        tx.execute(
            "UPDATE playlist_tracks SET position = ?1 WHERE id = ?2",
            params![pos as i64, row_id],
        )?;
    }
    tx.execute(
        "UPDATE playlists SET updated_at = ?1 WHERE id = ?2",
        params![now_i64(), playlist_id],
    )?;
    tx.commit()?;
    Ok(true)
}

pub fn update_track_set_video_id(
    conn: &Connection,
    playlist_id: &str,
    video_id: &str,
    set_video_id: &str,
) -> CoreResult<bool> {
    let row_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM playlist_tracks
             WHERE playlist_id = ?1 AND video_id = ?2 AND (set_video_id IS NULL OR set_video_id = '')
             ORDER BY position ASC LIMIT 1",
            params![playlist_id, video_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(id) = row_id else {
        return Ok(false);
    };
    conn.execute(
        "UPDATE playlist_tracks SET set_video_id = ?1 WHERE id = ?2",
        params![set_video_id, id],
    )?;
    Ok(true)
}

fn reindex_playlist_tracks(tx: &rusqlite::Transaction<'_>, playlist_id: &str) -> CoreResult<()> {
    let ids: Vec<i64> = {
        let mut stmt = tx.prepare(
            "SELECT id FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position ASC, id ASC",
        )?;
        let rows = stmt.query_map([playlist_id], |r| r.get::<_, i64>(0))?;
        let mut out = Vec::new();
        for id in rows {
            out.push(id?);
        }
        out
    };
    for (idx, row_id) in ids.iter().enumerate() {
        tx.execute(
            "UPDATE playlist_tracks SET position = ?1 WHERE id = ?2",
            params![idx as i64, row_id],
        )?;
    }
    Ok(())
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

// ── Sync Outbox ─────────────────────────────────────────────────────────────

pub fn enqueue_outbox(
    conn: &Connection,
    account_key: &str,
    op_kind: &str,
    entity_id: &str,
    payload_json: &str,
) -> CoreResult<i64> {
    let now = now_i64();
    conn.execute(
        "INSERT INTO sync_outbox (account_key, op_kind, entity_id, payload_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![account_key, op_kind, entity_id, payload_json, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_outbox(conn: &Connection, account_key: &str) -> CoreResult<Vec<OutboxOp>> {
    let mut stmt = conn.prepare(
        "SELECT id, account_key, op_kind, entity_id, payload_json, created_at, retry_count, last_error
         FROM sync_outbox
         WHERE account_key = ?1
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([account_key], |row| {
        let retry: i64 = row.get(6)?;
        Ok(OutboxOp {
            id: row.get(0)?,
            account_key: row.get(1)?,
            op_kind: row.get(2)?,
            entity_id: row.get(3)?,
            payload_json: row.get(4)?,
            created_at: row.get(5)?,
            retry_count: u32::try_from(retry.max(0)).unwrap_or(0),
            last_error: row.get(7)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn has_pending_outbox_for_entity(
    conn: &Connection,
    account_key: &str,
    entity_id: &str,
) -> CoreResult<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sync_outbox WHERE account_key = ?1 AND entity_id = ?2",
        params![account_key, entity_id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub fn complete_outbox(conn: &Connection, op_id: i64) -> CoreResult<()> {
    conn.execute("DELETE FROM sync_outbox WHERE id = ?1", [op_id])?;
    Ok(())
}

pub fn fail_outbox(conn: &Connection, op_id: i64, error_msg: &str) -> CoreResult<()> {
    conn.execute(
        "UPDATE sync_outbox SET retry_count = retry_count + 1, last_error = ?1 WHERE id = ?2",
        params![error_msg, op_id],
    )?;
    Ok(())
}

pub fn count_outbox(conn: &Connection, account_key: &str) -> CoreResult<usize> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sync_outbox WHERE account_key = ?1",
        [account_key],
        |row| row.get(0),
    )?;
    Ok(usize::try_from(count.max(0)).unwrap_or(0))
}
