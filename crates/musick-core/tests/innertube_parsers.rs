#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use serde_json::{Map, Value, json};
use tempfile::tempdir;

use musick_core::innertube::parsers::{
    extract_plays_from_browse, extract_visitor_data, parse_album, parse_artist, parse_home_feed,
    parse_playlist, parse_playlist_edit_results, parse_search_results, parse_watch_playlist,
    parse_yt_lyrics,
};
use musick_core::{
    AppPaths, AuthManager, CacheManager, CacheSettings, CatalogApi, ClientKind, InnerTubeClient,
    MemorySecretStore, PlaybackTelemetryEvent, PlaybackTracking, SearchItem,
};

fn load_fixture(name: &str) -> Value {
    let raw = match name {
        "home.json" => include_str!("fixtures/home.json"),
        "new_releases.json" => include_str!("fixtures/new_releases.json"),
        "search.json" => include_str!("fixtures/search.json"),
        "artist.json" => include_str!("fixtures/artist.json"),
        "album.json" => include_str!("fixtures/album.json"),
        "playlist.json" => include_str!("fixtures/playlist.json"),
        "watch.json" => include_str!("fixtures/watch.json"),
        "lyrics.json" => include_str!("fixtures/lyrics.json"),
        other => panic!("Unknown fixture {other}"),
    };
    serde_json::from_str(raw).unwrap()
}

#[test]
fn parses_home_mixes_and_new_releases_and_visitor_data() {
    let home = load_fixture("home.json");
    let new_rel = load_fixture("new_releases.json");

    assert_eq!(
        extract_visitor_data(&home).as_deref(),
        Some("CgtWT19WSVNJVE9SXzEyMw%3D%3D")
    );

    let feed = parse_home_feed(Some(&home), Some(&new_rel), None, None);
    assert_eq!(feed.mixes.len(), 1);
    assert_eq!(feed.mixes[0].video_id, "mix_v1");
    assert_eq!(feed.mixes[0].title, "Instant Crush");
    assert_eq!(feed.mixes[0].plays.as_deref(), Some("337M"));
    assert_eq!(
        feed.mixes[0].video_type.as_deref(),
        Some("MUSIC_VIDEO_TYPE_ATV")
    );

    assert_eq!(feed.new_releases.len(), 1);
    match &feed.new_releases[0] {
        SearchItem::Album(al) => {
            assert_eq!(al.browse_id, "MPREb_hyper");
            assert_eq!(al.title, "Hyperdrama");
            assert_eq!(al.year.as_deref(), Some("2024"));
            assert_eq!(al.artists[0].name, "Justice");
        }
        other => panic!("Expected album in new_releases, got {other:?}"),
    }
}

