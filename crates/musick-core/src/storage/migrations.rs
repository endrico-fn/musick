use rusqlite::Connection;

use crate::error::CoreResult;

const USER_SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS playlists (
    id TEXT PRIMARY KEY,
    account_key TEXT NOT NULL DEFAULT 'local',
    remote_id TEXT,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL CHECK (source IN ('local', 'ytm_synced')),
    thumb_url TEXT,
    remote_etag TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_synced_at INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_playlists_account_remote
    ON playlists(account_key, remote_id)
    WHERE remote_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS playlist_tracks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    video_id TEXT NOT NULL,
    set_video_id TEXT,
    title TEXT NOT NULL,
    artists_json TEXT NOT NULL,
    album TEXT,
    album_id TEXT,
    duration_sec INTEGER,
    thumb_url TEXT,
    video_type TEXT,
    is_available INTEGER NOT NULL DEFAULT 1,
    added_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_playlist_tracks_pid_pos
    ON playlist_tracks(playlist_id, position);

CREATE TABLE IF NOT EXISTS liked_songs (
    account_key TEXT NOT NULL DEFAULT 'local',
    video_id TEXT NOT NULL,
    track_json TEXT NOT NULL,
    liked_at INTEGER NOT NULL,
    PRIMARY KEY (account_key, video_id)
);

CREATE TABLE IF NOT EXISTS saved_collections (
    account_key TEXT NOT NULL DEFAULT 'local',
    kind TEXT NOT NULL CHECK (kind IN ('album', 'artist', 'playlist')),
    browse_id TEXT NOT NULL,
    title TEXT NOT NULL,
    subtitle TEXT,
    thumb_url TEXT,
    extra_json TEXT NOT NULL DEFAULT '{}',
    saved_at INTEGER NOT NULL,
    PRIMARY KEY (account_key, kind, browse_id)
);

CREATE TABLE IF NOT EXISTS sync_outbox (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_key TEXT NOT NULL,
    op_kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    retry_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS idx_sync_outbox_account_id
    ON sync_outbox(account_key, id);

CREATE TABLE IF NOT EXISTS play_history (
    video_id TEXT PRIMARY KEY,
    track_json TEXT NOT NULL,
    play_count INTEGER NOT NULL DEFAULT 1,
    last_played_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS search_history (
    query TEXT PRIMARY KEY,
    last_used_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS kv_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

const CACHE_SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS catalog_cache (
    cache_key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    last_accessed INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_catalog_cache_lru
    ON catalog_cache(last_accessed);

CREATE TABLE IF NOT EXISTS lyrics_cache (
    cache_key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    last_accessed INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_lyrics_cache_lru
    ON lyrics_cache(last_accessed);

CREATE TABLE IF NOT EXISTS image_index (
    url_hash TEXT PRIMARY KEY,
    url TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    last_accessed INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_image_index_lru
    ON image_index(last_accessed);

CREATE TABLE IF NOT EXISTS ephemeral_kv (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);
"#;

pub fn migrate_user_db(conn: &Connection) -> CoreResult<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;
    conn.execute_batch(USER_SCHEMA_V1)?;
    let now = crate::auth::now_unix_secs() as i64;
    conn.execute(
        "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (1, ?1)",
        [now],
    )?;
    Ok(())
}

pub fn migrate_cache_db(conn: &Connection) -> CoreResult<()> {
    conn.execute_batch(
        "PRAGMA auto_vacuum = INCREMENTAL;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;",
    )?;
    conn.execute_batch(CACHE_SCHEMA_V1)?;
    Ok(())
}
