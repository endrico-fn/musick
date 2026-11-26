pub mod cache;
pub mod library;
pub mod migrations;
pub mod playlists;

use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{PlaylistSource, SavedCollection, SavedCollectionKind, Track, UserPlaylist};
use crate::error::{CoreError, CoreResult};
use crate::paths::{AppPaths, ensure_private_dir};

pub use cache::{CacheManager, CacheStats, PruneReport};
pub use playlists::OutboxOp;

pub struct UserStore {
    conn: Mutex<Connection>,
}

impl UserStore {
    pub fn open(paths: &AppPaths) -> CoreResult<Self> {
        ensure_private_dir(&paths.data_dir)?;
        let conn = Connection::open(paths.user_db_file())?;
        migrations::migrate_user_db(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock_conn(&self) -> CoreResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| CoreError::Config("UserStore DB lock poisoned".into()))
    }

    // ── Playlists ───────────────────────────────────────────────────────────

    pub fn create_playlist(
        &self,
        account_key: &str,
        title: &str,
        description: &str,
        source: PlaylistSource,
        remote_id: Option<&str>,
        thumb_url: Option<&str>,
    ) -> CoreResult<UserPlaylist> {
        let conn = self.lock_conn()?;
        playlists::create_playlist(
            &conn,
            account_key,
            title,
            description,
            source,
            remote_id,
            thumb_url,
        )
    }

    pub fn get_playlist(&self, playlist_id: &str) -> CoreResult<Option<UserPlaylist>> {
        let conn = self.lock_conn()?;
        playlists::get_playlist(&conn, playlist_id)
    }

    pub fn get_playlist_by_remote_id(
        &self,
        account_key: &str,
        remote_id: &str,
    ) -> CoreResult<Option<UserPlaylist>> {
        let conn = self.lock_conn()?;
        playlists::get_playlist_by_remote_id(&conn, account_key, remote_id)
    }

    pub fn list_playlists(&self, account_key: &str) -> CoreResult<Vec<UserPlaylist>> {
        let conn = self.lock_conn()?;
        playlists::list_playlists(&conn, account_key)
    }

    pub fn list_synced_playlists_for_account(
        &self,
        account_key: &str,
    ) -> CoreResult<Vec<UserPlaylist>> {
        let conn = self.lock_conn()?;
        playlists::list_synced_playlists_for_account(&conn, account_key)
    }

    pub fn rename_playlist(
        &self,
        playlist_id: &str,
        new_title: &str,
        new_description: Option<&str>,
    ) -> CoreResult<UserPlaylist> {
        let conn = self.lock_conn()?;
        playlists::rename_playlist(&conn, playlist_id, new_title, new_description)
    }

    pub fn delete_playlist(&self, playlist_id: &str) -> CoreResult<bool> {
        let conn = self.lock_conn()?;
        playlists::delete_playlist(&conn, playlist_id)
    }

    pub fn downgrade_playlist_to_local(&self, playlist_id: &str) -> CoreResult<()> {
        let conn = self.lock_conn()?;
        playlists::downgrade_playlist_to_local(&conn, playlist_id)
    }

    pub fn promote_playlist_to_synced(
        &self,
        playlist_id: &str,
        account_key: &str,
        remote_id: &str,
    ) -> CoreResult<()> {
        let conn = self.lock_conn()?;
        playlists::promote_playlist_to_synced(&conn, playlist_id, account_key, remote_id)
    }

    pub fn mark_playlist_synced(
        &self,
        playlist_id: &str,
        title: &str,
        description: &str,
        thumb_url: Option<&str>,
    ) -> CoreResult<()> {
        let conn = self.lock_conn()?;
        playlists::mark_playlist_synced(&conn, playlist_id, title, description, thumb_url)
    }

    pub fn playlist_tracks(&self, playlist_id: &str) -> CoreResult<Vec<Track>> {
        let conn = self.lock_conn()?;
        playlists::playlist_tracks(&conn, playlist_id)
    }

