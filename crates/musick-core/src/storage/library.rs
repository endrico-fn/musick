use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};

use crate::auth::now_unix_secs;
use crate::domain::{SavedCollection, SavedCollectionKind, Track};
use crate::error::{CoreError, CoreResult};

fn now_i64() -> i64 {
    now_unix_secs() as i64
}

// ── Liked Songs ─────────────────────────────────────────────────────────────

pub fn like_track(conn: &Connection, account_key: &str, track: &Track) -> CoreResult<()> {
    let json = serde_json::to_string(track)?;
    conn.execute(
        "INSERT INTO liked_songs (account_key, video_id, track_json, liked_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(account_key, video_id)
         DO UPDATE SET track_json = excluded.track_json, liked_at = excluded.liked_at",
        params![account_key, track.video_id, json, now_i64()],
    )?;
    Ok(())
}

pub fn unlike_track(conn: &Connection, account_key: &str, video_id: &str) -> CoreResult<bool> {
    let changed = conn.execute(
        "DELETE FROM liked_songs WHERE (account_key = ?1 OR account_key = 'local') AND video_id = ?2",
        params![account_key, video_id],
    )?;
    Ok(changed > 0)
}

pub fn is_track_liked(conn: &Connection, account_key: &str, video_id: &str) -> CoreResult<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM liked_songs WHERE (account_key = ?1 OR account_key = 'local') AND video_id = ?2 LIMIT 1",
            params![account_key, video_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

pub fn list_liked_songs(conn: &Connection, account_key: &str) -> CoreResult<Vec<Track>> {
    let mut stmt = conn.prepare(
        "SELECT video_id, track_json FROM liked_songs
         WHERE account_key = ?1 OR account_key = 'local'
         ORDER BY liked_at DESC",
    )?;
    let rows = stmt.query_map([account_key], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in rows {
        let (vid, raw) = item?;
        if !seen.insert(vid) {
            continue;
        }
        if let Ok(track) = serde_json::from_str::<Track>(&raw) {
            out.push(track);
        }
    }
    Ok(out)
}

pub fn replace_remote_liked_songs(
    conn: &mut Connection,
    account_key: &str,
    tracks: &[Track],
) -> CoreResult<()> {
    if account_key == "local" {
        return Err(CoreError::InvalidInput(
            "Cannot replace remote liked songs for local scope".into(),
        ));
    }
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM liked_songs WHERE account_key = ?1",
        [account_key],
    )?;
    let base_ts = now_i64();
    for (idx, tr) in tracks.iter().enumerate() {
        let json = serde_json::to_string(tr)?;
        // Preserve remote ordering via descending timestamps.
        let ts = base_ts.saturating_sub(idx as i64);
        tx.execute(
            "INSERT OR REPLACE INTO liked_songs (account_key, video_id, track_json, liked_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![account_key, tr.video_id, json, ts],
        )?;
    }
    tx.commit()?;
    Ok(())
}

// ── Saved Collections (Albums, Artists, Community Playlists) ────────────────

pub fn save_collection(conn: &Connection, item: &SavedCollection) -> CoreResult<()> {
    conn.execute(
        "INSERT INTO saved_collections (
            account_key, kind, browse_id, title, subtitle, thumb_url, extra_json, saved_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(account_key, kind, browse_id)
         DO UPDATE SET
            title = excluded.title,
            subtitle = excluded.subtitle,
            thumb_url = excluded.thumb_url,
            extra_json = excluded.extra_json,
            saved_at = excluded.saved_at",
        params![
            item.account_key,
            item.kind.as_db_str(),
            item.browse_id,
            item.title,
            item.subtitle,
            item.thumb_url,
            item.extra_json,
            if item.saved_at > 0 {
                item.saved_at
            } else {
                now_i64()
            }
        ],
    )?;
    Ok(())
}

pub fn unsave_collection(
    conn: &Connection,
    account_key: &str,
    kind: SavedCollectionKind,
    browse_id: &str,
) -> CoreResult<bool> {
    let changed = conn.execute(
        "DELETE FROM saved_collections
         WHERE (account_key = ?1 OR account_key = 'local') AND kind = ?2 AND browse_id = ?3",
        params![account_key, kind.as_db_str(), browse_id],
    )?;
    Ok(changed > 0)
}

pub fn is_collection_saved(
    conn: &Connection,
    account_key: &str,
    kind: SavedCollectionKind,
    browse_id: &str,
) -> CoreResult<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM saved_collections
             WHERE (account_key = ?1 OR account_key = 'local') AND kind = ?2 AND browse_id = ?3
             LIMIT 1",
            params![account_key, kind.as_db_str(), browse_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

pub fn list_saved_collections(
    conn: &Connection,
    account_key: &str,
    kind: SavedCollectionKind,
) -> CoreResult<Vec<SavedCollection>> {
    let mut stmt = conn.prepare(
        "SELECT account_key, kind, browse_id, title, subtitle, thumb_url, extra_json, saved_at
         FROM saved_collections
         WHERE (account_key = ?1 OR account_key = 'local') AND kind = ?2
         ORDER BY saved_at DESC",
    )?;
    let rows = stmt.query_map(params![account_key, kind.as_db_str()], |row| {
        let kind_raw: String = row.get(1)?;
        Ok(SavedCollection {
            account_key: row.get(0)?,
            kind: SavedCollectionKind::from_db_str(&kind_raw).unwrap_or(kind),
            browse_id: row.get(2)?,
            title: row.get(3)?,
            subtitle: row.get(4)?,
            thumb_url: row.get(5)?,
            extra_json: row.get(6)?,
            saved_at: row.get(7)?,
        })
    })?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in rows {
        let entry = item?;
        if !seen.insert(entry.browse_id.clone()) {
            continue;
        }
        out.push(entry);
    }
    Ok(out)
}

pub fn replace_remote_collections(
    conn: &mut Connection,
    account_key: &str,
    kind: SavedCollectionKind,
    items: &[SavedCollection],
) -> CoreResult<()> {
    if account_key == "local" {
        return Err(CoreError::InvalidInput(
            "Cannot replace remote collections for local scope".into(),
        ));
    }
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM saved_collections WHERE account_key = ?1 AND kind = ?2",
        params![account_key, kind.as_db_str()],
    )?;
    let base_ts = now_i64();
    for (idx, item) in items.iter().enumerate() {
        let ts = base_ts.saturating_sub(idx as i64);
        tx.execute(
            "INSERT OR REPLACE INTO saved_collections (
                account_key, kind, browse_id, title, subtitle, thumb_url, extra_json, saved_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                account_key,
                kind.as_db_str(),
                item.browse_id,
                item.title,
                item.subtitle,
                item.thumb_url,
                item.extra_json,
                ts
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

// ── Play History & Search History ───────────────────────────────────────────

pub fn record_play(conn: &Connection, track: &Track) -> CoreResult<()> {
    let json = serde_json::to_string(track)?;
    conn.execute(
        "INSERT INTO play_history (video_id, track_json, play_count, last_played_at)
         VALUES (?1, ?2, 1, ?3)
         ON CONFLICT(video_id)
         DO UPDATE SET
            track_json = excluded.track_json,
            play_count = play_count + 1,
            last_played_at = excluded.last_played_at",
        params![track.video_id, json, now_i64()],
    )?;
    Ok(())
}

pub fn recent_plays(conn: &Connection, limit: usize) -> CoreResult<Vec<Track>> {
    let mut stmt =
        conn.prepare("SELECT track_json FROM play_history ORDER BY last_played_at DESC LIMIT ?1")?;
    let rows = stmt.query_map([limit as i64], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for item in rows {
        let raw = item?;
        if let Ok(tr) = serde_json::from_str::<Track>(&raw) {
            out.push(tr);
        }
    }
    Ok(out)
}

pub fn clear_play_history(conn: &Connection) -> CoreResult<()> {
    conn.execute("DELETE FROM play_history", [])?;
    Ok(())
}

pub fn record_search_query(conn: &Connection, query: &str) -> CoreResult<()> {
    let clean = query.trim();
    if clean.is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO search_history (query, last_used_at)
         VALUES (?1, ?2)
         ON CONFLICT(query) DO UPDATE SET last_used_at = excluded.last_used_at",
        params![clean, now_i64()],
    )?;
    conn.execute(
        "DELETE FROM search_history WHERE query NOT IN (
            SELECT query FROM search_history ORDER BY last_used_at DESC LIMIT 60
        )",
        [],
    )?;
    Ok(())
}

pub fn search_history(conn: &Connection, limit: usize) -> CoreResult<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT query FROM search_history ORDER BY last_used_at DESC LIMIT ?1")?;
    let rows = stmt.query_map([limit as i64], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for item in rows {
        out.push(item?);
    }
    Ok(out)
}

pub fn clear_search_history(conn: &Connection) -> CoreResult<()> {
    conn.execute("DELETE FROM search_history", [])?;
    Ok(())
}

// ── Account Isolation on Logout / Account Switch ────────────────────────────

/// Purge all remote-scoped synced data for `account_key` while strictly preserving `'local'` data.
pub fn purge_account(conn: &mut Connection, account_key: &str) -> CoreResult<()> {
    if account_key.trim().is_empty() || account_key == "local" {
        return Ok(());
    }
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM playlists WHERE account_key = ?1",
        [account_key],
    )?;
    tx.execute(
        "DELETE FROM liked_songs WHERE account_key = ?1",
        [account_key],
    )?;
    tx.execute(
        "DELETE FROM saved_collections WHERE account_key = ?1",
        [account_key],
    )?;
    tx.execute(
        "DELETE FROM sync_outbox WHERE account_key = ?1",
        [account_key],
    )?;
    tx.commit()?;
    Ok(())
}
