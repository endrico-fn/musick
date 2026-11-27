#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use serde_json::{Map, Value, json};
use tempfile::tempdir;

use musick_core::innertube::parsers::{
    extract_plays_from_browse, extract_visitor_data, parse_album, parse_artist,
    parse_artist_releases_with_continuation, parse_discover_feed, parse_playlist,
    parse_playlist_edit_results, parse_search_results, parse_search_results_with_continuation,
    parse_spelling_hint, parse_suggestions, parse_two_row_item, parse_watch_playlist,
    parse_yt_lyrics,
};
use musick_core::innertube::search_filter_params;
use musick_core::{
    AppPaths, AuthManager, CacheManager, CacheSettings, CatalogApi, ClientKind, InnerTubeClient,
    MemorySecretStore, PlaybackTelemetryEvent, PlaybackTracking, SearchCategory, SearchItem,
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

    let feed = parse_discover_feed(Some(&home), Some(&new_rel), None, None);
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
    assert_eq!(
        edit.added_set_video_ids,
        vec![("v1".into(), "SET99".into())]
    );
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
    let tracking = client.fetch_playback_tracking("vid_x", true).await.unwrap();
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

    // 4. Region updates cleanly via CatalogApi trait
    let api: Arc<dyn CatalogApi> = Arc::new(client);
    api.set_region("US");
}

#[test]
fn parses_live_youtube_music_shapes_for_thumbnails_radio_album_playlist_and_search_params() {
    // 1. musicTwoRowItemRenderer uses `thumbnailRenderer.musicThumbnailRenderer.thumbnail.thumbnails`
    let two_row = json!({
        "title": { "runs": [{ "text": "Mustahil" }] },
        "subtitle": { "runs": [{ "text": "Single" }, { "text": " • " }, { "text": "Robi DA7" }] },
        "navigationEndpoint": {
            "browseEndpoint": {
                "browseId": "MPREb_z43rR5FKyfe",
                "browseEndpointContextSupportedConfigs": {
                    "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ALBUM" }
                }
            }
        },
        "thumbnailRenderer": {
            "musicThumbnailRenderer": {
                "thumbnail": {
                    "thumbnails": [
                        { "url": "https://yt3.googleusercontent.com/abc=w226-h226-l90-rj", "width": 226, "height": 226 },
                        { "url": "https://yt3.googleusercontent.com/abc=w544-h544-l90-rj", "width": 544, "height": 544 }
                    ]
                }
            }
        }
    });
    match parse_two_row_item(&two_row).unwrap() {
        SearchItem::Album(al) => {
            assert_eq!(al.browse_id, "MPREb_z43rR5FKyfe");
            assert_eq!(al.thumbs.len(), 2);
            assert!(al.cover_url().unwrap().contains("w300-h300"));
        }
        other => panic!("Expected Album, got {other:?}"),
    }

    // 2. Live artist header uses `watchEndpoint.playlistId` on startRadioButton & playButton
    let artist_live = json!({
        "header": {
            "musicImmersiveHeaderRenderer": {
                "title": { "runs": [{ "text": "Daft Punk" }] },
                "startRadioButton": {
                    "buttonRenderer": {
                        "navigationEndpoint": {
                            "watchEndpoint": { "playlistId": "RDEM_LIVE_RADIO", "videoId": "v1" }
                        }
                    }
                },
                "playButton": {
                    "buttonRenderer": {
                        "navigationEndpoint": {
                            "watchEndpoint": { "playlistId": "RDAO_LIVE_SHUFFLE", "videoId": "v2" }
                        }
                    }
                }
            }
        }
    });
    let artist = parse_artist("UCRr1", &artist_live);
    assert_eq!(artist.radio_id.as_deref(), Some("RDEM_LIVE_RADIO"));
    assert_eq!(artist.shuffle_id.as_deref(), Some("RDAO_LIVE_SHUFFLE"));

    // 3. Live album header exposes `OLAK5uy_...` audioPlaylistId for `like/like` library saving
    let album_live = json!({
        "contents": {
            "twoColumnBrowseResultsRenderer": {
                "tabs": [{
                    "tabRenderer": {
                        "content": {
                            "sectionListRenderer": {
                                "contents": [{
                                    "musicResponsiveHeaderRenderer": {
                                        "title": { "runs": [{ "text": "Random Access Memories" }] },
                                        "buttons": [{
                                            "musicPlayButtonRenderer": {
                                                "playNavigationEndpoint": {
                                                    "watchPlaylistEndpoint": { "playlistId": "OLAK5uy_real_album_pid" }
                                                }
                                            }
                                        }]
                                    }
                                }]
                            }
                        }
                    }
                }]
            }
        }
    });
    let alb = parse_album("MPREb_RAM", &album_live);
    assert_eq!(
        alb.audio_playlist_id.as_deref(),
        Some("OLAK5uy_real_album_pid")
    );

    // 4. Live playlist header stores author in `facepile.avatarStackViewModel.text.content`
    let playlist_live = json!({
        "contents": {
            "twoColumnBrowseResultsRenderer": {
                "tabs": [{
                    "tabRenderer": {
                        "content": {
                            "sectionListRenderer": {
                                "contents": [{
                                    "musicResponsiveHeaderRenderer": {
                                        "title": { "runs": [{ "text": "Daft Punk on shuffle" }] },
                                        "subtitle": { "runs": [{ "text": "Playlist" }, { "text": " • " }, { "text": "2023" }] },
                                        "facepile": {
                                            "avatarStackViewModel": {
                                                "text": { "content": "Aleksy Oleksy" }
                                            }
                                        }
                                    }
                                }]
                            }
                        }
                    }
                }]
            }
        }
    });
    let (pl, _) = parse_playlist("PL_LIVE", &playlist_live);
    assert_eq!(pl.summary.author.as_deref(), Some("Aleksy Oleksy"));

    // 5. Featured Playlists vs Community Playlists use distinct raw base64 params (no `%3D`)
    let pl_param = search_filter_params(SearchCategory::Playlists).unwrap();
    let comm_param = search_filter_params(SearchCategory::Community).unwrap();
    assert_ne!(pl_param, comm_param);
    assert!(!pl_param.contains("%3D"));
    assert!(!comm_param.contains("%3D"));
}

