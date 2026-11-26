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