    pub fn add_tracks_to_playlist(
        &self,
        playlist_id: &str,
        tracks: &[Track],
    ) -> CoreResult<usize> {
        let mut conn = self.lock_conn()?;
        playlists::add_tracks_to_playlist(&mut conn, playlist_id, tracks)
    }

    pub fn replace_playlist_tracks(&self, playlist_id: &str, tracks: &[Track]) -> CoreResult<()> {
        let mut conn = self.lock_conn()?;
        playlists::replace_playlist_tracks(&mut conn, playlist_id, tracks)
    }

    pub fn remove_track_at(
        &self,
        playlist_id: &str,
        position: usize,
    ) -> CoreResult<Option<Track>> {
        let mut conn = self.lock_conn()?;
        playlists::remove_track_at(&mut conn, playlist_id, position)
    }

    pub fn move_track(&self, playlist_id: &str, from: usize, to: usize) -> CoreResult<bool> {
        let mut conn = self.lock_conn()?;
        playlists::move_track(&mut conn, playlist_id, from, to)
    }

    pub fn update_track_set_video_id(
        &self,
        playlist_id: &str,
        video_id: &str,
        set_video_id: &str,
    ) -> CoreResult<bool> {
        let conn = self.lock_conn()?;
        playlists::update_track_set_video_id(&conn, playlist_id, video_id, set_video_id)
    }

    pub fn export_playlist_json(&self, playlist_id: &str) -> CoreResult<String> {
        let conn = self.lock_conn()?;
        playlists::export_playlist_json(&conn, playlist_id)
    }

    pub fn export_playlist_m3u8(&self, playlist_id: &str) -> CoreResult<String> {
        let conn = self.lock_conn()?;
        playlists::export_playlist_m3u8(&conn, playlist_id)
    }

    pub fn import_playlist_json(&self, account_key: &str, raw: &str) -> CoreResult<UserPlaylist> {
        let mut conn = self.lock_conn()?;
        playlists::import_playlist_json(&mut conn, account_key, raw)
    }

    pub fn import_playlist_m3u8(
        &self,
        account_key: &str,
        fallback_title: &str,
        raw: &str,
    ) -> CoreResult<UserPlaylist> {
        let mut conn = self.lock_conn()?;
        playlists::import_playlist_m3u8(&mut conn, account_key, fallback_title, raw)
    }

    // ── Sync Outbox ─────────────────────────────────────────────────────────

    pub fn enqueue_outbox(
        &self,
        account_key: &str,
        op_kind: &str,
        entity_id: &str,
        payload_json: &str,
    ) -> CoreResult<i64> {
        let conn = self.lock_conn()?;
        playlists::enqueue_outbox(&conn, account_key, op_kind, entity_id, payload_json)
    }

    pub fn list_outbox(&self, account_key: &str) -> CoreResult<Vec<OutboxOp>> {
        let conn = self.lock_conn()?;
        playlists::list_outbox(&conn, account_key)
    }

    pub fn has_pending_outbox_for_entity(
        &self,
        account_key: &str,
        entity_id: &str,
    ) -> CoreResult<bool> {
        let conn = self.lock_conn()?;
        playlists::has_pending_outbox_for_entity(&conn, account_key, entity_id)
    }

    pub fn complete_outbox(&self, op_id: i64) -> CoreResult<()> {
        let conn = self.lock_conn()?;
        playlists::complete_outbox(&conn, op_id)
    }

    pub fn fail_outbox(&self, op_id: i64, error_msg: &str) -> CoreResult<()> {
        let conn = self.lock_conn()?;
        playlists::fail_outbox(&conn, op_id, error_msg)
    }

    pub fn count_outbox(&self, account_key: &str) -> CoreResult<usize> {
        let conn = self.lock_conn()?;
        playlists::count_outbox(&conn, account_key)
    }

    // ── Library (Liked Songs, Saved Albums/Artists/Playlists, Recent) ───────

