use std::collections::HashSet;

use crate::auth::now_unix_secs;
use crate::domain::{PlaylistSource, PlaylistSummary, SavedCollection, SavedCollectionKind};

use super::{SyncEngine, SyncReport, is_auth_expired_error};

fn is_saved_community_playlist(summary: &PlaylistSummary) -> bool {
    let rid = summary
        .browse_id
        .strip_prefix("VL")
        .unwrap_or(&summary.browse_id);
    if rid.starts_with("RDCLAK") || rid.starts_with("OLAK") || rid.starts_with("RD") {
        return true;
    }
    summary.author.as_deref().is_some_and(|a| {
        let low = a.trim().to_ascii_lowercase();
        low == "youtube music" || low == "youtube"
    })
}

impl SyncEngine {
    pub(super) async fn pull_remote_playlists(&self, acct: &str, report: &mut SyncReport) {
        let remote_summaries = match self.catalog.library_playlists().await {
            Ok(v) => v,
            Err(err) => {
                if is_auth_expired_error(&err) {
                    report.auth_expired = true;
                }
                report.errors.push(format!("pull_playlists: {err}"));
                return;
            }
        };

        let mut seen_remote_ids = HashSet::new();
        let mut saved_community = Vec::new();
        for summary in remote_summaries {
            let remote_id = summary
                .browse_id
                .strip_prefix("VL")
                .unwrap_or(&summary.browse_id)
                .to_string();

            if is_saved_community_playlist(&summary) {
                saved_community.push(SavedCollection {
                    account_key: acct.to_string(),
                    kind: SavedCollectionKind::Playlist,
                    browse_id: remote_id,
                    title: summary.title.clone(),
                    subtitle: summary.author.clone().or_else(|| summary.count.clone()),
                    thumb_url: summary.cover_url(),
                    extra_json: serde_json::to_string(&summary).unwrap_or_else(|_| "{}".into()),
                    saved_at: now_unix_secs() as i64,
                });
                continue;
            }

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

            if let Ok(full_pl) = self.catalog.playlist_uncached(&remote_id, 500).await {
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

        if !saved_community.is_empty()
            && self
                .store
                .replace_remote_collections(acct, SavedCollectionKind::Playlist, &saved_community)
                .is_ok()
        {
            report.pulled_saved_playlists = saved_community.len();
        }

        // reconcile remotely deleted synced playlists without touching local ones
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
                    if self.store.downgrade_playlist_to_local(&pl.id).is_ok() {
                        report.downgraded_to_local += 1;
                    }
                } else {
                    let _ = self.store.delete_playlist(&pl.id);
                }
            }
        }
    }

    pub(super) async fn pull_remote_likes(&self, acct: &str, report: &mut SyncReport) {
        match self.catalog.library_liked_songs(500).await {
            Ok(tracks) => {
                if self.store.replace_remote_liked_songs(acct, &tracks).is_ok() {
                    report.pulled_likes = tracks.len();
                }
            }
            Err(err) => {
                if is_auth_expired_error(&err) {
                    report.auth_expired = true;
                }
                report.errors.push(format!("pull_likes: {err}"));
            }
        }
    }

    pub(super) async fn pull_remote_albums(&self, acct: &str, report: &mut SyncReport) {
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
            Err(err) => {
                if is_auth_expired_error(&err) {
                    report.auth_expired = true;
                }
                report.errors.push(format!("pull_albums: {err}"));
            }
        }
    }

    pub(super) async fn pull_remote_artists(&self, acct: &str, report: &mut SyncReport) {
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
            Err(err) => {
                if is_auth_expired_error(&err) {
                    report.auth_expired = true;
                }
                report.errors.push(format!("pull_artists: {err}"));
            }
        }
    }
}
