use std::collections::HashSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::debug;

use crate::auth::{AuthManager, now_unix_secs};
use crate::domain::{
    PlaylistSource, SavedCollection, SavedCollectionKind, Track, UserPlaylist,
};
use crate::error::{CoreError, CoreResult};
use crate::innertube::{CatalogApi, PlaylistEditAction, SongRating};
use crate::storage::{OutboxOp, UserStore};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncReport {
    pub skipped_anonymous: bool,
    pub pushed: usize,
    pub failed: usize,
    pub pulled_playlists: usize,
    pub pulled_likes: usize,
    pub pulled_albums: usize,
    pub pulled_artists: usize,
    pub downgraded_to_local: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutboxPayload {
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
    store: Arc<UserStore>,
    catalog: Arc<dyn CatalogApi>,
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

    // ── Local-First Mutations with Automatic Outbox Queueing ────────────────

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
            self.enqueue(&updated.account_key, "rename_playlist", &updated.id, &payload)?;
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

    pub fn add_tracks_to_playlist(
        &self,
        playlist_id: &str,
        tracks: &[Track],
    ) -> CoreResult<usize> {
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

    pub fn remove_track_at(
        &self,
        playlist_id: &str,
        position: usize,
    ) -> CoreResult<Option<Track>> {
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

    /// Explicit opt-in upload of a local playlist to YouTube Music (`U` key).
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

        // Fetch back to populate `set_video_id` for future remote edits/removals.
        if let Ok(remote_pl) = self.catalog.playlist(&remote_id, 500).await
            && !remote_pl.tracks.is_empty()
        {
            let _ = self.store.replace_playlist_tracks(&pl.id, &remote_pl.tracks);
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

    // ── Full 2-Way Sync (FIFO Push + Reconciled Pull) ───────────────────────

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

        // Phase 1: FIFO push of offline outbox operations (strict per-entity FIFO ordering)
        let ops = self.store.list_outbox(&acct)?;
        let mut blocked_entities: HashSet<String> = HashSet::new();
        for op in ops {
            if blocked_entities.contains(&op.entity_id) {
                continue;
            }
            match self.process_outbox_op(&acct, &op).await {
                Ok(()) => {
                    self.store.complete_outbox(op.id)?;
                    report.pushed += 1;
                }
                Err(err) => {
                    let msg = err.to_string();
                    debug!("Sync outbox op {} ({}) failed: {msg}", op.id, op.op_kind);
                    let _ = self.store.fail_outbox(op.id, &msg);
                    blocked_entities.insert(op.entity_id.clone());
                    report.failed += 1;
                    report.errors.push(format!("{}: {msg}", op.op_kind));
                }
            }
        }

        // Phase 2: Reconciled remote pull
        self.pull_remote_playlists(&acct, &mut report).await;
        self.pull_remote_likes(&acct, &mut report).await;
        self.pull_remote_albums(&acct, &mut report).await;
        self.pull_remote_artists(&acct, &mut report).await;

        let _ = self
            .store
            .set_kv("last_sync_at", &now_unix_secs().to_string());

        Ok(report)
    }

    async fn process_outbox_op(&self, acct: &str, op: &OutboxOp) -> CoreResult<()> {
        let payload: OutboxPayload = serde_json::from_str(&op.payload_json)?;
        match payload {
            OutboxPayload::CreatePlaylist {
                title,
                description,
                video_ids,
            } => {
                let remote_id = self
                    .catalog
                    .create_playlist(&title, &description, &video_ids)
                    .await?;
                self.store
                    .promote_playlist_to_synced(&op.entity_id, acct, &remote_id)?;
                if !video_ids.is_empty()
                    && let Ok(remote_pl) = self.catalog.playlist(&remote_id, 500).await
                {
                    let _ = self
                        .store
                        .replace_playlist_tracks(&op.entity_id, &remote_pl.tracks);
                }
                Ok(())
            }
            OutboxPayload::RenamePlaylist { title, description } => {
                let Some(remote_id) = self.resolve_remote_id(&op.entity_id)? else {
                    return Ok(());
                };
                let mut actions = vec![PlaylistEditAction::SetName(title)];
                if let Some(d) = description {
                    actions.push(PlaylistEditAction::SetDescription(d));
                }
                let _ = self.catalog.edit_playlist(&remote_id, &actions).await?;
                Ok(())
            }
            OutboxPayload::DeletePlaylist { remote_id } => {
                self.catalog.delete_playlist(&remote_id).await
            }
            OutboxPayload::AddTracks { video_ids } => {
                let Some(remote_id) = self.resolve_remote_id(&op.entity_id)? else {
                    return Ok(());
                };
                let actions: Vec<PlaylistEditAction> = video_ids
                    .into_iter()
                    .map(|video_id| PlaylistEditAction::AddVideo { video_id })
                    .collect();
                let res = self.catalog.edit_playlist(&remote_id, &actions).await?;
                for (vid, svid) in res.added_set_video_ids {
                    let _ = self
                        .store
                        .update_track_set_video_id(&op.entity_id, &vid, &svid);
                }
                Ok(())
            }
            OutboxPayload::RemoveTrack {
                video_id,
                set_video_id,
            } => {
                let Some(remote_id) = self.resolve_remote_id(&op.entity_id)? else {
                    return Ok(());
                };
                let resolved_svid = match set_video_id.filter(|s| !s.is_empty()) {
                    Some(s) => Some(s),
                    None => {
                        // Refresh remote items once to locate missing set_video_id (Review Focus #2).
                        let remote_pl = self.catalog.playlist(&remote_id, 500).await?;
                        remote_pl
                            .tracks
                            .into_iter()
                            .find(|t| t.video_id == video_id)
                            .and_then(|t| t.set_video_id)
                    }
                };
                if let Some(svid) = resolved_svid {
                    let _ = self
                        .catalog
                        .edit_playlist(
                            &remote_id,
                            &[PlaylistEditAction::RemoveVideo {
                                video_id,
                                set_video_id: svid,
                            }],
                        )
                        .await?;
                }
                Ok(())
            }
            OutboxPayload::MoveTrack {
                video_id,
                set_video_id,
                successor_video_id,
                successor_set_video_id,
            } => {
                let Some(remote_id) = self.resolve_remote_id(&op.entity_id)? else {
                    return Ok(());
                };
                let mut svid = set_video_id.filter(|s| !s.is_empty());
                let mut succ_svid = successor_set_video_id.filter(|s| !s.is_empty());
                if svid.is_none() || (successor_video_id.is_some() && succ_svid.is_none()) {
                    let remote_pl = self.catalog.playlist(&remote_id, 500).await?;
                    if svid.is_none() {
                        svid = remote_pl
                            .tracks
                            .iter()
                            .find(|t| t.video_id == video_id)
                            .and_then(|t| t.set_video_id.clone());
                    }
                    if let Some(ref succ_vid) = successor_video_id
                        && succ_svid.is_none()
                    {
                        succ_svid = remote_pl
                            .tracks
                            .iter()
                            .find(|t| t.video_id == *succ_vid)
                            .and_then(|t| t.set_video_id.clone());
                    }
                }
                if let Some(s) = svid {
                    let _ = self
                        .catalog
                        .edit_playlist(
                            &remote_id,
                            &[PlaylistEditAction::MoveVideoBefore {
                                set_video_id: s,
                                moved_before_set_video_id: succ_svid,
                            }],
                        )
                        .await?;
                }
                Ok(())
            }
            OutboxPayload::RateSong { video_id, rating } => {
                self.catalog.rate_song(&video_id, rating).await
            }
            OutboxPayload::SetAlbumLibrary { browse_id, save } => {
                self.catalog.set_album_library_status(&browse_id, save).await
            }
            OutboxPayload::SetArtistSubscription {
                channel_id,
                subscribe,
            } => {
                self.catalog
                    .set_artist_subscription(&channel_id, subscribe)
                    .await
            }
            OutboxPayload::SetPlaylistLibrary { playlist_id, save } => {
                self.catalog
                    .set_playlist_library_status(&playlist_id, save)
                    .await
            }
        }
    }

    fn resolve_remote_id(&self, local_playlist_id: &str) -> CoreResult<Option<String>> {
        let Some(pl) = self.store.get_playlist(local_playlist_id)? else {
            return Ok(None);
        };
        if pl.source == PlaylistSource::YtmSynced && pl.remote_id.is_none() {
            return Err(CoreError::Sync(
                "Playlist pending initial remote creation".into(),
            ));
        }
        Ok(pl.remote_id)
    }

    async fn pull_remote_playlists(&self, acct: &str, report: &mut SyncReport) {
        let remote_summaries = match self.catalog.library_playlists().await {
            Ok(v) => v,
            Err(err) => {
                report.errors.push(format!("pull_playlists: {err}"));
                return;
            }
        };

        let mut seen_remote_ids = HashSet::new();
        for summary in remote_summaries {
            let remote_id = summary
                .browse_id
                .strip_prefix("VL")
                .unwrap_or(&summary.browse_id)
                .to_string();
            seen_remote_ids.insert(remote_id.clone());

            let existing = self
                .store
                .get_playlist_by_remote_id(acct, &remote_id)
                .ok()
                .flatten();

            let local_id = match existing {
                Some(pl) => {
                    let has_pending = self
                        .store
                        .has_pending_outbox_for_entity(acct, &pl.id)
                        .unwrap_or(false);
                    if has_pending {
                        // Do not clobber unpushed offline local edits!
                        report.pulled_playlists += 1;
                        continue;
                    }
                    let _ = self.store.mark_playlist_synced(
                        &pl.id,
                        &summary.title,
                        &pl.description,
                        summary.cover_url().as_deref(),
                    );
                    pl.id
                }
                None => match self.store.create_playlist(
                    acct,
                    &summary.title,
                    "",
                    PlaylistSource::YtmSynced,
                    Some(&remote_id),
                    summary.cover_url().as_deref(),
                ) {
                    Ok(created) => created.id,
                    Err(err) => {
                        report.errors.push(format!("create_mirror: {err}"));
                        continue;
                    }
                },
            };

            if let Ok(full_pl) = self.catalog.playlist(&remote_id, 500).await {
                let _ = self.store.mark_playlist_synced(
                    &local_id,
                    &full_pl.summary.title,
                    full_pl.description.as_deref().unwrap_or(""),
                    full_pl.summary.cover_url().as_deref(),
                );
                let _ = self
                    .store
                    .replace_playlist_tracks(&local_id, &full_pl.tracks);
            }
            report.pulled_playlists += 1;
        }

        // Reconcile remotely deleted YtmSynced playlists (NEVER touch Local playlists!).
        if let Ok(existing_synced) = self.store.list_synced_playlists_for_account(acct) {
            for pl in existing_synced {
                let Some(ref rid) = pl.remote_id else {
                    continue;
                };
                if seen_remote_ids.contains(rid) {
                    continue;
                }
                let has_pending = self
                    .store
                    .has_pending_outbox_for_entity(acct, &pl.id)
                    .unwrap_or(false);
                if has_pending {
                    // Downgrade to Local so user edits survive remote deletion!
                    if self.store.downgrade_playlist_to_local(&pl.id).is_ok() {
                        report.downgraded_to_local += 1;
                    }
                } else {
                    let _ = self.store.delete_playlist(&pl.id);
                }
            }
        }
    }

    async fn pull_remote_likes(&self, acct: &str, report: &mut SyncReport) {
        match self.catalog.library_liked_songs(500).await {
            Ok(tracks) => {
                if self.store.replace_remote_liked_songs(acct, &tracks).is_ok() {
                    report.pulled_likes = tracks.len();
                }
            }
            Err(err) => report.errors.push(format!("pull_likes: {err}")),
        }
    }

    async fn pull_remote_albums(&self, acct: &str, report: &mut SyncReport) {
        match self.catalog.library_albums().await {
            Ok(albums) => {
                let items: Vec<SavedCollection> = albums
                    .into_iter()
                    .map(|a| SavedCollection {
                        account_key: acct.to_string(),
                        kind: SavedCollectionKind::Album,
                        browse_id: a.browse_id.clone(),
                        title: a.title.clone(),
                        subtitle: Some(a.subtitle()),
                        thumb_url: a.cover_url(),
                        extra_json: serde_json::to_string(&a).unwrap_or_else(|_| "{}".into()),
                        saved_at: now_unix_secs() as i64,
                    })
                    .collect();
                let count = items.len();
                if self
                    .store
                    .replace_remote_collections(acct, SavedCollectionKind::Album, &items)
                    .is_ok()
                {
                    report.pulled_albums = count;
                }
            }
            Err(err) => report.errors.push(format!("pull_albums: {err}")),
        }
    }

    async fn pull_remote_artists(&self, acct: &str, report: &mut SyncReport) {
        match self.catalog.library_artists().await {
            Ok(artists) => {
                let items: Vec<SavedCollection> = artists
                    .into_iter()
                    .map(|a| SavedCollection {
                        account_key: acct.to_string(),
                        kind: SavedCollectionKind::Artist,
                        browse_id: a.browse_id.clone(),
                        title: a.name.clone(),
                        subtitle: a.subtitle.clone(),
                        thumb_url: a.cover_url(),
                        extra_json: serde_json::to_string(&a).unwrap_or_else(|_| "{}".into()),
                        saved_at: now_unix_secs() as i64,
                    })
                    .collect();
                let count = items.len();
                if self
                    .store
                    .replace_remote_collections(acct, SavedCollectionKind::Artist, &items)
                    .is_ok()
                {
                    report.pulled_artists = count;
                }
            }
            Err(err) => report.errors.push(format!("pull_artists: {err}")),
        }
    }
}
