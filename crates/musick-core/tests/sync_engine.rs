#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tempfile::tempdir;

use musick_core::{
    Album, AlbumSummary, AppPaths, Artist, ArtistSummary, AuthManager, CatalogApi, CoreError,
    CoreResult, DiscoverFeed, Lyrics, MemorySecretStore, PlaybackTelemetryEvent, PlaybackTracking,
    Playlist, PlaylistEditAction, PlaylistEditResult, PlaylistSource, PlaylistSummary, RadioBatch,
    SavedCollectionKind, SearchCategory, SearchItem, SongRating, SyncEngine, Track, UserStore,
};

#[derive(Default)]
struct MockState {
    offline: bool,
    auth_expired: bool,
    fail_edits_for: Option<String>,
    playlists: HashMap<String, Playlist>,
    liked_songs: Vec<Track>,
    saved_playlist_ops: Vec<(String, bool)>,
    edit_log: Vec<(String, Vec<PlaylistEditAction>)>,
    next_id: usize,
}

#[derive(Default)]
struct MockCatalog {
    state: Mutex<MockState>,
}

impl MockCatalog {
    fn seed_playlist(&self, remote_id: &str, title: &str, tracks: Vec<Track>) {
        let mut st = self.state.lock().unwrap();
        st.playlists.insert(
            remote_id.to_string(),
            Playlist {
                summary: PlaylistSummary {
                    browse_id: remote_id.to_string(),
                    title: title.to_string(),
                    author: Some("User".into()),
                    count: Some(format!("{} songs", tracks.len())),
                    thumbs: Vec::new(),
                },
                tracks,
                description: Some("Remote desc".into()),
                views: None,
                duration_text: None,
                year: None,
                total: None,
            },
        );
    }

    fn set_offline(&self, offline: bool) {
        self.state.lock().unwrap().offline = offline;
    }

    fn delete_remote_playlist(&self, remote_id: &str) {
        self.state.lock().unwrap().playlists.remove(remote_id);
    }
}

#[async_trait]
impl CatalogApi for MockCatalog {
    async fn discover_feed(&self, _region: &str) -> CoreResult<DiscoverFeed> {
        Ok(DiscoverFeed::default())
    }
    async fn search(
        &self,
        _category: SearchCategory,
        _query: &str,
        _limit: usize,
    ) -> CoreResult<Vec<SearchItem>> {
        Ok(Vec::new())
    }
    async fn suggestions(&self, _query: &str) -> CoreResult<Vec<String>> {
        Ok(Vec::new())
    }
    async fn artist(&self, _browse_id: &str) -> CoreResult<Artist> {
        Err(CoreError::NotFound("unused".into()))
    }
    async fn artist_releases(
        &self,
        _browse_id: &str,
        _params: Option<&str>,
        _kind: &str,
    ) -> CoreResult<Vec<AlbumSummary>> {
        Ok(Vec::new())
    }
    async fn album(&self, _browse_id: &str) -> CoreResult<Album> {
        Err(CoreError::NotFound("unused".into()))
    }
    async fn playlist(&self, playlist_id: &str, _limit: usize) -> CoreResult<Playlist> {
        let st = self.state.lock().unwrap();
        if st.offline {
            return Err(CoreError::Catalog("offline".into()));
        }
        st.playlists
            .get(playlist_id)
            .cloned()
            .ok_or_else(|| CoreError::NotFound(playlist_id.to_string()))
    }
    async fn radio(
        &self,
        _video_id: Option<&str>,
        _playlist_id: Option<&str>,
        _continuation: Option<&str>,
        _limit: usize,
    ) -> CoreResult<RadioBatch> {
        Ok(RadioBatch::default())
    }
    async fn yt_lyrics(&self, _video_id: &str) -> CoreResult<Option<Lyrics>> {
        Ok(None)
    }

    async fn library_playlists(&self) -> CoreResult<Vec<PlaylistSummary>> {
        let st = self.state.lock().unwrap();
        if st.offline {
            return Err(CoreError::Catalog("offline".into()));
        }
        Ok(st.playlists.values().map(|p| p.summary.clone()).collect())
    }
    async fn library_liked_songs(&self, _limit: usize) -> CoreResult<Vec<Track>> {
        let st = self.state.lock().unwrap();
        if st.offline {
            return Err(CoreError::Catalog("offline".into()));
        }
        Ok(st.liked_songs.clone())
    }
    async fn library_albums(&self) -> CoreResult<Vec<AlbumSummary>> {
        Ok(Vec::new())
    }
    async fn library_artists(&self) -> CoreResult<Vec<ArtistSummary>> {
        Ok(Vec::new())
    }

