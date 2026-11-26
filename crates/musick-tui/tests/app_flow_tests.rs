#![allow(clippy::unwrap_used, clippy::expect_used)]

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use musick_audio::CavaFrame;
use musick_core::{
    AlbumSummary, AppCommand, ArtistRef, AuthMode, CacheStats, CoverSize, HomeFeed, LyricLine,
    Lyrics, PlaybackSnapshot, PlaylistSource, PositionTick, QueueSnapshot, SavedCollection,
    SavedCollectionKind, SearchItem, Settings, Track, UserPlaylist,
};
use musick_tui::keymap::Action;
use musick_tui::theme::UNICODE_ICONS;
use musick_tui::views::{
    HomeDiscoverTab, HomePanelMode, HomeView, LibraryTab, LibraryView, PlaylistsPane,
    PlaylistsView, QueueView, RenderCtx, TrackSortMode, ViewEffect,
};
use musick_tui::widgets::{ActiveModal, BrailleSpinner, CoverRenderer};

fn buf_str(term: &Terminal<TestBackend>) -> String {
    let b = term.backend().buffer();
    let mut out = String::new();
    for y in 0..b.area.height {
        for x in 0..b.area.width {
            out.push_str(b[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn make_ctx<'a>(
    snap: &'a PlaybackSnapshot,
    queue: &'a QueueSnapshot,
    lyrics: &'a Lyrics,
    cava: &'a CavaFrame,
    settings: &'a Settings,
    stats: &'a CacheStats,
) -> RenderCtx<'a> {
    RenderCtx {
        snapshot: snap,
        position: PositionTick::default(),
        queue,
        lyrics,
        cava,
        settings,
        icons: UNICODE_ICONS,
        spinner: BrailleSpinner::new(false),
        auth_mode: AuthMode::Anonymous,
        syncing: false,
        pending_sync: 0,
        cache_stats: stats,
    }
}

fn sample_track(id: &str, title: &str) -> Track {
    Track {
        video_id: id.into(),
        title: title.into(),
        artists: vec![ArtistRef {
            name: "Daft Punk".into(),
            id: Some("UCdp".into()),
        }],
        album: Some("Discovery".into()),
        album_id: Some("MPREb1".into()),
        duration: Some(215),
        thumbs: Vec::new(),
        plays: Some("100M".into()),
        video_type: Some("MUSIC_VIDEO_TYPE_ATV".into()),
        set_video_id: None,
        is_available: true,
    }
}

#[test]
fn home_view_defaults_to_mixes_new_releases_handles_search_and_expanded_lyrics() {
    let mut home = HomeView::new();
    let snap = PlaybackSnapshot {
        track: Some(sample_track("v1", "Instant Crush")),
        playing: true,
        ..PlaybackSnapshot::default()
    };
    let queue = QueueSnapshot::default();
    let lyrics = Lyrics {
        lines: vec![LyricLine::new(1.0, "I didn't want to be the one to forget")],
        synced: true,
        source: "LRCLIB".into(),
        translation_lang: String::new(),
    };
    let cava = CavaFrame::default();
    let settings = Settings::default();
    let stats = CacheStats::default();
    let ctx = make_ctx(&snap, &queue, &lyrics, &cava, &settings, &stats);
    let mut covers = CoverRenderer::halfblocks_for_test();

    // 1. Loading state renders Braille spinner (zero skeleton blocks)
    let mut term = Terminal::new(TestBackend::new(110, 28)).unwrap();
    term.draw(|f| home.render(f, Rect::new(0, 0, 110, 28), &ctx, &mut covers))
        .unwrap();
    let loading_text = buf_str(&term);
    assert!(loading_text.contains("Discover"));
    assert!(loading_text.contains("Mixes"));
    assert!(loading_text.contains("New Releases"));
    assert!(loading_text.contains("⠋ Loading Mixes"));
    assert!(loading_text.contains("Now Playing"));
    assert!(loading_text.contains("Cava"));

    // 2. Populate feed & switch between Mixes and New Releases via Tab
    home.set_home_feed(HomeFeed {
        mixes: vec![sample_track("m1", "Digital Love")],
        new_releases: vec![SearchItem::Album(AlbumSummary {
            browse_id: "MPREb_new".into(),
            title: "Hyperdrama".into(),
            year: Some("2024".into()),
            kind: "Album".into(),
            artists: vec![ArtistRef {
                name: "Justice".into(),
                id: None,
            }],
            thumbs: Vec::new(),
        })],
        ..HomeFeed::default()
    });
    assert_eq!(home.discover_tab, HomeDiscoverTab::Mixes);
    home.handle_action(Action::NextSubTab);
    assert_eq!(home.discover_tab, HomeDiscoverTab::NewReleases);
    term.draw(|f| home.render(f, Rect::new(0, 0, 110, 28), &ctx, &mut covers))
        .unwrap();
    assert!(buf_str(&term).contains("Hyperdrama"));

    // 3. Inline Search via `/` + submit -> Results mode
    assert!(matches!(
        home.handle_action(Action::StartSearch),
        ViewEffect::TriggerAutocomplete { .. }
    ));
    assert!(home.is_insert_mode());
    home.handle_action(Action::InputChar('d'));
    home.handle_action(Action::InputChar('p'));
    assert!(matches!(
        home.handle_action(Action::InputSubmit),
        ViewEffect::TriggerSearch { query, .. } if query == "dp"
    ));
    assert_eq!(home.mode, HomePanelMode::SearchResults);

    // 4. Expanded Lyrics via `L` flips bottom-left to Track Details
    home.toggle_expanded_lyrics();
    assert_eq!(home.mode, HomePanelMode::Lyrics);
    term.draw(|f| home.render(f, Rect::new(0, 0, 110, 28), &ctx, &mut covers))
        .unwrap();
    let expanded = buf_str(&term);
    assert!(expanded.contains("Track Details"));
    assert!(expanded.contains("I didn't want to be the one to forget"));

    // Toggle back restores SearchResults, Esc returns to Discover
    home.toggle_expanded_lyrics();
    assert_eq!(home.mode, HomePanelMode::SearchResults);
    home.handle_action(Action::Escape);
    assert_eq!(home.mode, HomePanelMode::Discover);
}

#[test]
fn playlists_library_and_queue_views_render_and_emit_expected_effects() {
    let snap = PlaybackSnapshot::default();
    let lyrics = Lyrics::default();
    let cava = CavaFrame::default();
    let settings = Settings::default();
    let stats = CacheStats::default();
    let mut covers = CoverRenderer::halfblocks_for_test();

    // Playlists View
    let mut pl_view = PlaylistsView::new();
    pl_view.set_playlists(vec![UserPlaylist {
        id: "p1".into(),
        account_key: "local".into(),
        remote_id: None,
        title: "Focus Synth".into(),
        description: String::new(),
        source: PlaylistSource::Local,
        thumb_url: None,
        remote_etag: None,
        track_count: 2,
        created_at: 1,
        updated_at: 1,
        last_synced_at: None,
    }]);
    pl_view.set_selected_tracks(vec![
        sample_track("t1", "Track One"),
        sample_track("t2", "Track Two"),
    ]);

    assert!(matches!(
        pl_view.handle_action(Action::CycleCoverSize),
        ViewEffect::OpenModal(ActiveModal::CreatePlaylist { .. })
    ));
    assert!(matches!(
        pl_view.handle_action(Action::RenamePlaylist),
        ViewEffect::OpenModal(ActiveModal::RenamePlaylist { .. })
    ));
    assert!(matches!(
        pl_view.handle_action(Action::UploadPlaylist),
        ViewEffect::UploadPlaylist { playlist_id } if playlist_id == "p1"
    ));

    // Switch to right Tracks pane and test J reorder & dd remove
    pl_view.handle_action(Action::MoveRight);
    assert_eq!(pl_view.pane, PlaylistsPane::Tracks);
    assert!(matches!(
        pl_view.handle_action(Action::MoveItemDown),
        ViewEffect::MovePlaylistTrack { from: 0, to: 1, .. }
    ));
    assert!(matches!(
        pl_view.handle_action(Action::DeleteItem),
        ViewEffect::RemovePlaylistTrack { position: 1, .. }
    ));

    // Library View (all 6 bookmark-free tabs)
    let mut lib = LibraryView::new(CoverSize::M);
    lib.liked_songs = vec![sample_track("lk1", "Liked Track")];
    lib.saved_albums = vec![SavedCollection {
        account_key: "local".into(),
        kind: SavedCollectionKind::Album,
        browse_id: "MPREb_1".into(),
        title: "Saved Album One".into(),
        subtitle: Some("2024 · Album".into()),
        thumb_url: None,
        extra_json: "{}".into(),
        saved_at: 1,
    }];
    let mut seen_tabs = Vec::new();
    for _ in 0..6 {
        seen_tabs.push(lib.active_tab);
        lib.handle_action(Action::NextSubTab);
    }
    assert_eq!(
        seen_tabs,
        vec![
            LibraryTab::UserPlaylists,
            LibraryTab::LikedSongs,
            LibraryTab::SavedAlbums,
            LibraryTab::SavedArtists,
            LibraryTab::SavedPlaylists,
            LibraryTab::Recent,
        ]
    );

    let queue = QueueSnapshot {
        tracks: vec![
            sample_track("q0", "Played"),
            sample_track("q1", "Current"),
            sample_track("q2", "Priority Next"),
            sample_track("q3", "Context"),
        ],
        index: Some(1),
        priority_count: 1,
        can_undo: true,
        summary: "2 up next (1 prioritized)".into(),
    };
    let ctx = make_ctx(&snap, &queue, &lyrics, &cava, &settings, &stats);
    let mut q_view = QueueView::new();
    let mut term = Terminal::new(TestBackend::new(96, 22)).unwrap();
    term.draw(|f| q_view.render(f, Rect::new(0, 0, 96, 22), &ctx))
        .unwrap();
    let q_rendered = buf_str(&term);
    assert!(q_rendered.contains("NOW"));
    assert!(q_rendered.contains("NEXT"));
    assert!(q_rendered.contains("DONE"));

    q_view.cursor = 2;
    assert!(matches!(
        q_view.handle_action(Action::MoveItemDown, &queue),
        ViewEffect::Command(AppCommand::MoveQueueItem { from: 2, to: 3 })
    ));
    assert!(matches!(
        q_view.handle_action(Action::Undo, &queue),
        ViewEffect::Command(AppCommand::UndoQueue)
    ));
    assert!(matches!(
        q_view.handle_action(Action::Deduplicate, &queue),
        ViewEffect::Command(AppCommand::DeduplicateQueue)
    ));
    assert!(matches!(
        q_view.handle_action(Action::ClearUpcoming, &queue),
        ViewEffect::Command(AppCommand::ClearUpcomingQueue)
    ));

    // Render LibraryView across tabs without panic
    lib.active_tab = LibraryTab::SavedAlbums;
    lib.grid_mode = true;
    term.draw(|f| lib.render(f, Rect::new(0, 0, 96, 22), &ctx, &mut covers))
        .unwrap();
    assert!(buf_str(&term).contains("Saved Album One"));
}

#[tokio::test]
async fn full_app_navigation_detail_views_settings_privacy_lock_and_narrow_72x20_render() {
    use std::sync::Arc;
    use tempfile::tempdir;

    use musick_audio::{
        AudioBackend, AudioProperties, AudioResult, BackendEvent, HybridStreamResolver,
        PlayerEngine, PlayerEngineDeps, RawStream, StreamBackend,
    };
    use musick_core::{
        Album, AppPaths, Artist, ArtistSummary, AudioQuality, AuthManager, CacheManager,
        CacheSettings, ImageService, InnerTubeClient, LyricsService, MemorySecretStore, Playlist,
        PlaylistSummary, SearchService, SyncEngine, UserStore, build_http_client,
    };
    use musick_tui::app::{App, AppServices, NavPage};
    use musick_tui::views::{AlbumView, ArtistTab, ArtistView, PlaylistDetailView};
    use tokio::sync::broadcast;

    struct DummyBackend {
        tx: broadcast::Sender<BackendEvent>,
    }
    impl DummyBackend {
        fn new() -> Self {
            let (tx, _) = broadcast::channel(8);
            Self { tx }
        }
    }
    #[async_trait::async_trait]
    impl AudioBackend for DummyBackend {
        async fn start(&self) -> AudioResult<()> {
            Ok(())
        }
        async fn stop(&self) -> AudioResult<()> {
            Ok(())
        }
        async fn load_url(&self, _url: &str) -> AudioResult<()> {
            Ok(())
        }
        async fn append_gapless(&self, _url: &str) -> AudioResult<()> {
            Ok(())
        }
        async fn clear_gapless_tail(&self) -> AudioResult<()> {
            Ok(())
        }
        async fn stop_track(&self) -> AudioResult<()> {
            Ok(())
        }
        async fn toggle_pause(&self) -> AudioResult<bool> {
            Ok(false)
        }
        async fn set_paused(&self, _paused: bool) -> AudioResult<()> {
            Ok(())
        }
        async fn seek(&self, _seconds: f64, _relative: bool) -> AudioResult<()> {
            Ok(())
        }
        async fn set_volume(&self, volume: u8) -> AudioResult<u8> {
            Ok(volume)
        }
        async fn apply_settings(&self, _settings: &Settings) -> AudioResult<()> {
            Ok(())
        }
        fn subscribe_events(&self) -> broadcast::Receiver<BackendEvent> {
            self.tx.subscribe()
        }
        fn properties(&self) -> AudioProperties {
            AudioProperties::default()
        }
    }

    struct DummyStream;
    #[async_trait::async_trait]
    impl StreamBackend for DummyStream {
        async fn resolve(&self, vid: &str, _q: AudioQuality) -> AudioResult<RawStream> {
            Ok(RawStream {
                url: format!("https://example.invalid/{vid}"),
                codec: "opus".into(),
                bitrate: 128,
                sample_rate: 48_000,
            })
        }
    }

    let dir = tempdir().unwrap();
    let paths = AppPaths::from_roots(
        dir.path().join("cfg"),
        dir.path().join("data"),
        dir.path().join("cache"),
        dir.path().join("rt"),
    );
    let http = build_http_client();
    let store = Arc::new(UserStore::open(&paths).unwrap());
    let cache = Arc::new(CacheManager::open(&paths, CacheSettings::default()).unwrap());
    let auth = Arc::new(AuthManager::new_anonymous(
        Arc::new(MemorySecretStore::new()),
        http.clone(),
    ));
    let catalog = Arc::new(
        InnerTubeClient::new(http.clone(), auth.clone(), cache.clone(), "ID")
            .with_base_url("http://127.0.0.1:1"),
    );
    let search = Arc::new(SearchService::new(
        catalog.clone(),
        store.clone(),
        Settings::default(),
    ));
    let lyrics = Arc::new(
        LyricsService::new(
            Some(catalog.clone()),
            cache.clone(),
            http.clone(),
            Settings::default(),
        )
        .with_ytdlp_captions(false),
    );
    let images = Arc::new(ImageService::new(http, cache.clone()));
    let sync = Arc::new(SyncEngine::new(
        store.clone(),
        catalog.clone(),
        auth.clone(),
    ));
    let resolver = Arc::new(HybridStreamResolver::with_backends(
        None,
        Arc::new(DummyStream),
        AudioQuality::Best,
        Arc::new(|| 1000.0),
    ));
    let (engine_handle, engine_join) = PlayerEngine::spawn(PlayerEngineDeps {
        backend: Arc::new(DummyBackend::new()),
        resolver,
        catalog: catalog.clone(),
        lyrics,
        store: store.clone(),
        auth: auth.clone(),
        settings: Settings::default(),
    });

    let mut app = App::new(
        AppServices {
            paths,
            store,
            cache,
            auth,
            catalog,
            search,
            images,
            sync,
            engine: engine_handle.clone(),
            cava: None,
        },
        Settings::default(),
        CoverRenderer::halfblocks_for_test(),
    );

    // 1. 5-tab navigation (1-5 and gt/gT)
    for tab_i in 0..5 {
        app.dispatch_action(Action::SwitchTab(tab_i));
        assert_eq!(app.active_tab, tab_i);
    }
    app.dispatch_action(Action::NextTab);
    assert_eq!(app.active_tab, 0);
    app.dispatch_action(Action::PrevTab);
    assert_eq!(app.active_tab, 4);

    // 2. SettingsView: report_playback_history is strictly locked OFF while Anonymous!
    app.settings_view.cursor = 5; // ReportPlaybackHistory row
    app.dispatch_action(Action::Activate);
    assert!(!app.settings.privacy.report_playback_history);
    assert!(
        app.status_toast
            .as_ref()
            .is_some_and(|(m, _)| m.contains("Anonymous"))
    );

    // 3. Render EVERY root tab at narrow 72x20 AND wide 120x32 without panic (Review Focus #4)
    let mut narrow = Terminal::new(TestBackend::new(72, 20)).unwrap();
    let mut wide = Terminal::new(TestBackend::new(120, 32)).unwrap();

    for tab_i in 0..5 {
        app.dispatch_action(Action::SwitchTab(tab_i));
        narrow.draw(|f| app.render(f)).unwrap();
        wide.draw(|f| app.render(f)).unwrap();
    }

    // Expanded Lyrics + Help modal at narrow 72x20
    app.dispatch_action(Action::ToggleLyrics);
    narrow.draw(|f| app.render(f)).unwrap();
    app.dispatch_action(Action::ToggleHelp);
    narrow.draw(|f| app.render(f)).unwrap();
    app.dispatch_action(Action::InputCancel);

    // 4. Push ArtistView (7 sub-tabs + split overview >= 106 + cover size cycle), AlbumView, PlaylistDetailView
    let mut art_view = ArtistView::new("UCdp1".into(), "Daft Punk".into(), CoverSize::M, false);
    art_view.set_artist(Artist {
        summary: ArtistSummary {
            browse_id: "UCdp1".into(),
            name: "Daft Punk".into(),
            subtitle: Some("32M monthly".into()),
            thumbs: Vec::new(),
        },
        description: Some("French duo".into()),
        listeners: Some("32M monthly".into()),
        songs: vec![sample_track("s1", "One More Time")],
        albums: vec![AlbumSummary {
            browse_id: "MPREb_disc".into(),
            title: "Discovery".into(),
            year: Some("2001".into()),
            kind: "Album".into(),
            artists: Vec::new(),
            thumbs: Vec::new(),
        }],
        singles: Vec::new(),
        playlists: Vec::new(),
        related: vec![ArtistSummary {
            browse_id: "UCjust".into(),
            name: "Justice".into(),
            subtitle: None,
            thumbs: Vec::new(),
        }],
        albums_params: None,
        singles_params: None,
        radio_id: None,
        songs_playlist_id: None,
        videos: Vec::new(),
        subscribers: None,
        views: None,
        albums_browse_id: None,
        singles_browse_id: None,
        videos_playlist_id: None,
        shuffle_id: None,
    });
    app.nav_stack.push(NavPage::Artist(Box::new(art_view)));

    // Wide render (>=106) shows split overview ("Latest & Top Releases")
    wide.draw(|f| app.render(f)).unwrap();
    assert!(buf_str(&wide).contains("Latest & Top Releases"));

    // Cycle all 7 Artist sub-tabs and cover size at narrow 72x20
    for _ in 0..ArtistTab::ALL.len() {
        narrow.draw(|f| app.render(f)).unwrap();
        app.dispatch_action(Action::CycleCoverSize);
        app.dispatch_action(Action::NextSubTab);
    }

    // Push AlbumView & PlaylistDetailView and render at 72x20
    let mut alb_view = AlbumView::new("MPREb_disc".into(), "Discovery".into(), false);
    alb_view.set_album(Album {
        summary: AlbumSummary {
            browse_id: "MPREb_disc".into(),
            title: "Discovery".into(),
            year: Some("2001".into()),
            kind: "Album".into(),
            artists: Vec::new(),
            thumbs: Vec::new(),
        },
        tracks: vec![sample_track("s1", "One More Time")],
        description: None,
        duration_text: Some("60 min".into()),
        audio_playlist_id: None,
    });
    app.nav_stack.push(NavPage::Album(alb_view));
    narrow.draw(|f| app.render(f)).unwrap();

    let mut pl_det = PlaylistDetailView::new("RDCLAK1".into(), "Synthwave".into(), false);
    pl_det.set_playlist(Playlist {
        summary: PlaylistSummary {
            browse_id: "RDCLAK1".into(),
            title: "Synthwave".into(),
            author: Some("Community".into()),
            count: Some("1 song".into()),
            thumbs: Vec::new(),
        },
        tracks: vec![sample_track("s2", "Nightcall")],
        description: None,
        views: None,
        duration_text: None,
        year: None,
        total: Some(1),
    });
    app.nav_stack.push(NavPage::PlaylistDetail(pl_det));
    narrow.draw(|f| app.render(f)).unwrap();

    // Backspace pops detail views cleanly back to root tab
    app.dispatch_action(Action::GoBack);
    assert_eq!(app.nav_stack.len(), 2);
    app.dispatch_action(Action::GoBack);
    app.dispatch_action(Action::GoBack);
    assert!(app.nav_stack.is_empty());

    // Bracketed paste: ignored in Normal mode, strips newlines in single-line modals, preserves newlines in BrowserAuthImport
    app.handle_paste("q\n");
    assert!(!app.should_quit);
    app.modal = Some(musick_tui::widgets::ActiveModal::CreatePlaylist {
        input: String::new(),
    });
    app.handle_paste("Chill\r\nMix");
    if let Some(musick_tui::widgets::ActiveModal::CreatePlaylist { ref input }) = app.modal {
        assert_eq!(input, "ChillMix");
    } else {
        panic!("expected CreatePlaylist modal");
    }
    app.modal = Some(musick_tui::widgets::ActiveModal::BrowserAuthImport {
        raw_headers: String::new(),
        page_id: String::new(),
        active_field: 0,
        error: None,
    });
    app.handle_paste("Cookie: SAPISID=abc\r\nOrigin: https://music.youtube.com");
    if let Some(musick_tui::widgets::ActiveModal::BrowserAuthImport {
        ref raw_headers, ..
    }) = app.modal
    {
        assert_eq!(
            raw_headers,
            "Cookie: SAPISID=abc\nOrigin: https://music.youtube.com"
        );
    } else {
        panic!("expected BrowserAuthImport modal");
    }
    app.modal = None;

    // 5. Regression: QueueView reorder updates app.queue_snap immediately even when length/index unchanged!
    engine_handle
        .send(AppCommand::PlayTracks {
            tracks: vec![
                sample_track("q1", "First"),
                sample_track("q2", "Second"),
                sample_track("q3", "Third"),
            ],
            start_index: 0,
            autoplay: false,
        })
        .await;
    for _ in 0..20 {
        app.poll_engine_and_bg();
        if app.queue_snap.tracks.len() == 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
    }
    assert_eq!(app.queue_snap.tracks[1].video_id, "q2");
    app.dispatch_action(Action::SwitchTab(3)); // Queue tab
    app.queue.cursor = 1;
    app.dispatch_action(Action::MoveItemDown); // J: swap index 1 and 2
    for _ in 0..20 {
        app.poll_engine_and_bg();
        if app.queue_snap.tracks[1].video_id == "q3" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
    }
    assert_eq!(app.queue_snap.tracks[1].video_id, "q3");
    assert_eq!(app.queue_snap.tracks[2].video_id, "q2");

    // 6. Regression: Volume `+` & Translation `t` persist across unrelated Settings changes (never snap back!)
    app.dispatch_action(Action::VolumeUp);
    app.dispatch_action(Action::VolumeUp);
    app.dispatch_action(Action::ToggleTranslation);
    for _ in 0..20 {
        app.poll_engine_and_bg();
        if app.snapshot.volume == 90 && app.snapshot.show_translation {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
    }
    assert_eq!(app.settings.player.volume, 90);
    assert!(app.settings.lyrics.translate_to_id);

    // Change unrelated setting (IconMode at row 13) + verify `h` cycles backward on AudioQuality (row 20)
    // and `h` does NOT trigger action buttons like AuthOAuth (row 0) or ClearHistory (row 7)!
    app.dispatch_action(Action::SwitchTab(4)); // Settings
    app.settings_view.cursor = 0; // AuthOAuth
    app.dispatch_action(Action::MoveLeft);
    assert!(app.modal.is_none()); // h must NOT open OAuth wizard!
    app.settings_view.cursor = 13; // IconMode
    app.dispatch_action(Action::Activate);
    app.settings_view.cursor = 20; // AudioQuality (Best -> MoveLeft -> Saver)
    app.dispatch_action(Action::MoveLeft);
    assert_eq!(
        app.settings.player.audio_quality,
        musick_core::AudioQuality::Saver
    );
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    app.poll_engine_and_bg();
    assert_eq!(app.snapshot.volume, 90); // NEVER snapped back to 80!
    assert!(app.snapshot.show_translation); // NEVER reset to false!

    // 7. Regression: PlaylistDetailView supports inline `/` filter and Esc clears filter before popping!
    let mut pl_filter = PlaylistDetailView::new("PL_FILT".into(), "Mix".into(), false);
    pl_filter.set_playlist(Playlist {
        summary: PlaylistSummary {
            browse_id: "PL_FILT".into(),
            title: "Mix".into(),
            author: Some("Curator".into()),
            count: Some("2 tracks".into()),
            thumbs: Vec::new(),
        },
        tracks: vec![
            sample_track("f1", "Midnight City"),
            sample_track("f2", "Solar Sailer"),
        ],
        description: None,
        views: None,
        duration_text: None,
        year: None,
        total: Some(2),
    });
    app.nav_stack.push(NavPage::PlaylistDetail(pl_filter));
    app.dispatch_action(Action::StartSearch);
    assert_eq!(app.input_mode(), musick_tui::keymap::InputMode::Insert);
    app.dispatch_action(Action::InputChar('s'));
    app.dispatch_action(Action::InputChar('o'));
    app.dispatch_action(Action::InputChar('l'));
    app.dispatch_action(Action::InputSubmit);
    assert_eq!(app.input_mode(), musick_tui::keymap::InputMode::Normal);
    wide.draw(|f| app.render(f)).unwrap();
    let filt_buf = buf_str(&wide);
    assert!(filt_buf.contains("Solar Sailer"));
    assert!(!filt_buf.contains("Midnight City"));
    // First Esc clears filter without popping nav_stack; second Esc pops!
    app.dispatch_action(Action::Escape);
    assert_eq!(app.nav_stack.len(), 1);
    app.dispatch_action(Action::Escape);
    assert!(app.nav_stack.is_empty());

    // 8. Regression: ArtistView never prints duplicate "subscribers subscribers"
    let mut art_sub = ArtistView::new("UCsub".into(), "Daft Punk".into(), CoverSize::M, false);
    art_sub.set_artist(Artist {
        summary: ArtistSummary {
            browse_id: "UCsub".into(),
            name: "Daft Punk".into(),
            subtitle: Some("65M monthly audience".into()),
            thumbs: Vec::new(),
        },
        description: None,
        listeners: Some("65M monthly audience".into()),
        songs: vec![sample_track("s1", "One More Time")],
        albums: Vec::new(),
        singles: Vec::new(),
        playlists: Vec::new(),
        related: Vec::new(),
        albums_params: None,
        singles_params: None,
        radio_id: None,
        songs_playlist_id: None,
        videos: Vec::new(),
        subscribers: Some("7.23M subscribers".into()),
        views: None,
        albums_browse_id: None,
        singles_browse_id: None,
        videos_playlist_id: None,
        shuffle_id: None,
    });
    app.nav_stack.push(NavPage::Artist(Box::new(art_sub)));
    wide.draw(|f| app.render(f)).unwrap();
    let art_buf = buf_str(&wide);
    assert!(art_buf.contains("7.23M subscribers"));
    assert!(!art_buf.contains("subscribers subscribers"));
    app.nav_stack.clear();

    // 9. Regression: Closing OAuthWizard ignores late BgMsg::OAuthPollCompleted!
    app.modal = None;
    app.handle_bg_msg(musick_tui::app::BgMsg::OAuthPollCompleted(Ok(
        musick_core::OAuthPollStatus::Authorized(musick_core::OAuthCredentials {
            client_id: "cid".into(),
            client_secret: "csec".into(),
            access_token: "tok".into(),
            refresh_token: "ref".into(),
            expires_at: 9999999999,
            scope: String::new(),
            token_type: "Bearer".into(),
            page_id: None,
            account_label: None,
        }),
    )));
    assert_eq!(app.settings.privacy.auth_mode, AuthMode::Anonymous);

    // 10. Regression: TrackTable renders thin scrollbar when rows exceed viewport height
    let many_tracks: Vec<Track> = (0..30)
        .map(|i| sample_track(&format!("v{i}"), &format!("Long Track Title #{i}")))
        .collect();
    app.home.set_home_feed(HomeFeed {
        mixes: many_tracks,
        ..HomeFeed::default()
    });
    app.dispatch_action(Action::SwitchTab(0));
    wide.draw(|f| app.render(f)).unwrap();
    let sb_buf = buf_str(&wide);
    assert!(sb_buf.contains('┃') || sb_buf.contains('│'));

    engine_handle.send(AppCommand::Shutdown).await;
    let _ = engine_join.await;
}

fn sample_track_with(id: &str, title: &str, artist: &str, duration: Option<u32>) -> Track {
    Track {
        video_id: id.into(),
        title: title.into(),
        artists: vec![ArtistRef {
            name: artist.into(),
            id: None,
        }],
        album: Some("Test Album".into()),
        album_id: Some("MPREb_test".into()),
        duration,
        thumbs: Vec::new(),
        plays: None,
        video_type: Some("MUSIC_VIDEO_TYPE_ATV".into()),
        set_video_id: None,
        is_available: true,
    }
}

#[test]
fn playlists_pane_aware_filter_sort_remove_and_reorder_guards() {
    use musick_core::{Playlist, PlaylistSummary};
    use musick_tui::views::PlaylistDetailView;

    let mut pl_view = PlaylistsView::new();
    pl_view.set_playlists(vec![
        UserPlaylist {
            id: "p1".into(),
            account_key: "local".into(),
            remote_id: None,
            title: "Night Drive".into(),
            description: String::new(),
            source: PlaylistSource::Local,
            thumb_url: None,
            remote_etag: None,
            track_count: 4,
            created_at: 1,
            updated_at: 1,
            last_synced_at: None,
        },
        UserPlaylist {
            id: "p2".into(),
            account_key: "local".into(),
            remote_id: None,
            title: "Morning Coffee".into(),
            description: String::new(),
            source: PlaylistSource::Local,
            thumb_url: None,
            remote_etag: None,
            track_count: 1,
            created_at: 2,
            updated_at: 2,
            last_synced_at: None,
        },
    ]);
    pl_view.set_selected_tracks(vec![
        sample_track_with("v0", "Zebra Song", "Kavinsky", Some(260)),
        sample_track_with("v1", "alpha wave", "Daft Punk", None),
        sample_track_with("v2", "Midnight City", "M83", Some(180)),
        sample_track_with("v3", "Alpha Dawn", "Kavinsky", Some(120)),
    ]);

    // 1. Pressing `o` in List pane does NOT change track_sort
    assert_eq!(pl_view.pane, PlaylistsPane::List);
    pl_view.handle_action(Action::CycleSort);
    assert_eq!(pl_view.track_sort, TrackSortMode::Original);

    // 2. Pressing `/` in List pane filters ONLY playlists, not tracks
    pl_view.handle_action(Action::StartSearch);
    assert!(pl_view.is_insert_mode());
    assert!(pl_view.filter_active);
    assert!(!pl_view.track_filter_active);
    for ch in "night".chars() {
        pl_view.handle_action(Action::InputChar(ch));
    }
    pl_view.handle_action(Action::InputSubmit);
    assert!(!pl_view.is_insert_mode());
    assert_eq!(pl_view.filtered_playlists().len(), 1);
    assert_eq!(pl_view.filtered_playlists()[0].id, "p1");
    assert!(pl_view.track_filter_query.is_empty());

    // 3. Switch to Tracks pane: `o` cycles Original -> Title -> Duration -> Original
    pl_view.handle_action(Action::MoveRight);
    assert_eq!(pl_view.pane, PlaylistsPane::Tracks);
    pl_view.track_cursor = 2;

    pl_view.handle_action(Action::CycleSort);
    assert_eq!(pl_view.track_sort, TrackSortMode::Title);
    assert_eq!(pl_view.track_cursor, 0);
    let title_order: Vec<(usize, &str)> = pl_view
        .visible_tracks()
        .into_iter()
        .map(|(i, t)| (i, t.video_id.as_str()))
        .collect();
    assert_eq!(
        title_order,
        vec![(3, "v3"), (1, "v1"), (2, "v2"), (0, "v0")]
    );

    // `dd` while sorted by Title maps cursor 0 ("Alpha Dawn") -> true original index 3!
    assert!(matches!(
        pl_view.handle_action(Action::DeleteItem),
        ViewEffect::RemovePlaylistTrack {
            ref playlist_id,
            position: 3
        } if playlist_id == "p1"
    ));

    // `J` / `K` while non-Original sorted safely guards with StatusToast & does NOT move cursor
    pl_view.track_cursor = 1;
    assert!(matches!(
        pl_view.handle_action(Action::MoveItemDown),
        ViewEffect::StatusToast(msg) if msg.contains("Original")
    ));
    assert_eq!(pl_view.track_cursor, 1);
    assert!(matches!(
        pl_view.handle_action(Action::MoveItemUp),
        ViewEffect::StatusToast(msg) if msg.contains("Original")
    ));
    assert_eq!(pl_view.track_cursor, 1);

    // Cycle to Duration: ascending with None placed last
    pl_view.handle_action(Action::CycleSort);
    assert_eq!(pl_view.track_sort, TrackSortMode::Duration);
    assert_eq!(pl_view.track_cursor, 0);
    let dur_order: Vec<(usize, &str)> = pl_view
        .visible_tracks()
        .into_iter()
        .map(|(i, t)| (i, t.video_id.as_str()))
        .collect();
    assert_eq!(dur_order, vec![(3, "v3"), (2, "v2"), (0, "v0"), (1, "v1")]);

    // Cycle back to Original
    pl_view.handle_action(Action::CycleSort);
    assert_eq!(pl_view.track_sort, TrackSortMode::Original);

    // 4. Pressing `/` in Tracks pane filters ONLY tracks (by title OR artist), keeping list filter intact
    pl_view.handle_action(Action::StartSearch);
    assert!(pl_view.is_insert_mode());
    assert!(!pl_view.filter_active);
    assert!(pl_view.track_filter_active);
    for ch in "kavinsky".chars() {
        pl_view.handle_action(Action::InputChar(ch));
    }
    pl_view.handle_action(Action::InputSubmit);
    assert!(!pl_view.is_insert_mode());
    assert_eq!(pl_view.filter_query, "night");
    assert_eq!(pl_view.track_filter_query, "kavinsky");
    let filt_tracks: Vec<(usize, &str)> = pl_view
        .visible_tracks()
        .into_iter()
        .map(|(i, t)| (i, t.video_id.as_str()))
        .collect();
    assert_eq!(filt_tracks, vec![(0, "v0"), (3, "v3")]);

    // Move to 2nd filtered match ("Alpha Dawn", orig_idx = 3) and press `dd` -> position 3!
    pl_view.handle_action(Action::MoveDown);
    assert_eq!(pl_view.track_cursor, 1);
    assert!(matches!(
        pl_view.handle_action(Action::DeleteItem),
        ViewEffect::RemovePlaylistTrack {
            ref playlist_id,
            position: 3
        } if playlist_id == "p1"
    ));

    // `J` / `K` while track-filtered guards with StatusToast & leaves track_cursor untouched
    assert!(matches!(
        pl_view.handle_action(Action::MoveItemUp),
        ViewEffect::StatusToast(msg) if msg.contains("filter")
    ));
    assert_eq!(pl_view.track_cursor, 1);

    // Combine filter + Title sort: "Alpha Dawn" (orig 3) comes before "Zebra Song" (orig 0)
    pl_view.handle_action(Action::CycleSort);
    assert_eq!(pl_view.track_sort, TrackSortMode::Title);
    assert_eq!(pl_view.track_cursor, 0);
    assert!(matches!(
        pl_view.handle_action(Action::DeleteItem),
        ViewEffect::RemovePlaylistTrack { position: 3, .. }
    ));
    assert!(matches!(
        pl_view.handle_action(Action::MoveItemDown),
        ViewEffect::StatusToast(msg) if msg.contains("filter") && msg.contains("Original")
    ));

    // Render filtered + sorted PlaylistsView at wide & narrow sizes without panic
    let snap = PlaybackSnapshot::default();
    let queue = QueueSnapshot::default();
    let lyrics = Lyrics::default();
    let cava = CavaFrame::default();
    let settings = Settings::default();
    let stats = CacheStats::default();
    let ctx = make_ctx(&snap, &queue, &lyrics, &cava, &settings, &stats);
    let mut wide = Terminal::new(TestBackend::new(110, 26)).unwrap();
    let mut narrow = Terminal::new(TestBackend::new(72, 20)).unwrap();
    wide.draw(|f| pl_view.render(f, Rect::new(0, 0, 110, 26), &ctx))
        .unwrap();
    narrow
        .draw(|f| pl_view.render(f, Rect::new(0, 0, 72, 20), &ctx))
        .unwrap();
    let wide_str = buf_str(&wide);
    assert!(wide_str.contains("Title"));
    assert!(wide_str.contains("Alpha Dawn"));
    assert!(!wide_str.contains("Midnight City"));

    // Esc layered behavior: 1st clears track filter, 2nd moves to List pane, 3rd clears list filter
    pl_view.handle_action(Action::Escape);
    assert!(pl_view.track_filter_query.is_empty());
    assert_eq!(pl_view.pane, PlaylistsPane::Tracks);
    assert_eq!(pl_view.filter_query, "night");

    pl_view.handle_action(Action::Escape);
    assert_eq!(pl_view.pane, PlaylistsPane::List);
    assert_eq!(pl_view.filter_query, "night");

    pl_view.handle_action(Action::Escape);
    assert!(pl_view.filter_query.is_empty());

    // 5. PlaylistDetailView sort parity (`o` cycles Original | Title | Duration)
    let mut detail = PlaylistDetailView::new("PL_SORT".into(), "Mix".into(), false);
    detail.set_playlist(Playlist {
        summary: PlaylistSummary {
            browse_id: "PL_SORT".into(),
            title: "Mix".into(),
            author: Some("Curator".into()),
            count: Some("3 songs".into()),
            thumbs: Vec::new(),
        },
        tracks: vec![
            sample_track_with("d0", "Zeta", "A", Some(240)),
            sample_track_with("d1", "Beta", "B", Some(90)),
            sample_track_with("d2", "Alpha", "C", None),
        ],
        description: None,
        views: None,
        duration_text: None,
        year: None,
        total: Some(3),
    });
    assert_eq!(detail.sort_mode, TrackSortMode::Original);
    detail.handle_action(Action::CycleSort);
    assert_eq!(detail.sort_mode, TrackSortMode::Title);
    assert_eq!(
        detail
            .visible_tracks()
            .into_iter()
            .map(|(_, t)| t.video_id.as_str())
            .collect::<Vec<_>>(),
        vec!["d2", "d1", "d0"]
    );
    detail.handle_action(Action::CycleSort);
    assert_eq!(detail.sort_mode, TrackSortMode::Duration);
    assert_eq!(
        detail
            .visible_tracks()
            .into_iter()
            .map(|(_, t)| t.video_id.as_str())
            .collect::<Vec<_>>(),
        vec!["d1", "d0", "d2"]
    );
    detail.handle_action(Action::CycleSort);
    assert_eq!(detail.sort_mode, TrackSortMode::Original);
}

#[test]
fn detail_views_toggle_save_never_flips_optimistically() {
    use musick_tui::views::{AlbumView, ArtistView, PlaylistDetailView};

    let mut artist = ArtistView::new("UC1".into(), "Artist".into(), CoverSize::M, false);
    assert!(matches!(
        artist.handle_action(Action::ToggleSave),
        ViewEffect::ToggleSave {
            kind: SavedCollectionKind::Artist,
            ref browse_id,
            ..
        } if browse_id == "UC1"
    ));
    assert!(!artist.saved);
    artist.saved = true;
    let _ = artist.handle_action(Action::ToggleSave);
    assert!(artist.saved);

    let mut album = AlbumView::new("MPREb1".into(), "Album".into(), false);
    assert!(matches!(
        album.handle_action(Action::ToggleSave),
        ViewEffect::ToggleSave {
            kind: SavedCollectionKind::Album,
            ref browse_id,
            ..
        } if browse_id == "MPREb1"
    ));
    assert!(!album.saved);
    album.saved = true;
    let _ = album.handle_action(Action::ToggleSave);
    assert!(album.saved);

    let mut pl = PlaylistDetailView::new("PL1".into(), "List".into(), false);
    assert!(matches!(
        pl.handle_action(Action::ToggleSave),
        ViewEffect::ToggleSave {
            kind: SavedCollectionKind::Playlist,
            ref browse_id,
            ..
        } if browse_id == "PL1"
    ));
    assert!(!pl.saved);
    pl.saved = true;
    let _ = pl.handle_action(Action::ToggleSave);
    assert!(pl.saved);
}

#[tokio::test]
async fn app_playlists_immediate_sync_and_authoritative_toggle_save_with_error_guard() {
    use std::sync::Arc;
    use tempfile::tempdir;

    use musick_audio::{
        AudioBackend, AudioProperties, AudioResult, BackendEvent, HybridStreamResolver,
        PlayerEngine, PlayerEngineDeps, RawStream, StreamBackend,
    };
    use musick_core::{
        AppPaths, AudioQuality, AuthManager, CacheManager, CacheSettings, ImageService,
        InnerTubeClient, LyricsService, MemorySecretStore, SearchService, SyncEngine, UserStore,
        build_http_client,
    };
    use musick_tui::app::{App, AppServices, NavPage};
    use musick_tui::views::{AlbumView, ArtistView, PlaylistDetailView};
    use tokio::sync::broadcast;

    struct DummyBackend {
        tx: broadcast::Sender<BackendEvent>,
    }
    impl DummyBackend {
        fn new() -> Self {
            let (tx, _) = broadcast::channel(8);
            Self { tx }
        }
    }
    #[async_trait::async_trait]
    impl AudioBackend for DummyBackend {
        async fn start(&self) -> AudioResult<()> {
            Ok(())
        }
        async fn stop(&self) -> AudioResult<()> {
            Ok(())
        }
        async fn load_url(&self, _url: &str) -> AudioResult<()> {
            Ok(())
        }
        async fn append_gapless(&self, _url: &str) -> AudioResult<()> {
            Ok(())
        }
        async fn clear_gapless_tail(&self) -> AudioResult<()> {
            Ok(())
        }
        async fn stop_track(&self) -> AudioResult<()> {
            Ok(())
        }
        async fn toggle_pause(&self) -> AudioResult<bool> {
            Ok(false)
        }
        async fn set_paused(&self, _paused: bool) -> AudioResult<()> {
            Ok(())
        }
        async fn seek(&self, _seconds: f64, _relative: bool) -> AudioResult<()> {
            Ok(())
        }
        async fn set_volume(&self, volume: u8) -> AudioResult<u8> {
            Ok(volume)
        }
        async fn apply_settings(&self, _settings: &Settings) -> AudioResult<()> {
            Ok(())
        }
        fn subscribe_events(&self) -> broadcast::Receiver<BackendEvent> {
            self.tx.subscribe()
        }
        fn properties(&self) -> AudioProperties {
            AudioProperties::default()
        }
    }

    struct DummyStream;
    #[async_trait::async_trait]
    impl StreamBackend for DummyStream {
        async fn resolve(&self, vid: &str, _q: AudioQuality) -> AudioResult<RawStream> {
            Ok(RawStream {
                url: format!("https://example.invalid/{vid}"),
                codec: "opus".into(),
                bitrate: 128,
                sample_rate: 48_000,
            })
        }
    }

    let dir = tempdir().unwrap();
    let paths = AppPaths::from_roots(
        dir.path().join("cfg"),
        dir.path().join("data"),
        dir.path().join("cache"),
        dir.path().join("rt"),
    );
    let http = build_http_client();
    let store = Arc::new(UserStore::open(&paths).unwrap());
    let cache = Arc::new(CacheManager::open(&paths, CacheSettings::default()).unwrap());
    let auth = Arc::new(AuthManager::new_anonymous(
        Arc::new(MemorySecretStore::new()),
        http.clone(),
    ));
    let catalog = Arc::new(
        InnerTubeClient::new(http.clone(), auth.clone(), cache.clone(), "ID")
            .with_base_url("http://127.0.0.1:1"),
    );
    let search = Arc::new(SearchService::new(
        catalog.clone(),
        store.clone(),
        Settings::default(),
    ));
    let lyrics = Arc::new(
        LyricsService::new(
            Some(catalog.clone()),
            cache.clone(),
            http.clone(),
            Settings::default(),
        )
        .with_ytdlp_captions(false),
    );
    let images = Arc::new(ImageService::new(http, cache.clone()));
    let sync = Arc::new(SyncEngine::new(
        store.clone(),
        catalog.clone(),
        auth.clone(),
    ));
    let resolver = Arc::new(HybridStreamResolver::with_backends(
        None,
        Arc::new(DummyStream),
        AudioQuality::Best,
        Arc::new(|| 1000.0),
    ));
    let (engine_handle, engine_join) = PlayerEngine::spawn(PlayerEngineDeps {
        backend: Arc::new(DummyBackend::new()),
        resolver,
        catalog: catalog.clone(),
        lyrics,
        store: store.clone(),
        auth: auth.clone(),
        settings: Settings::default(),
    });

    // Seed two playlists before launching App
    let p_alpha = sync.create_playlist("Alpha List", "", false).unwrap();
    sync.add_tracks_to_playlist(
        &p_alpha.id,
        &[
            sample_track_with("a0", "Zulu Track", "Artist A", Some(300)),
            sample_track_with("a1", "Echo Track", "Artist B", Some(100)),
            sample_track_with("a2", "Bravo Track", "Artist A", Some(200)),
        ],
    )
    .unwrap();
    let p_beta = sync.create_playlist("Beta List", "", false).unwrap();
    sync.add_tracks_to_playlist(
        &p_beta.id,
        &[sample_track_with(
            "b0",
            "Beta Anthem",
            "Artist C",
            Some(150),
        )],
    )
    .unwrap();

    let mut app = App::new(
        AppServices {
            paths: paths.clone(),
            store: store.clone(),
            cache,
            auth,
            catalog,
            search,
            images,
            sync: sync.clone(),
            engine: engine_handle.clone(),
            cava: None,
        },
        Settings::default(),
        CoverRenderer::halfblocks_for_test(),
    );

    // 1. Switch to Playlists tab and verify immediate right-pane track sync on navigation/filter
    //    WITHOUT calling poll_engine_and_bg()!
    app.dispatch_action(Action::SwitchTab(1));
    let first_id = app.playlists.selected_playlist().unwrap().id.clone();
    let expected_first_tracks = store.playlist_tracks(&first_id).unwrap();
    assert_eq!(app.playlists.selected_tracks, expected_first_tracks);

    // Move down to second playlist -> right pane MUST sync immediately on dispatch_action!
    app.dispatch_action(Action::MoveDown);
    let second_id = app.playlists.selected_playlist().unwrap().id.clone();
    assert_ne!(second_id, first_id);
    let expected_second_tracks = store.playlist_tracks(&second_id).unwrap();
    assert_eq!(app.playlists.selected_tracks, expected_second_tracks);

    // Filter left pane to "Alpha" -> right pane MUST sync immediately to Alpha's 3 tracks!
    app.dispatch_action(Action::StartSearch);
    for ch in "Alpha".chars() {
        app.dispatch_action(Action::InputChar(ch));
    }
    app.dispatch_action(Action::InputSubmit);
    assert_eq!(app.playlists.selected_playlist().unwrap().id, p_alpha.id);
    assert_eq!(app.playlists.selected_tracks.len(), 3);
    assert_eq!(app.playlists.selected_tracks[0].video_id, "a0");

    // 2. End-to-end filtered + sorted `dd` removes TRUE original index in SQLite and keeps track filter/sort!
    app.dispatch_action(Action::MoveRight); // Tracks pane
    app.dispatch_action(Action::StartSearch); // track filter
    for ch in "Artist A".chars() {
        app.dispatch_action(Action::InputChar(ch));
    }
    app.dispatch_action(Action::InputSubmit);
    app.dispatch_action(Action::CycleSort); // Title: Bravo (orig 2), Zulu (orig 0)
    assert_eq!(app.playlists.track_sort, TrackSortMode::Title);

    // Guarded `J` while filtered+sorted must NOT touch SQLite order and MUST set toast
    app.dispatch_action(Action::MoveItemDown);
    assert!(app.status_toast.is_some());
    assert_eq!(
        store
            .playlist_tracks(&p_alpha.id)
            .unwrap()
            .into_iter()
            .map(|t| t.video_id)
            .collect::<Vec<_>>(),
        vec!["a0", "a1", "a2"]
    );

    // `dd` on cursor 0 ("Bravo Track", orig 2) removes "a2" from SQLite, NOT "a0"!
    app.dispatch_action(Action::DeleteItem);
    assert_eq!(
        store
            .playlist_tracks(&p_alpha.id)
            .unwrap()
            .into_iter()
            .map(|t| t.video_id)
            .collect::<Vec<_>>(),
        vec!["a0", "a1"]
    );
    // Track filter & sort are preserved while reloading the same playlist!
    assert_eq!(app.playlists.track_filter_query, "Artist A");
    assert_eq!(app.playlists.track_sort, TrackSortMode::Title);
    assert_eq!(app.playlists.visible_tracks().len(), 1);
    assert_eq!(app.playlists.visible_tracks()[0].1.video_id, "a0");

    // Returning to List pane and clearing left filter switches playlist if first_id != p_alpha.id,
    // and typing a non-matching left filter immediately clears right-pane tracks!
    app.dispatch_action(Action::MoveLeft);
    app.dispatch_action(Action::StartSearch);
    for ch in "zzzzzz".chars() {
        app.dispatch_action(Action::InputChar(ch));
    }
    app.dispatch_action(Action::InputSubmit);
    assert!(app.playlists.selected_playlist().is_none());
    assert!(app.playlists.selected_tracks.is_empty());
    assert!(app.playlists.track_filter_query.is_empty());

    // 3. Authoritative `w` (ToggleSave) updates ArtistView, AlbumView, and PlaylistDetailView
    let mut wide = Terminal::new(TestBackend::new(110, 26)).unwrap();

    // ArtistView
    app.nav_stack.push(NavPage::Artist(Box::new(ArtistView::new(
        "UC_SAVE".into(),
        "Saved Artist".into(),
        CoverSize::M,
        false,
    ))));
    app.dispatch_action(Action::ToggleSave);
    assert!(matches!(app.nav_stack.last(), Some(NavPage::Artist(v)) if v.saved));
    wide.draw(|f| app.render(f)).unwrap();
    assert!(buf_str(&wide).contains("Saved ✓"));
    app.dispatch_action(Action::ToggleSave);
    assert!(matches!(app.nav_stack.last(), Some(NavPage::Artist(v)) if !v.saved));
    app.nav_stack.pop();

    // AlbumView
    app.nav_stack.push(NavPage::Album(AlbumView::new(
        "MPREb_SAVE".into(),
        "Saved Album".into(),
        false,
    )));
    app.dispatch_action(Action::ToggleSave);
    assert!(matches!(app.nav_stack.last(), Some(NavPage::Album(v)) if v.saved));
    wide.draw(|f| app.render(f)).unwrap();
    assert!(buf_str(&wide).contains("Saved ✓"));
    app.dispatch_action(Action::ToggleSave);
    assert!(matches!(app.nav_stack.last(), Some(NavPage::Album(v)) if !v.saved));
    app.nav_stack.pop();

    // PlaylistDetailView
    app.nav_stack
        .push(NavPage::PlaylistDetail(PlaylistDetailView::new(
            "PL_SAVE".into(),
            "Saved Community".into(),
            false,
        )));
    app.dispatch_action(Action::ToggleSave);
    assert!(matches!(app.nav_stack.last(), Some(NavPage::PlaylistDetail(v)) if v.saved));
    wide.draw(|f| app.render(f)).unwrap();
    assert!(buf_str(&wide).contains("Saved in Library ✓"));
    app.dispatch_action(Action::ToggleSave);
    assert!(matches!(app.nav_stack.last(), Some(NavPage::PlaylistDetail(v)) if !v.saved));

    // 4. SQLite error guard: corrupt DB in-place so toggle_save_collection fails,
    //    and verify `saved` stays authoritatively false (never flips optimistically!)
    let db_path = paths.user_db_file();
    let wal_path = db_path.with_file_name("musick.db-wal");
    let shm_path = db_path.with_file_name("musick.db-shm");
    std::fs::write(&db_path, vec![0xFFu8; 8192]).unwrap();
    if wal_path.exists() {
        std::fs::write(&wal_path, vec![0xFFu8; 8192]).unwrap();
    }
    if shm_path.exists() {
        std::fs::write(&shm_path, vec![0xFFu8; 8192]).unwrap();
    }

    app.status_toast = None;
    app.dispatch_action(Action::ToggleSave);
    assert!(
        app.status_toast
            .as_ref()
            .is_some_and(|(m, _)| m.starts_with("Save failed:")),
        "expected SQLite Save failed toast, got {:?}",
        app.status_toast.as_ref().map(|(m, _)| m.as_str())
    );
    assert!(matches!(app.nav_stack.last(), Some(NavPage::PlaylistDetail(v)) if !v.saved));
    wide.draw(|f| app.render(f)).unwrap();
    assert!(!buf_str(&wide).contains("Saved in Library ✓"));

    // Also verify ArtistView & AlbumView do NOT flip on SQLite error
    app.nav_stack.push(NavPage::Artist(Box::new(ArtistView::new(
        "UC_ERR".into(),
        "Err Artist".into(),
        CoverSize::M,
        false,
    ))));
    app.status_toast = None;
    app.dispatch_action(Action::ToggleSave);
    assert!(
        app.status_toast
            .as_ref()
            .is_some_and(|(m, _)| m.starts_with("Save failed:"))
    );
    assert!(matches!(app.nav_stack.last(), Some(NavPage::Artist(v)) if !v.saved));

    app.nav_stack.push(NavPage::Album(AlbumView::new(
        "MPREb_ERR".into(),
        "Err Album".into(),
        true,
    )));
    app.status_toast = None;
    app.dispatch_action(Action::ToggleSave);
    assert!(
        app.status_toast
            .as_ref()
            .is_some_and(|(m, _)| m.starts_with("Save failed:"))
    );
    assert!(matches!(app.nav_stack.last(), Some(NavPage::Album(v)) if v.saved));

    engine_handle.send(AppCommand::Shutdown).await;
    let _ = engine_join.await;
}