    pub fn like_track(&self, account_key: &str, track: &Track) -> CoreResult<()> {
        let conn = self.lock_conn()?;
        library::like_track(&conn, account_key, track)
    }

    pub fn unlike_track(&self, account_key: &str, video_id: &str) -> CoreResult<bool> {
        let conn = self.lock_conn()?;
        library::unlike_track(&conn, account_key, video_id)
    }

    pub fn is_track_liked(&self, account_key: &str, video_id: &str) -> CoreResult<bool> {
        let conn = self.lock_conn()?;
        library::is_track_liked(&conn, account_key, video_id)
    }

    pub fn list_liked_songs(&self, account_key: &str) -> CoreResult<Vec<Track>> {
        let conn = self.lock_conn()?;
        library::list_liked_songs(&conn, account_key)
    }

    pub fn replace_remote_liked_songs(&self, account_key: &str, tracks: &[Track]) -> CoreResult<()> {
        let mut conn = self.lock_conn()?;
        library::replace_remote_liked_songs(&mut conn, account_key, tracks)
    }

    pub fn save_collection(&self, item: &SavedCollection) -> CoreResult<()> {
        let conn = self.lock_conn()?;
        library::save_collection(&conn, item)
    }

    pub fn unsave_collection(
        &self,
        account_key: &str,
        kind: SavedCollectionKind,
        browse_id: &str,
    ) -> CoreResult<bool> {
        let conn = self.lock_conn()?;
        library::unsave_collection(&conn, account_key, kind, browse_id)
    }

    pub fn is_collection_saved(
        &self,
        account_key: &str,
        kind: SavedCollectionKind,
        browse_id: &str,
    ) -> CoreResult<bool> {
        let conn = self.lock_conn()?;
        library::is_collection_saved(&conn, account_key, kind, browse_id)
    }

    pub fn list_saved_collections(
        &self,
        account_key: &str,
        kind: SavedCollectionKind,
    ) -> CoreResult<Vec<SavedCollection>> {
        let conn = self.lock_conn()?;
        library::list_saved_collections(&conn, account_key, kind)
    }

    pub fn replace_remote_collections(
        &self,
        account_key: &str,
        kind: SavedCollectionKind,
        items: &[SavedCollection],
    ) -> CoreResult<()> {
        let mut conn = self.lock_conn()?;
        library::replace_remote_collections(&mut conn, account_key, kind, items)
    }

    pub fn record_play(&self, track: &Track) -> CoreResult<()> {
        let conn = self.lock_conn()?;
        library::record_play(&conn, track)
    }

    pub fn recent_plays(&self, limit: usize) -> CoreResult<Vec<Track>> {
        let conn = self.lock_conn()?;
        library::recent_plays(&conn, limit)
    }

    pub fn clear_play_history(&self) -> CoreResult<()> {
        let conn = self.lock_conn()?;
        library::clear_play_history(&conn)
    }

    pub fn record_search_query(&self, query: &str) -> CoreResult<()> {
        let conn = self.lock_conn()?;
        library::record_search_query(&conn, query)
    }

    pub fn search_history(&self, limit: usize) -> CoreResult<Vec<String>> {
        let conn = self.lock_conn()?;
        library::search_history(&conn, limit)
    }

    pub fn clear_search_history(&self) -> CoreResult<()> {
        let conn = self.lock_conn()?;
        library::clear_search_history(&conn)
    }

    pub fn purge_account(&self, account_key: &str) -> CoreResult<()> {
        let mut conn = self.lock_conn()?;
        library::purge_account(&mut conn, account_key)
    }

    pub fn get_kv(&self, key: &str) -> CoreResult<Option<String>> {
        let conn = self.lock_conn()?;
        let val = conn
            .query_row("SELECT value FROM kv_state WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(val)
    }

    pub fn set_kv(&self, key: &str, value: &str) -> CoreResult<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT OR REPLACE INTO kv_state (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }
}