    async fn create_playlist(
        &self,
        title: &str,
        description: &str,
        video_ids: &[String],
    ) -> CoreResult<String> {
        let mut st = self.state.lock().unwrap();
        if st.auth_expired {
            return Err(CoreError::Auth("HTTP 401 unauthorized".into()));
        }
        if st.offline {
            return Err(CoreError::Catalog("offline".into()));
        }
        st.next_id += 1;
        let rid = format!("PL_REM_{}", st.next_id);
        let tracks: Vec<Track> = video_ids
            .iter()
            .enumerate()
            .map(|(i, vid)| {
                let mut t = Track::minimal(vid.clone(), format!("Track {vid}"));
                t.set_video_id = Some(format!("SET_{rid}_{i}"));
                t
            })
            .collect();
        st.playlists.insert(
            rid.clone(),
            Playlist {
                summary: PlaylistSummary {
                    browse_id: rid.clone(),
                    title: title.to_string(),
                    author: Some("Me".into()),
                    count: Some(format!("{} songs", tracks.len())),
                    thumbs: Vec::new(),
                },
                tracks,
                description: Some(description.to_string()),
                views: None,
                duration_text: None,
                year: None,
                total: None,
            },
        );
        Ok(rid)
    }

    async fn edit_playlist(
        &self,
        remote_playlist_id: &str,
        actions: &[PlaylistEditAction],
    ) -> CoreResult<PlaylistEditResult> {
        let mut st = self.state.lock().unwrap();
        if st.offline {
            return Err(CoreError::Catalog("offline".into()));
        }
        if st.fail_edits_for.as_deref() == Some(remote_playlist_id) {
            return Err(CoreError::Catalog("404 playlist deleted remotely".into()));
        }
        st.edit_log
            .push((remote_playlist_id.to_string(), actions.to_vec()));

        let mut added_set_video_ids = Vec::new();
        if let Some(pl) = st.playlists.get_mut(remote_playlist_id) {
            for act in actions {
                match act {
                    PlaylistEditAction::AddVideo { video_id } => {
                        let svid = format!("SVID_{}_{}", video_id, pl.tracks.len() + 1);
                        let mut tr = Track::minimal(video_id.clone(), format!("Song {video_id}"));
                        tr.set_video_id = Some(svid.clone());
                        pl.tracks.push(tr);
                        added_set_video_ids.push((video_id.clone(), svid));
                    }
                    PlaylistEditAction::RemoveVideo {
                        video_id,
                        set_video_id,
                    } => {
                        pl.tracks.retain(|t| {
                            !(t.video_id == *video_id
                                && t.set_video_id.as_deref() == Some(set_video_id.as_str()))
                        });
                    }
                    PlaylistEditAction::SetName(name) => {
                        pl.summary.title = name.clone();
                    }
                    PlaylistEditAction::SetDescription(desc) => {
                        pl.description = Some(desc.clone());
                    }
                    PlaylistEditAction::MoveVideoBefore { .. } => {}
                }
            }
        }
        Ok(PlaylistEditResult {
            added_set_video_ids,
        })
    }

    async fn delete_playlist(&self, remote_playlist_id: &str) -> CoreResult<()> {
        let mut st = self.state.lock().unwrap();
        if st.offline {
            return Err(CoreError::Catalog("offline".into()));
        }
        st.playlists.remove(remote_playlist_id);
        Ok(())
    }

    async fn rate_song(&self, _video_id: &str, _rating: SongRating) -> CoreResult<()> {
        if self.state.lock().unwrap().offline {
            return Err(CoreError::Catalog("offline".into()));
        }
        Ok(())
    }

    async fn set_artist_subscription(&self, _channel_id: &str, _subscribe: bool) -> CoreResult<()> {
        Ok(())
    }

    async fn set_album_library_status(&self, _browse_id: &str, _save: bool) -> CoreResult<()> {
        Ok(())
    }

    async fn set_playlist_library_status(&self, playlist_id: &str, save: bool) -> CoreResult<()> {
        let mut st = self.state.lock().unwrap();
        if st.offline {
            return Err(CoreError::Catalog("offline".into()));
        }
        st.saved_playlist_ops.push((playlist_id.to_string(), save));
        Ok(())
    }

    async fn fetch_playback_tracking(
        &self,
        _video_id: &str,
        _report_enabled: bool,
    ) -> CoreResult<Option<PlaybackTracking>> {
        Ok(None)
    }

    async fn report_playback(
        &self,
        _tracking: &PlaybackTracking,
        _cpn: &str,
        _event: PlaybackTelemetryEvent,
        _report_enabled: bool,
    ) -> CoreResult<()> {
        Ok(())
    }
}