#[test]
fn parses_search_artist_album_playlist_watch_and_timed_lyrics() {
    let search = load_fixture("search.json");
    let items = parse_search_results(&search);
    assert_eq!(items.len(), 2);
    assert!(matches!(&items[0], SearchItem::Artist(a) if a.name == "Daft Punk"));
    match &items[1] {
        SearchItem::Track(t) => {
            assert_eq!(t.video_id, "s_v1");
            assert_eq!(t.album.as_deref(), Some("Random Access Memories"));
            assert_eq!(t.duration, Some(369));
        }
        other => panic!("Expected track, got {other:?}"),
    }

    let artist_raw = load_fixture("artist.json");
    let artist = parse_artist("UCdp1", &artist_raw);
    assert_eq!(artist.summary.name, "Daft Punk");
    assert_eq!(artist.songs_playlist_id.as_deref(), Some("OLAK_top_songs"));
    assert_eq!(artist.radio_id.as_deref(), Some("RDEMdp_radio"));
    assert_eq!(artist.songs.len(), 1);
    assert_eq!(artist.songs[0].plays.as_deref(), Some("450M"));
    let plays_map = extract_plays_from_browse(&artist_raw);
    assert_eq!(plays_map.get("art_s1").map(String::as_str), Some("450M"));

    let album_raw = load_fixture("album.json");
    let album = parse_album("MPREb_disc", &album_raw);
    assert_eq!(album.summary.title, "Discovery");
    assert_eq!(album.summary.year.as_deref(), Some("2001"));
    assert_eq!(album.tracks.len(), 1);
    assert_eq!(album.tracks[0].duration, Some(320));
    assert_eq!(album.tracks[0].artists[0].name, "Daft Punk");
    assert_eq!(album.tracks[0].album.as_deref(), Some("Discovery"));

    let pl_raw = load_fixture("playlist.json");
    let (pl, cont) = parse_playlist("PL_SYNTH", &pl_raw);
    assert_eq!(pl.summary.title, "Synthwave Night Drive");
    assert_eq!(pl.tracks.len(), 1);
    assert_eq!(pl.tracks[0].set_video_id.as_deref(), Some("SET_VID_001"));
    assert_eq!(cont.as_deref(), Some("CONT_TOKEN_PL_2"));

    let watch_raw = load_fixture("watch.json");
    let (batch, lyrics_id, related_id) = parse_watch_playlist(&watch_raw);
    assert_eq!(batch.playlist_id.as_deref(), Some("RDAMVMseed1"));
    assert_eq!(batch.continuation.as_deref(), Some("RADIO_CONT_NEXT_42"));
    assert_eq!(batch.tracks.len(), 1);
    assert_eq!(batch.tracks[0].video_id, "rad_1");
    assert_eq!(batch.tracks[0].set_video_id.as_deref(), Some("PSET_1"));
    assert_eq!(lyrics_id.as_deref(), Some("MPLYt_abc123"));
    assert_eq!(related_id.as_deref(), Some("MPTRt_xyz789"));

    let lyr_raw = load_fixture("lyrics.json");
    let lyrics = parse_yt_lyrics(&lyr_raw).unwrap();
    assert!(lyrics.synced);
    assert_eq!(lyrics.lines.len(), 2);
    assert!((lyrics.lines[0].time - 12.5).abs() < 1e-6);
    assert_eq!(lyrics.lines[1].text, "Make it better");

    let edit_resp = json!({
        "playlistEditResults": [
            { "playlistEditVideoAddedResultData": { "videoId": "v1", "setVideoId": "SET99" } }
        ]
    });
    let edit = parse_playlist_edit_results(&edit_resp);
    assert_eq!(edit.added_set_video_ids, vec![("v1".into(), "SET99".into())]);
}

#[tokio::test]
async fn context_injects_visitor_data_and_brand_account_and_gates_telemetry_strictly() {
    let dir = tempdir().unwrap();
    let paths = AppPaths::from_roots(
        dir.path().join("cfg"),
        dir.path().join("data"),
        dir.path().join("cache"),
        dir.path().join("rt"),
    );
    let cache = Arc::new(CacheManager::open(&paths, CacheSettings::default()).unwrap());
    cache.set_visitor_data("VD_COHERENT_77");

    let store = Arc::new(MemorySecretStore::new());
    let http = reqwest::Client::new();
    let auth = Arc::new(AuthManager::new_anonymous(store, http.clone()));

    // Point base URL to an unreachable local port so any unintended HTTP call would fail!
    let client = InnerTubeClient::new(http, auth.clone(), cache.clone(), "ID")
        .with_base_url("http://127.0.0.1:1");

    // 1. Strict telemetry no-op in Anonymous mode even if report_enabled=true
    let tracking = client
        .fetch_playback_tracking("vid_x", true)
        .await
        .unwrap();
    assert!(tracking.is_none());

    let fake_track = PlaybackTracking {
        video_id: "vid_x".into(),
        playback_url: Some("http://127.0.0.1:1/playback".into()),
        watchtime_url: Some("http://127.0.0.1:1/watchtime".into()),
        length_secs: 200,
    };
    client
        .report_playback(&fake_track, "cpn123", PlaybackTelemetryEvent::Start, true)
        .await
        .unwrap();

    // 2. Brand Account injection in payload context
    auth.import_browser_headers("cookie: SAPISID=abc\nx-goog-pageid: 555666777", None)
        .unwrap();
    let body = client.build_request_body(ClientKind::WebRemix, "ID", Map::new());
    assert_eq!(
        body["context"]["client"]["visitorData"].as_str(),
        Some("VD_COHERENT_77")
    );
    assert_eq!(
        body["context"]["user"]["onBehalfOfUser"].as_str(),
        Some("555666777")
    );

    // 3. Even when authenticated, telemetry stays a strict no-op when report_enabled=false
    assert!(
        client
            .fetch_playback_tracking("vid_x", false)
            .await
            .unwrap()
            .is_none()
    );
    client
        .report_playback(
            &fake_track,
            "cpn123",
            PlaybackTelemetryEvent::Watchtime {
                elapsed_secs: 30.0,
                duration_secs: 200.0,
                playing: true,
            },
            false,
        )
        .await
        .unwrap();
}