#[test]
fn parses_playlist_second_subtitle_split_and_append_continuation_items() {
    let raw = json!({
        "header": {
            "musicDetailHeaderRenderer": {
                "title": { "runs": [{ "text": "Midnight Synth" }] },
                "secondSubtitle": {
                    "runs": [
                        { "text": "1.2M views" },
                        { "text": " • " },
                        { "text": "50 songs" },
                        { "text": " • " },
                        { "text": "3 hours 10 minutes" }
                    ]
                }
            }
        },
        "onResponseReceivedActions": [{
            "appendContinuationItemsAction": {
                "continuationItems": [
                    {
                        "musicResponsiveListItemRenderer": {
                            "flexColumns": [
                                {
                                    "musicResponsiveListItemFlexColumnRenderer": {
                                        "text": {
                                            "runs": [{
                                                "text": "Nightcall",
                                                "navigationEndpoint": {
                                                    "watchEndpoint": { "videoId": "cont_vid_1" }
                                                }
                                            }]
                                        }
                                    }
                                },
                                {
                                    "musicResponsiveListItemFlexColumnRenderer": {
                                        "text": {
                                            "runs": [
                                                { "text": "Kavinsky" },
                                                { "text": " & " },
                                                { "text": "Lovefoxxx" }
                                            ]
                                        }
                                    }
                                }
                            ]
                        }
                    },
                    {
                        "continuationItemRenderer": {
                            "continuationEndpoint": {
                                "continuationCommand": { "token": "NEXT_APPEND_TOKEN" }
                            }
                        }
                    }
                ]
            }
        }]
    });

    let (pl, cont) = parse_playlist("PL_CONT", &raw);
    assert_eq!(pl.views.as_deref(), Some("1.2M views"));
    assert_eq!(pl.summary.count.as_deref(), Some("50 songs"));
    assert_eq!(pl.duration_text.as_deref(), Some("3 hours 10 minutes"));
    assert_eq!(pl.tracks.len(), 1);
    assert_eq!(pl.tracks[0].video_id, "cont_vid_1");
    assert_eq!(pl.tracks[0].artists.len(), 2);
    assert_eq!(pl.tracks[0].artists[0].name, "Kavinsky");
    assert_eq!(pl.tracks[0].artists[1].name, "Lovefoxxx");
    assert_eq!(cont.as_deref(), Some("NEXT_APPEND_TOKEN"));
}