fn setup() -> (
    tempfile::TempDir,
    Arc<UserStore>,
    Arc<MockCatalog>,
    Arc<AuthManager>,
    SyncEngine,
) {
    let dir = tempdir().unwrap();
    let paths = AppPaths::from_roots(
        dir.path().join("cfg"),
        dir.path().join("data"),
        dir.path().join("cache"),
        dir.path().join("rt"),
    );
    let store = Arc::new(UserStore::open(&paths).unwrap());
    let catalog = Arc::new(MockCatalog::default());
    let auth = Arc::new(AuthManager::new_anonymous(
        Arc::new(MemorySecretStore::new()),
        reqwest::Client::new(),
    ));
    let sync = SyncEngine::new(store.clone(), catalog.clone(), auth.clone());
    (dir, store, catalog, auth, sync)
}

#[tokio::test]
async fn anonymous_sync_is_strict_noop() {
    let (_dir, _store, _cat, _auth, sync) = setup();
    let report = sync.sync_now().await.unwrap();
    assert!(report.skipped_anonymous);
    assert_eq!(report.pushed, 0);
}

#[tokio::test]
async fn fifo_push_captures_set_video_id_and_refreshes_missing_set_video_id_before_remove() {
    let (_dir, store, catalog, auth, sync) = setup();
    auth.import_browser_headers("cookie: SAPISID=tok123", None)
        .unwrap();

    // Create synced playlist offline -> add track -> go online -> sync pushes in FIFO order
    catalog.set_offline(true);
    let pl = sync.create_playlist("Road Trip", "Highway", true).unwrap();
    sync.add_tracks_to_playlist(&pl.id, &[Track::minimal("vidA", "Track A")])
        .unwrap();

    // First sync attempt fails offline without losing outbox ops
    let offline_rep = sync.sync_now().await.unwrap();
    assert_eq!(offline_rep.pushed, 0);
    assert!(offline_rep.failed >= 1);
    assert_eq!(sync.pending_count(), 2);

    // Go online -> FIFO push creates remote playlist, then adds track and stores set_video_id
    catalog.set_offline(false);
    let rep = sync.sync_now().await.unwrap();
    assert_eq!(rep.pushed, 2);
    assert_eq!(sync.pending_count(), 0);

    let synced_pl = store.get_playlist(&pl.id).unwrap().unwrap();
    let remote_id = synced_pl.remote_id.clone().unwrap();
    let tracks = store.playlist_tracks(&pl.id).unwrap();
    assert_eq!(tracks.len(), 1);
    assert!(tracks[0].set_video_id.is_some());

    // Now test removing a track whose local set_video_id is missing (None):
    // SyncEngine MUST refresh remote playlist once to discover set_video_id before RemoveVideo!
    let mut track_without_svid = Track::minimal("vidA", "Track A");
    track_without_svid.set_video_id = None;
    store
        .replace_playlist_tracks(&pl.id, &[track_without_svid])
        .unwrap();

    let removed = sync.remove_track_at(&pl.id, 0).unwrap();
    assert!(removed.is_some());
    assert!(removed.unwrap().set_video_id.is_none());

    let rep2 = sync.sync_now().await.unwrap();
    assert_eq!(rep2.pushed, 1);

    // Verify remote track was actually removed via discovered set_video_id
    let remote_after = catalog.playlist(&remote_id, 50).await.unwrap();
    assert!(remote_after.tracks.is_empty());
}

