mod pull;
mod push;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::auth::{AuthManager, now_unix_secs};
use crate::domain::{PlaylistSource, SavedCollection, SavedCollectionKind, Track, UserPlaylist};
use crate::error::{CoreError, CoreResult};
use crate::innertube::{CatalogApi, SongRating};
use crate::storage::UserStore;

pub const MAX_OUTBOX_RETRIES: u32 = 5;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncReport {
    pub skipped_anonymous: bool,
    pub pushed: usize,
    pub failed: usize,
    #[serde(default)]
    pub skipped_exhausted: usize,
    pub pulled_playlists: usize,
    #[serde(default)]
    pub pulled_saved_playlists: usize,
    pub pulled_likes: usize,
    pub pulled_albums: usize,
    pub pulled_artists: usize,
    pub downgraded_to_local: usize,
    #[serde(default)]
    pub auth_expired: bool,
    pub errors: Vec<String>,
}

pub(super) fn is_auth_expired_error(err: &CoreError) -> bool {
    match err {
        CoreError::Auth(_) => true,
        _ => {
            let msg = err.to_string();
            msg.contains("HTTP 401")
                || msg.contains("HTTP 403")
                || msg.contains("401 Unauthorized")
                || msg.contains("403 Forbidden")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum OutboxPayload {
    CreatePlaylist {
        title: String,
        description: String,
        video_ids: Vec<String>,
    },
    RenamePlaylist {
        title: String,
        description: Option<String>,
    },
    DeletePlaylist {
        remote_id: String,
    },
    AddTracks {
        video_ids: Vec<String>,
    },
    RemoveTrack {
        video_id: String,
        set_video_id: Option<String>,
    },
    MoveTrack {
        video_id: String,
        set_video_id: Option<String>,
        successor_video_id: Option<String>,
        successor_set_video_id: Option<String>,
    },
    RateSong {
        video_id: String,
        rating: SongRating,
    },
    SetAlbumLibrary {
        browse_id: String,
        save: bool,
    },
    SetArtistSubscription {
        channel_id: String,
        subscribe: bool,
    },
    SetPlaylistLibrary {
        playlist_id: String,
        save: bool,
    },
}

pub struct SyncEngine {
    pub(super) store: Arc<UserStore>,
    pub(super) catalog: Arc<dyn CatalogApi>,
    auth: Arc<AuthManager>,
    sync_lock: Mutex<()>,
}

impl SyncEngine {
    pub fn new(
        store: Arc<UserStore>,
        catalog: Arc<dyn CatalogApi>,
        auth: Arc<AuthManager>,
    ) -> Self {
        Self {
            store,
            catalog,
            auth,
            sync_lock: Mutex::new(()),
        }
    }

    pub fn store(&self) -> &Arc<UserStore> {
        &self.store
    }

    pub fn auth(&self) -> &Arc<AuthManager> {
        &self.auth
    }

    pub fn pending_count(&self) -> usize {
        if !self.auth.is_authenticated() {
            return 0;
        }
        self.store
            .count_outbox(&self.auth.account_key())
            .unwrap_or(0)
    }

    pub fn create_playlist(
        &self,
        title: &str,
        description: &str,
        sync_if_authenticated: bool,
    ) -> CoreResult<UserPlaylist> {
        let auth_ok = sync_if_authenticated && self.auth.is_authenticated();
        let (acct, source) = if auth_ok {
            (self.auth.account_key(), PlaylistSource::YtmSynced)
        } else {
            ("local".to_string(), PlaylistSource::Local)
        };

        let pl = self
            .store
            .create_playlist(&acct, title, description, source, None, None)?;

        if auth_ok {
            let payload = OutboxPayload::CreatePlaylist {
                title: pl.title.clone(),
                description: pl.description.clone(),
                video_ids: Vec::new(),
            };
            self.enqueue(&acct, "create_playlist", &pl.id, &payload)?;
        }
        Ok(pl)
    }

    pub fn rename_playlist(
        &self,
        playlist_id: &str,
        new_title: &str,
        new_description: Option<&str>,
    ) -> CoreResult<UserPlaylist> {
        let updated = self
            .store
            .rename_playlist(playlist_id, new_title, new_description)?;
        if updated.source == PlaylistSource::YtmSynced && self.auth.is_authenticated() {
            let payload = OutboxPayload::RenamePlaylist {
                title: updated.title.clone(),
                description: new_description.map(str::to_string),
            };
            self.enqueue(
                &updated.account_key,
                "rename_playlist",
                &updated.id,
                &payload,
            )?;
        }
        Ok(updated)
    }

    pub fn delete_playlist(&self, playlist_id: &str) -> CoreResult<bool> {
        let existing = self.store.get_playlist(playlist_id)?;
        let Some(pl) = existing else {
            return Ok(false);
        };
        if pl.source == PlaylistSource::YtmSynced
            && self.auth.is_authenticated()
            && let Some(remote_id) = pl.remote_id
        {
            let payload = OutboxPayload::DeletePlaylist { remote_id };
            self.enqueue(&pl.account_key, "delete_playlist", &pl.id, &payload)?;
        }
        self.store.delete_playlist(playlist_id)
    }

    pub fn add_tracks_to_playlist(&self, playlist_id: &str, tracks: &[Track]) -> CoreResult<usize> {
        let added = self.store.add_tracks_to_playlist(playlist_id, tracks)?;
        if added > 0
            && let Some(pl) = self.store.get_playlist(playlist_id)?
            && pl.source == PlaylistSource::YtmSynced
            && self.auth.is_authenticated()
        {
            let payload = OutboxPayload::AddTracks {
                video_ids: tracks.iter().map(|t| t.video_id.clone()).collect(),
            };
            self.enqueue(&pl.account_key, "add_tracks", &pl.id, &payload)?;
        }
        Ok(added)
    }

    pub fn remove_track_at(&self, playlist_id: &str, position: usize) -> CoreResult<Option<Track>> {
        let pl_opt = self.store.get_playlist(playlist_id)?;
        let removed = self.store.remove_track_at(playlist_id, position)?;
        if let (Some(pl), Some(tr)) = (pl_opt, removed.as_ref())
            && pl.source == PlaylistSource::YtmSynced
            && self.auth.is_authenticated()
        {
            let payload = OutboxPayload::RemoveTrack {
                video_id: tr.video_id.clone(),
                set_video_id: tr.set_video_id.clone(),
            };
            self.enqueue(&pl.account_key, "remove_track", &pl.id, &payload)?;
        }
        Ok(removed)
    }

    pub fn move_track(&self, playlist_id: &str, from: usize, to: usize) -> CoreResult<bool> {
        let pl_opt = self.store.get_playlist(playlist_id)?;
        let moved = self.store.move_track(playlist_id, from, to)?;
        if moved
            && let Some(pl) = pl_opt
            && pl.source == PlaylistSource::YtmSynced
            && self.auth.is_authenticated()
        {
            let tracks = self.store.playlist_tracks(playlist_id)?;
            if let Some(moved_tr) = tracks.get(to) {
                let succ = tracks.get(to + 1);
                let payload = OutboxPayload::MoveTrack {
                    video_id: moved_tr.video_id.clone(),
                    set_video_id: moved_tr.set_video_id.clone(),
                    successor_video_id: succ.map(|s| s.video_id.clone()),
                    successor_set_video_id: succ.and_then(|s| s.set_video_id.clone()),
                };
                self.enqueue(&pl.account_key, "move_track", &pl.id, &payload)?;
            }
        }
        Ok(moved)
    }

    /// explicit opt-in upload of a local playlist to youtube music
    pub async fn promote_local_to_synced(&self, playlist_id: &str) -> CoreResult<UserPlaylist> {
        if !self.auth.is_authenticated() {
            return Err(CoreError::Auth(
                "Sign in (OAuth or Browser) to upload a local playlist to YouTube Music".into(),
            ));
        }
        let pl = self
            .store
            .get_playlist(playlist_id)?
            .ok_or_else(|| CoreError::NotFound(format!("Playlist {playlist_id} not found")))?;
        if pl.source == PlaylistSource::YtmSynced && pl.remote_id.is_some() {
            return Ok(pl);
        }

        let tracks = self.store.playlist_tracks(playlist_id)?;
        let video_ids: Vec<String> = tracks.iter().map(|t| t.video_id.clone()).collect();
        let remote_id = self
            .catalog
            .create_playlist(&pl.title, &pl.description, &video_ids)
            .await?;

        let acct = self.auth.account_key();
        self.store
            .promote_playlist_to_synced(&pl.id, &acct, &remote_id)?;

        // fetch back to populate `set_video_id` for future remote edits/removals
        if let Ok(remote_pl) = self.catalog.playlist_uncached(&remote_id, 500).await
            && !remote_pl.tracks.is_empty()
        {
            let _ = self
                .store
                .replace_playlist_tracks(&pl.id, &remote_pl.tracks);
        }

        self.store
            .get_playlist(&pl.id)?
            .ok_or_else(|| CoreError::NotFound("Promoted playlist disappeared".into()))
    }

    pub fn toggle_like(&self, track: &Track) -> CoreResult<bool> {
        let acct = self.auth.account_key();
        let currently_liked = self.store.is_track_liked(&acct, &track.video_id)?;
        let now_liked = !currently_liked;
        if now_liked {
            self.store.like_track(&acct, track)?;
        } else {
            self.store.unlike_track(&acct, &track.video_id)?;
        }
        if self.auth.is_authenticated() {
            let payload = OutboxPayload::RateSong {
                video_id: track.video_id.clone(),
                rating: if now_liked {
                    SongRating::Like
                } else {
                    SongRating::Indifferent
                },
            };
            self.enqueue(&acct, "rate_song", &track.video_id, &payload)?;
        }
        Ok(now_liked)
    }

    pub fn toggle_save_collection(
        &self,
        kind: SavedCollectionKind,
        browse_id: &str,
        title: &str,
        subtitle: Option<String>,
        thumb_url: Option<String>,
    ) -> CoreResult<bool> {
        let acct = self.auth.account_key();
        let currently_saved = self.store.is_collection_saved(&acct, kind, browse_id)?;
        let now_saved = !currently_saved;
        if now_saved {
            self.store.save_collection(&SavedCollection {
                account_key: acct.clone(),
                kind,
                browse_id: browse_id.to_string(),
                title: title.to_string(),
                subtitle,
                thumb_url,
                extra_json: "{}".to_string(),
                saved_at: now_unix_secs() as i64,
            })?;
        } else {
            self.store.unsave_collection(&acct, kind, browse_id)?;
        }

        if self.auth.is_authenticated() {
            let (op_kind, payload) = match kind {
                SavedCollectionKind::Album => (
                    "set_album_library",
                    OutboxPayload::SetAlbumLibrary {
                        browse_id: browse_id.to_string(),
                        save: now_saved,
                    },
                ),
                SavedCollectionKind::Artist => (
                    "set_artist_subscription",
                    OutboxPayload::SetArtistSubscription {
                        channel_id: browse_id.to_string(),
                        subscribe: now_saved,
                    },
                ),
                SavedCollectionKind::Playlist => (
                    "set_playlist_library",
                    OutboxPayload::SetPlaylistLibrary {
                        playlist_id: browse_id.to_string(),
                        save: now_saved,
                    },
                ),
            };
            self.enqueue(&acct, op_kind, browse_id, &payload)?;
        }

        Ok(now_saved)
    }

    fn enqueue(
        &self,
        account_key: &str,
        op_kind: &str,
        entity_id: &str,
        payload: &OutboxPayload,
    ) -> CoreResult<()> {
        let json = serde_json::to_string(payload)?;
        let _ = self
            .store
            .enqueue_outbox(account_key, op_kind, entity_id, &json)?;
        Ok(())
    }

    pub async fn sync_now(&self) -> CoreResult<SyncReport> {
        let _guard = self.sync_lock.lock().await;
        if !self.auth.is_authenticated() {
            return Ok(SyncReport {
                skipped_anonymous: true,
                ..SyncReport::default()
            });
        }

        let acct = self.auth.account_key();
        let mut report = SyncReport::default();

        self.push_outbox(&acct, &mut report).await?;

        if !report.auth_expired {
            self.pull_remote_playlists(&acct, &mut report).await;
            self.pull_remote_likes(&acct, &mut report).await;
            self.pull_remote_albums(&acct, &mut report).await;
            self.pull_remote_artists(&acct, &mut report).await;
        }

        let _ = self
            .store
            .set_kv("last_sync_at", &now_unix_secs().to_string());

        Ok(report)
    }
}
