#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;

use rusqlite::Connection;
use tempfile::tempdir;

use musick_core::{
    AppPaths, ArtistRef, CacheManager, CacheSettings, LyricLine, Lyrics, PlaylistSource,
    SavedCollection, SavedCollectionKind, Track, UserStore,
};

fn test_paths() -> (tempfile::TempDir, AppPaths) {
    let dir = tempdir().unwrap();
    let paths = AppPaths::from_roots(
        dir.path().join("cfg"),
        dir.path().join("data"),
        dir.path().join("cache"),
        dir.path().join("rt"),
    );
    (dir, paths)
}

fn sample_track(id: &str, title: &str) -> Track {
    Track {
        video_id: id.into(),
        title: title.into(),
        artists: vec![ArtistRef {
            name: "Daft Punk".into(),
            id: Some("UC123".into()),
        }],
        album: Some("Discovery".into()),
        album_id: Some("MPREb_1".into()),
        duration: Some(210),
        thumbs: Vec::new(),
        plays: Some("10M".into()),
        video_type: Some("MUSIC_VIDEO_TYPE_ATV".into()),
        set_video_id: None,
        is_available: true,
    }
}

fn valid_png_bytes(payload_len: usize) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
    b.resize(payload_len.max(16), 0x42);
    b
}

#[test]
fn migrations_are_idempotent_and_playlist_crud_reorders_cleanly() {
    let (_dir, paths) = test_paths();
    let store1 = UserStore::open(&paths).unwrap();
    drop(store1);
    let store = UserStore::open(&paths).unwrap();

    let pl = store
        .create_playlist(
            "local",
            "Late Night",
            "Chill beats",
            PlaylistSource::Local,
            None,
            None,
        )
        .unwrap();
    assert_eq!(pl.track_count, 0);

    let t1 = sample_track("vid1", "One More Time");
    let t2 = sample_track("vid2", "Aerodynamic");
    let t3 = sample_track("vid3", "Digital Love");
    store.add_tracks_to_playlist(&pl.id, &[t1, t2, t3]).unwrap();

    let tracks = store.playlist_tracks(&pl.id).unwrap();
    assert_eq!(
        tracks
            .iter()
            .map(|t| t.video_id.as_str())
            .collect::<Vec<_>>(),
        vec!["vid1", "vid2", "vid3"]
    );

    // Move track 2 -> 0
    assert!(store.move_track(&pl.id, 2, 0).unwrap());
    let tracks = store.playlist_tracks(&pl.id).unwrap();
    assert_eq!(
        tracks
            .iter()
            .map(|t| t.video_id.as_str())
            .collect::<Vec<_>>(),
        vec!["vid3", "vid1", "vid2"]
    );

    // Remove middle track -> contiguous positions preserved
    let removed = store.remove_track_at(&pl.id, 1).unwrap().unwrap();
    assert_eq!(removed.video_id, "vid1");
    let tracks = store.playlist_tracks(&pl.id).unwrap();
    assert_eq!(
        tracks
            .iter()
            .map(|t| t.video_id.as_str())
            .collect::<Vec<_>>(),
        vec!["vid3", "vid2"]
    );

    // Rename
    let renamed = store
        .rename_playlist(&pl.id, "Midnight Vibes", Some("Updated"))
        .unwrap();
    assert_eq!(renamed.title, "Midnight Vibes");
    assert_eq!(renamed.description, "Updated");
    assert_eq!(renamed.track_count, 2);

    // Export & Import JSON and M3U8
    let exported_json = store.export_playlist_json(&pl.id).unwrap();
    let imported_json = store.import_playlist_json("local", &exported_json).unwrap();
    assert_eq!(imported_json.title, "Midnight Vibes");
    assert_eq!(imported_json.track_count, 2);

    let exported_m3u8 = store.export_playlist_m3u8(&pl.id).unwrap();
    let imported_m3u8 = store
        .import_playlist_m3u8("local", "Fallback", &exported_m3u8)
        .unwrap();
    assert_eq!(imported_m3u8.title, "Midnight Vibes");
    assert_eq!(imported_m3u8.track_count, 2);
}

#[test]
fn account_isolation_purges_remote_rows_without_touching_local_data() {
    let (_dir, paths) = test_paths();
    let store = UserStore::open(&paths).unwrap();
    let acct = "ytm:abcdef0123456789";

    let local_pl = store
        .create_playlist(
            "local",
            "My Local List",
            "",
            PlaylistSource::Local,
            None,
            None,
        )
        .unwrap();
    let synced_pl = store
        .create_playlist(
            acct,
            "My YTM Cloud List",
            "",
            PlaylistSource::YtmSynced,
            Some("PL_REMOTE_1"),
            None,
        )
        .unwrap();
    store
        .add_tracks_to_playlist(&local_pl.id, &[sample_track("l1", "Local Song")])
        .unwrap();
    store
        .add_tracks_to_playlist(&synced_pl.id, &[sample_track("r1", "Remote Song")])
        .unwrap();

    store
        .like_track("local", &sample_track("l_like", "Local Like"))
        .unwrap();
    store
        .like_track(acct, &sample_track("r_like", "Remote Like"))
        .unwrap();

    store
        .save_collection(&SavedCollection {
            account_key: "local".into(),
            kind: SavedCollectionKind::Album,
            browse_id: "MPREb_local".into(),
            title: "Local Album".into(),
            subtitle: None,
            thumb_url: None,
            extra_json: "{}".into(),
            saved_at: 1,
        })
        .unwrap();
    store
        .save_collection(&SavedCollection {
            account_key: acct.into(),
            kind: SavedCollectionKind::Album,
            browse_id: "MPREb_remote".into(),
            title: "Remote Album".into(),
            subtitle: None,
            thumb_url: None,
            extra_json: "{}".into(),
            saved_at: 2,
        })
        .unwrap();
    store
        .enqueue_outbox(acct, "rename_playlist", &synced_pl.id, "{}")
        .unwrap();

    // Logout purges only `acct`, never `'local'`
    store.purge_account(acct).unwrap();
    store.purge_account("local").unwrap(); // must be a strict no-op

    let remaining_pls = store.list_playlists("local").unwrap();
    assert_eq!(remaining_pls.len(), 1);
    assert_eq!(remaining_pls[0].id, local_pl.id);
    assert_eq!(store.playlist_tracks(&local_pl.id).unwrap().len(), 1);
    assert!(store.get_playlist(&synced_pl.id).unwrap().is_none());

    let likes = store.list_liked_songs("local").unwrap();
    assert_eq!(likes.len(), 1);
    assert_eq!(likes[0].video_id, "l_like");

    let albums = store
        .list_saved_collections("local", SavedCollectionKind::Album)
        .unwrap();
    assert_eq!(albums.len(), 1);
    assert_eq!(albums[0].browse_id, "MPREb_local");
    assert_eq!(store.count_outbox(acct).unwrap(), 0);
}