#[tokio::test]
async fn pull_preserves_local_playlists_downgrades_deleted_remote_with_pending_edits_and_syncs_community_save()
 {
    let (_dir, store, catalog, auth, sync) = setup();

    // Create a strictly local playlist BEFORE logging in
    let local_pl = sync
        .create_playlist("My Private Local", "Never auto-deleted", false)
        .unwrap();
    sync.add_tracks_to_playlist(&local_pl.id, &[Track::minimal("loc1", "Local Track")])
        .unwrap();

    // Seed two remote playlists in YouTube Music
    catalog.seed_playlist(
        "PL_KEEP",
        "Cloud Keep",
        vec![Track::minimal("c1", "Cloud Song")],
    );
    catalog.seed_playlist(
        "PL_DEL_WITH_EDIT",
        "Cloud Edited Offline",
        vec![Track::minimal("c2", "Old Song")],
    );

    // Log in & sync to mirror remote playlists
    auth.import_browser_headers("cookie: SAPISID=tok999", None)
        .unwrap();
    let rep1 = sync.sync_now().await.unwrap();
    assert_eq!(rep1.pulled_playlists, 2);

    // Local playlist must remain Local and untouched
    let local_after = store.get_playlist(&local_pl.id).unwrap().unwrap();
    assert_eq!(local_after.source, PlaylistSource::Local);
    assert_eq!(local_after.track_count, 1);

    // User edits PL_DEL_WITH_EDIT locally while it gets deleted on the browser!
    let acct = auth.account_key();
    let edited_pl = store
        .get_playlist_by_remote_id(&acct, "PL_DEL_WITH_EDIT")
        .unwrap()
        .unwrap();
    sync.rename_playlist(&edited_pl.id, "Keep My Offline Edits", None)
        .unwrap();

    // Remote browser deletes PL_DEL_WITH_EDIT and rejects edit with 404
    catalog.delete_remote_playlist("PL_DEL_WITH_EDIT");
    catalog.state.lock().unwrap().fail_edits_for = Some("PL_DEL_WITH_EDIT".into());

    let rep2 = sync.sync_now().await.unwrap();
    assert_eq!(rep2.downgraded_to_local, 1);

    let rescued = store.get_playlist(&edited_pl.id).unwrap().unwrap();
    assert_eq!(rescued.source, PlaylistSource::Local);
    assert_eq!(rescued.account_key, "local");
    assert_eq!(rescued.title, "Keep My Offline Edits");
    assert!(rescued.remote_id.is_none());

    // Promote local playlist to YTM explicitly (`U` Upload to YTM)
    catalog.state.lock().unwrap().fail_edits_for = None;
    let promoted = sync.promote_local_to_synced(&local_pl.id).await.unwrap();
    assert_eq!(promoted.source, PlaylistSource::YtmSynced);
    assert!(promoted.remote_id.is_some());

    // Save community playlist and verify remote `set_playlist_library_status` sync
    let saved = sync
        .toggle_save_collection(
            SavedCollectionKind::Playlist,
            "RDCLAK5uy_comm1",
            "Lo-Fi Beats",
            Some("Community".into()),
            None,
        )
        .unwrap();
    assert!(saved);
    let _ = sync.sync_now().await.unwrap();
    assert_eq!(
        catalog.state.lock().unwrap().saved_playlist_ops,
        vec![("RDCLAK5uy_comm1".to_string(), true)]
    );
}

#[tokio::test]
async fn classifies_community_playlists_caps_outbox_retries_and_flags_auth_expiry() {
    let (_dir, store, catalog, auth, sync) = setup();
    auth.import_browser_headers("cookie: SAPISID=tok_cls", None)
        .unwrap();

    // 1. Owned PL* -> UserPlaylist; Community RDCLAK* -> SavedCollectionKind::Playlist
    catalog.seed_playlist(
        "PL_OWNED_1",
        "My Mix",
        vec![Track::minimal("t1", "Owned Song")],
    );
    catalog.seed_playlist(
        "RDCLAK5uy_editorial",
        "Editorial Mix",
        vec![Track::minimal("t2", "Editorial Song")],
    );
    let rep = sync.sync_now().await.unwrap();
    assert_eq!(rep.pulled_playlists, 1);
    assert_eq!(rep.pulled_saved_playlists, 1);
    let acct = auth.account_key();
    assert!(
        store
            .get_playlist_by_remote_id(&acct, "PL_OWNED_1")
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .get_playlist_by_remote_id(&acct, "RDCLAK5uy_editorial")
            .unwrap()
            .is_none()
    );
    let saved_pls = store
        .list_saved_collections(&acct, SavedCollectionKind::Playlist)
        .unwrap();
    assert!(
        saved_pls
            .iter()
            .any(|c| c.browse_id == "RDCLAK5uy_editorial")
    );

    // 2. Retry cap stops retrying permanently failing outbox ops after MAX_OUTBOX_RETRIES
    catalog.set_offline(true);
    let _pl = sync.create_playlist("Flaky", "", true).unwrap();
    for _ in 0..musick_core::sync::MAX_OUTBOX_RETRIES {
        let r = sync.sync_now().await.unwrap();
        assert_eq!(r.failed, 1);
        assert_eq!(r.skipped_exhausted, 0);
    }
    let exhausted = sync.sync_now().await.unwrap();
    assert_eq!(exhausted.failed, 0);
    assert_eq!(exhausted.skipped_exhausted, 1);

    // 3. 401/Auth error sets report.auth_expired = true and halts outbox loop
    catalog.set_offline(false);
    catalog.state.lock().unwrap().auth_expired = true;
    let _pl2 = sync.create_playlist("Needs Reauth", "", true).unwrap();
    let auth_rep = sync.sync_now().await.unwrap();
    assert!(auth_rep.auth_expired);
    assert_eq!(auth_rep.failed, 1);
}
