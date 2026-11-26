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
    PlaylistsView, QueueView, RenderCtx, ViewEffect,
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

    engine_handle.send(AppCommand::Shutdown).await;
    let _ = engine_join.await;
}