#[test]
fn parses_artist_views_separates_featured_and_live_shelves_and_fixes_singles_kind() {
    let raw = json!({
        "header": {
            "musicImmersiveHeaderRenderer": {
                "title": { "runs": [{ "text": "Justice" }] }
            }
        },
        "contents": {
            "singleColumnBrowseResultsRenderer": {
                "tabs": [{
                    "tabRenderer": {
                        "content": {
                            "sectionListRenderer": {
                                "contents": [
                                    {
                                        "musicDescriptionShelfRenderer": {
                                            "subheader": { "runs": [{ "text": "910M views" }] },
                                            "description": { "runs": [{ "text": "French electronic duo" }] }
                                        }
                                    },
                                    {
                                        "musicCarouselShelfRenderer": {
                                            "header": {
                                                "musicCarouselShelfBasicHeaderRenderer": {
                                                    "title": { "runs": [{ "text": "Albums" }] }
                                                }
                                            },
                                            "contents": [{
                                                "musicTwoRowItemRenderer": {
                                                    "title": { "runs": [{ "text": "Cross" }] },
                                                    "subtitle": { "runs": [{ "text": "2007" }] },
                                                    "navigationEndpoint": {
                                                        "browseEndpoint": { "browseId": "MPREb_cross" }
                                                    }
                                                }
                                            }]
                                        }
                                    },
                                    {
                                        "musicCarouselShelfRenderer": {
                                            "header": {
                                                "musicCarouselShelfBasicHeaderRenderer": {
                                                    "title": { "runs": [{ "text": "Featured on" }] }
                                                }
                                            },
                                            "contents": [
                                                {
                                                    "musicTwoRowItemRenderer": {
                                                        "title": { "runs": [{ "text": "Electro Compil" }] },
                                                        "subtitle": { "runs": [{ "text": "Album" }] },
                                                        "navigationEndpoint": {
                                                            "browseEndpoint": { "browseId": "MPREb_comp" }
                                                        }
                                                    }
                                                },
                                                {
                                                    "musicTwoRowItemRenderer": {
                                                        "title": { "runs": [{ "text": "French Touch Essentials" }] },
                                                        "subtitle": { "runs": [{ "text": "YouTube Music" }] },
                                                        "navigationEndpoint": {
                                                            "browseEndpoint": { "browseId": "VLRDCLAK_ft" }
                                                        }
                                                    }
                                                }
                                            ]
                                        }
                                    },
                                    {
                                        "musicCarouselShelfRenderer": {
                                            "header": {
                                                "musicCarouselShelfBasicHeaderRenderer": {
                                                    "title": { "runs": [{ "text": "Live performances" }] }
                                                }
                                            },
                                            "contents": [{
                                                "musicTwoRowItemRenderer": {
                                                    "title": { "runs": [{ "text": "Genesis (Live)" }] },
                                                    "subtitle": { "runs": [{ "text": "Justice" }, { "text": " • " }, { "text": "12M plays" }] },
                                                    "navigationEndpoint": {
                                                        "watchEndpoint": { "videoId": "live_v1" }
                                                    }
                                                }
                                            }]
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }]
            }
        }
    });

    let artist = parse_artist("UC_just", &raw);
    assert_eq!(artist.views.as_deref(), Some("910M views"));
    assert_eq!(artist.albums.len(), 1);
    assert_eq!(artist.albums[0].browse_id, "MPREb_cross");
    assert_eq!(artist.playlists.len(), 1);
    assert_eq!(artist.playlists[0].browse_id, "RDCLAK_ft");
    assert_eq!(artist.videos.len(), 1);
    assert_eq!(artist.videos[0].video_id, "live_v1");
    assert_eq!(artist.videos[0].plays.as_deref(), Some("12M"));

    // Artist Singles release page: overrides default "Album" -> "Single" while preserving explicit "EP"
    let singles_raw = json!({
        "contents": {
            "gridRenderer": {
                "items": [
                    {
                        "musicTwoRowItemRenderer": {
                            "title": { "runs": [{ "text": "Generator" }] },
                            "subtitle": { "runs": [{ "text": "2024" }] },
                            "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREb_gen" } }
                        }
                    },
                    {
                        "musicTwoRowItemRenderer": {
                            "title": { "runs": [{ "text": "Planisphere" }] },
                            "subtitle": { "runs": [{ "text": "EP" }, { "text": " • " }, { "text": "2008" }] },
                            "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREb_plan" } }
                        }
                    }
                ]
            }
        }
    });
    let (singles, _) = parse_artist_releases_with_continuation(&singles_raw, "Single");
    assert_eq!(singles.len(), 2);
    assert_eq!(singles[0].kind, "Single");
    assert_eq!(singles[1].kind, "EP");
}

#[test]
fn parses_mpla_artist_card_shelf_metadata_spelling_hint_and_entity_suggestions() {
    // 1. MPLA browseId maps to Artist with stripped UC id
    let mpla_row = json!({
        "title": { "runs": [{ "text": "Daft Punk" }] },
        "subtitle": { "runs": [{ "text": "Artist" }] },
        "navigationEndpoint": {
            "browseEndpoint": { "browseId": "MPLAUC_dp_123" }
        }
    });
    match parse_two_row_item(&mpla_row).unwrap() {
        SearchItem::Artist(a) => assert_eq!(a.browse_id, "UC_dp_123"),
        other => panic!("expected Artist, got {other:?}"),
    }

    // 2. musicCardShelfRenderer preserves Track album/plays/unlinked artists + continuation + didYouMean
    let search_raw = json!({
        "contents": {
            "tabbedSearchResultsRenderer": {
                "tabs": [{
                    "tabRenderer": {
                        "content": {
                            "sectionListRenderer": {
                                "contents": [
                                    {
                                        "itemSectionRenderer": {
                                            "contents": [{
                                                "didYouMeanRenderer": {
                                                    "correctedQuery": { "runs": [{ "text": "daft punk" }] }
                                                }
                                            }]
                                        }
                                    },
                                    {
                                        "musicCardShelfRenderer": {
                                            "title": {
                                                "runs": [{
                                                    "text": "Instant Crush",
                                                    "navigationEndpoint": {
                                                        "watchEndpoint": { "videoId": "top_v1" }
                                                    }
                                                }]
                                            },
                                            "subtitle": {
                                                "runs": [
                                                    { "text": "Song" },
                                                    { "text": " • " },
                                                    { "text": "Daft Punk", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCdp" } } },
                                                    { "text": " & " },
                                                    { "text": "Julian Casablancas" },
                                                    { "text": " • " },
                                                    { "text": "Random Access Memories", "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREb_ram" } } },
                                                    { "text": " • " },
                                                    { "text": "530M plays" },
                                                    { "text": " • " },
                                                    { "text": "5:37" }
                                                ]
                                            }
                                        }
                                    }
                                ],
                                "continuations": [{
                                    "nextContinuationData": { "continuation": "SEARCH_CONT_2" }
                                }]
                            }
                        }
                    }
                }]
            }
        }
    });

    assert_eq!(
        parse_spelling_hint(&search_raw).as_deref(),
        Some("daft punk")
    );
    let (items, cont) = parse_search_results_with_continuation(&search_raw);
    assert_eq!(cont.as_deref(), Some("SEARCH_CONT_2"));
    assert_eq!(items.len(), 1);
    match &items[0] {
        SearchItem::Track(t) => {
            assert_eq!(t.video_id, "top_v1");
            assert_eq!(t.artists.len(), 2);
            assert_eq!(t.artists[0].name, "Daft Punk");
            assert_eq!(t.artists[1].name, "Julian Casablancas");
            assert_eq!(t.album.as_deref(), Some("Random Access Memories"));
            assert_eq!(t.album_id.as_deref(), Some("MPREb_ram"));
            assert_eq!(t.plays.as_deref(), Some("530M"));
            assert_eq!(t.duration, Some(337));
        }
        other => panic!("expected top track, got {other:?}"),
    }

    // 3. Entity suggestions inside musicResponsiveListItemRenderer are included alongside text suggestions
    let sugg_raw = json!({
        "contents": [{
            "searchSuggestionsSectionRenderer": {
                "contents": [
                    {
                        "searchSuggestionRenderer": {
                            "navigationEndpoint": { "searchEndpoint": { "query": "daft punk" } }
                        }
                    },
                    {
                        "musicResponsiveListItemRenderer": {
                            "flexColumns": [{
                                "musicResponsiveListItemFlexColumnRenderer": {
                                    "text": { "runs": [{ "text": "Discovery" }] }
                                }
                            }]
                        }
                    }
                ]
            }
        }]
    });
    assert_eq!(
        parse_suggestions(&sugg_raw),
        vec!["daft punk".to_string(), "Discovery".to_string()]
    );
}