#[test]
fn cache_manager_clear_preserves_user_db_rotates_visitor_data_and_self_heals_corruption() {
    let (_dir, paths) = test_paths();
    let user_store = UserStore::open(&paths).unwrap();
    let pl = user_store
        .create_playlist("local", "Untouched", "", PlaylistSource::Local, None, None)
        .unwrap();

    let cache = CacheManager::open(&paths, CacheSettings::default()).unwrap();
    cache.set_visitor_data("VISITOR_123");
    assert_eq!(cache.get_visitor_data().as_deref(), Some("VISITOR_123"));

    cache.put_catalog("home:ID", &vec!["mix1", "mix2"], Some(600));
    let cached_home: Vec<String> = cache.get_catalog("home:ID").unwrap();
    assert_eq!(cached_home, vec!["mix1", "mix2"]);

    let lyr = Lyrics {
        lines: vec![LyricLine::new(1.0, "Hello")],
        synced: true,
        source: "LRCLIB".into(),
        translation_lang: String::new(),
    };
    cache.put_lyrics("vid1:auto", &lyr);
    assert_eq!(cache.get_lyrics("vid1:auto").unwrap(), lyr);

    let img_url = "https://lh3.googleusercontent.com/cover1";
    cache.put_image_bytes(img_url, valid_png_bytes(512));
    assert!(cache.get_image_bytes(img_url).is_some());

    // Simulate corrupt JSON in cache.db and truncated blob on disk -> self-healing!
    {
        let raw_conn = Connection::open(paths.cache_db_file()).unwrap();
        raw_conn
            .execute(
                "INSERT OR REPLACE INTO catalog_cache
                    (cache_key, value_json, size_bytes, created_at, expires_at, last_accessed)
                 VALUES ('corrupt_key', '{not valid json', 15, 1, 9999999999, 1)",
                [],
            )
            .unwrap();
        raw_conn
            .execute(
                "INSERT OR REPLACE INTO lyrics_cache
                    (cache_key, value_json, size_bytes, created_at, expires_at, last_accessed)
                 VALUES ('corrupt_lyr', '[[broken', 8, 1, 9999999999, 1)",
                [],
            )
            .unwrap();
    }
    assert!(cache.get_catalog::<Vec<String>>("corrupt_key").is_none());
    assert!(cache.get_lyrics("corrupt_lyr").is_none());

    // Corrupt image file directly on disk (HTML error page / truncated)
    let corrupt_url = "https://lh3.googleusercontent.com/corrupt";
    cache.put_image_bytes(corrupt_url, valid_png_bytes(128));
    // Re-open cache manager to bypass L1
    let cache2 = CacheManager::open(&paths, CacheSettings::default()).unwrap();
    for entry in fs::read_dir(paths.images_cache_dir()).unwrap() {
        let p = entry.unwrap().path();
        fs::write(&p, b"<html>502</html>").unwrap();
    }
    assert!(cache2.get_image_bytes(corrupt_url).is_none());

    // Clear cache wipes cache.db + visitor_data + images, while user_store remains intact
    cache2.clear_cache().unwrap();
    assert!(cache2.get_visitor_data().is_none());
    assert_eq!(cache2.stats().unwrap().total_bytes, 0);
    assert!(user_store.get_playlist(&pl.id).unwrap().is_some());
}

#[test]
fn cache_prune_enforces_lru_byte_and_file_budgets_and_removes_orphans() {
    let (_dir, paths) = test_paths();
    let cache = CacheManager::open(&paths, CacheSettings::default()).unwrap();

    for i in 0..6 {
        let url = format!("https://lh3.googleusercontent.com/img_{i}");
        cache.put_image_bytes(&url, valid_png_bytes(1024));
    }
    // Create an untracked orphan blob in images/
    let orphan_path = paths.images_cache_dir().join("orphan_deadbeef.bin");
    fs::write(&orphan_path, valid_png_bytes(256)).unwrap();

    // Enforce max 2 files & max 2500 bytes
    let report = cache
        .prune_with_budgets(1024 * 1024, 1024 * 1024, 2500, 2, 30 * 86_400)
        .unwrap();
    assert_eq!(report.images_evicted, 4);
    assert_eq!(report.orphan_files_removed, 1);
    assert!(!orphan_path.exists());

    let stats = cache.stats().unwrap();
    assert_eq!(stats.image_files, 2);
    assert!(stats.image_bytes <= 2500);
}
