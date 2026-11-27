use std::collections::HashSet;

use tracing::debug;

use crate::domain::PlaylistSource;
use crate::error::{CoreError, CoreResult};
use crate::innertube::PlaylistEditAction;
use crate::storage::OutboxOp;

use super::{MAX_OUTBOX_RETRIES, OutboxPayload, SyncEngine, SyncReport, is_auth_expired_error};

impl SyncEngine {
    pub(super) async fn push_outbox(&self, acct: &str, report: &mut SyncReport) -> CoreResult<()> {
        let ops = self.store.list_outbox(acct)?;
        let mut blocked_entities: HashSet<String> = HashSet::new();
        for op in ops {
            if blocked_entities.contains(&op.entity_id) {
                continue;
            }
            if op.retry_count >= MAX_OUTBOX_RETRIES {
                blocked_entities.insert(op.entity_id.clone());
                report.skipped_exhausted += 1;
                report.errors.push(format!(
                    "{}: retry cap ({MAX_OUTBOX_RETRIES}) reached",
                    op.op_kind
                ));
                continue;
            }
            match self.process_outbox_op(acct, &op).await {
                Ok(()) => {
                    self.store.complete_outbox(op.id)?;
                    report.pushed += 1;
                }
                Err(err) => {
                    let auth_err = is_auth_expired_error(&err);
                    let msg = err.to_string();
                    debug!("Sync outbox op {} ({}) failed: {msg}", op.id, op.op_kind);
                    let _ = self.store.fail_outbox(op.id, &msg);
                    blocked_entities.insert(op.entity_id.clone());
                    report.failed += 1;
                    report.errors.push(format!("{}: {msg}", op.op_kind));
                    if auth_err {
                        report.auth_expired = true;
                        break;
                    }
                }
            }
        }
        Ok(())
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
                    && let Ok(remote_pl) = self.catalog.playlist_uncached(&remote_id, 500).await
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
                        let remote_pl = self.catalog.playlist_uncached(&remote_id, 500).await?;
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
                    let remote_pl = self.catalog.playlist_uncached(&remote_id, 500).await?;
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
                self.catalog
                    .set_album_library_status(&browse_id, save)
                    .await
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
}
