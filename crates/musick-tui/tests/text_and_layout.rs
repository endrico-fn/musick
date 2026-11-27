#![allow(clippy::unwrap_used, clippy::expect_used)]

use musick_core::LyricLine;
use musick_tui::text::{
    Align, align_cells, cell_width, split_row_cells, trunc_cells, wrap_bilingual_lyric_lines,
    wrap_cells,
};
use musick_tui::widgets::{BRAILLE_FRAMES, BRAILLE_STATIC, BrailleSpinner};

#[test]
fn unicode_cjk_and_emoji_truncation_and_alignment_fit_exact_display_columns() {
    let cjk = "夜に駆ける YOASOBI 🎵✨";
    let truncated = trunc_cells(cjk, 10);
    assert!(cell_width(&truncated) <= 10);
    assert!(truncated.ends_with('…'));

    let aligned = align_cells(cjk, 12, Align::Left);
    assert_eq!(cell_width(&aligned), 12);

    let right_aligned = align_cells("3:45", 8, Align::Right);
    assert_eq!(cell_width(&right_aligned), 8);
    assert!(right_aligned.ends_with("3:45"));

    let (left, right, gap) =
        split_row_cells("夜に駆ける - Long Title Name", "337M · 4:21", 30, 2, 0.4);
    assert!(cell_width(&left) + gap + cell_width(&right) <= 28);

    let wrapped = wrap_cells("こんにちは世界音楽プレイヤーテスト", 10);
    assert!(wrapped.len() >= 3);
    for ln in &wrapped {
        assert!(cell_width(ln) <= 10);
    }
}

#[test]
fn bilingual_lyric_wrapping_and_braille_spinner() {
    let lines = vec![
        LyricLine {
            time: 1.0,
            text: "Around the world around the world".into(),
            translation: "Di seluruh dunia di seluruh dunia".into(),
        },
        LyricLine {
            time: 5.0,
            text: "夜に駆ける".into(),
            translation: "Berlari menuju malam".into(),
        },
    ];

    let rows = wrap_bilingual_lyric_lines(&lines, 18, true, true, 0);
    assert!(rows.iter().any(|r| r.is_translation));
    assert!(rows.iter().any(|r| r.is_spacer));
    for r in &rows {
        assert!(cell_width(&r.text) <= 18);
    }

    let mut sp = BrailleSpinner::new(false);
    assert_eq!(sp.frame(), BRAILLE_FRAMES[0]);
    sp.advance();
    assert_eq!(sp.frame(), BRAILLE_FRAMES[1]);

    let mut sp_static = BrailleSpinner::new(true);
    sp_static.advance();
    assert_eq!(sp_static.frame(), BRAILLE_STATIC);
}

mod widget_render_tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

    use musick_core::{
        AuthMode, CavaPalette, ImageProtocol, LyricLine, Lyrics, PlaybackSnapshot, PositionTick,
        Track,
    };
    use musick_tui::theme::{ACCENT, BORDER, UNICODE_ICONS, WARN_GOLD, panel_block};
    use musick_tui::widgets::{
        BrailleSpinner, CavaWidget, ColumnDef, CoverRenderer, LyricsBox, MediaListItem,
        NowPlayingBar, SCROLL_THUMB_GLYPH, SCROLL_TRACK_GLYPH, TableRow, TopBar, TrackTable,
        render_media_list,
    };

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn topbar_renders_five_tabs_and_no_cookies_badge_at_wide_and_narrow_widths() {
        for (w, h) in [(120u16, 30u16), (72u16, 20u16)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| {
                f.render_widget(
                    TopBar {
                        active_tab: 0,
                        icons: UNICODE_ICONS,
                        auth_mode: AuthMode::Anonymous,
                        syncing: false,
                        pending_sync: 0,
                        spinner: BrailleSpinner::new(false),
                        status_toast: None,
                    },
                    Rect::new(0, 0, w, 2),
                );
            })
            .unwrap();
            let rendered = buffer_text(&term);
            assert!(rendered.contains("musick"));
            assert!(rendered.contains("1 Home"));
            assert!(rendered.contains("●"));
            if w >= 100 {
                assert!(rendered.contains("2 Playlists"));
                assert!(rendered.contains("3 Library"));
                assert!(rendered.contains("4 Queue"));
                assert!(rendered.contains("5 Settings"));
                assert!(rendered.contains("no cookies"));
            }
        }
    }

    #[test]
    fn now_playing_bar_renders_gold_overdrive_and_braille_resolving() {
        let mut term = Terminal::new(TestBackend::new(100, 10)).unwrap();
        let snap = PlaybackSnapshot {
            track: Some(Track::minimal("v1", "Instant Crush")),
            playing: true,
            paused: false,
            resolving: true,
            volume: 120,
            ..PlaybackSnapshot::default()
        };
        let pos = PositionTick {
            position_secs: 65.0,
            duration_secs: 200.0,
            buffering: false,
            active_lyric_idx: None,
        };
        term.draw(|f| {
            f.render_widget(
                NowPlayingBar {
                    snapshot: &snap,
                    position: pos,
                    icons: UNICODE_ICONS,
                    spinner: BrailleSpinner::new(false),
                    focused: false,
                },
                Rect::new(0, 0, 100, 5),
            );
        })
        .unwrap();

        let rendered = buffer_text(&term);
        assert!(rendered.contains("Instant Crush"));
        assert!(rendered.contains("⠋ Resolving"));
        assert!(rendered.contains("120%"));

        // Verify gold overdrive styling exists on row 2
        let buf = term.backend().buffer();
        let has_gold = (0..100).any(|x| buf[(x, 2)].fg == WARN_GOLD);
        assert!(
            has_gold,
            "Expected gold WARN_GOLD foreground at 120% volume"
        );
    }

    #[test]
    fn lyrics_box_centers_active_stanza_with_indonesian_translation_and_cava_renders() {
        let mut term = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let lyrics = Lyrics {
            lines: vec![
                LyricLine {
                    time: 1.0,
                    text: "Line zero".into(),
                    translation: "Baris nol".into(),
                },
                LyricLine {
                    time: 5.0,
                    text: "Active chorus".into(),
                    translation: "Reff aktif".into(),
                },
                LyricLine {
                    time: 10.0,
                    text: "Next line".into(),
                    translation: "Baris berikutnya".into(),
                },
            ],
            synced: true,
            source: "LRCLIB".into(),
            translation_lang: "id".into(),
        };

        term.draw(|f| {
            f.render_widget(
                LyricsBox {
                    lyrics: &lyrics,
                    active_idx: Some(1),
                    has_track: true,
                    show_translation: true,
                    translating: false,
                    blank_between: true,
                    pad_left: 2,
                    scroll_offset: 0,
                    spinner: BrailleSpinner::new(false),
                },
                Rect::new(0, 0, 30, 10),
            );
            f.render_widget(
                CavaWidget {
                    bars: &[0.2, 0.5, 0.9, 0.4, 0.7],
                    palette: CavaPalette::Teal,
                    idle: false,
                },
                Rect::new(30, 0, 30, 10),
            );
        })
        .unwrap();

        let rendered = buffer_text(&term);
        assert!(rendered.contains("Active chorus"));
        assert!(rendered.contains("Reff aktif"));
        assert!(rendered.contains('█'));
    }

    #[test]
    fn now_playing_bar_hides_buffering_spinner_when_error_is_active() {
        let mut term = Terminal::new(TestBackend::new(100, 10)).unwrap();
        let snap = PlaybackSnapshot {
            track: Some(Track::minimal("v1", "One Last Breath")),
            playing: false,
            resolving: false,
            error: Some("loading failed".into()),
            ..PlaybackSnapshot::default()
        };
        let pos = PositionTick {
            position_secs: 0.0,
            duration_secs: 0.0,
            buffering: true,
            active_lyric_idx: None,
        };
        term.draw(|f| {
            f.render_widget(
                NowPlayingBar {
                    snapshot: &snap,
                    position: pos,
                    icons: UNICODE_ICONS,
                    spinner: BrailleSpinner::new(false),
                    focused: false,
                },
                Rect::new(0, 0, 100, 5),
            );
        })
        .unwrap();

        let rendered = buffer_text(&term);
        assert!(rendered.contains("Error: loading failed"));
        assert!(!rendered.contains("Buffering"));
    }

    #[test]
    fn cover_renderer_computes_true_1_to_1_square_panel_and_inner_rect_for_real_font_aspect() {
        for (fw, fh) in [(8u16, 16u16), (9, 20), (10, 22)] {
            let covers = CoverRenderer::with_font_size((fw, fh));
            let panel_h = 19u16;
            let panel_w = covers.square_panel_width(panel_h, 70);

            // Outer bordered panel visual pixel aspect must be within ±5% of 1:1 square
            let outer_px_w = f64::from(panel_w) * f64::from(fw);
            let outer_px_h = f64::from(panel_h) * f64::from(fh);
            let outer_ratio = outer_px_w / outer_px_h;
            assert!(
                (0.95..=1.05).contains(&outer_ratio),
                "font ({fw}x{fh}): outer {panel_w}x{panel_h} has ratio {outer_ratio:.3}"
            );

            // Inner cover rect (accounting for border + 1-cell horizontal panel padding) fills 100% of inner height
            let block = panel_block("Now Playing", None, false);
            let inner = block.inner(Rect::new(0, 0, panel_w, panel_h));
            assert_eq!(inner.width, panel_w - 4);
            assert_eq!(inner.height, panel_h - 2);
            let sq = covers.square_rect(inner);
            assert_eq!(sq.height, inner.height);
            assert!(sq.width <= inner.width);
            let inner_ratio =
                (f64::from(sq.width) * f64::from(fw)) / (f64::from(sq.height) * f64::from(fh));
            assert!(
                (0.98..=1.08).contains(&inner_ratio),
                "font ({fw}x{fh}): inner {}x{} has ratio {inner_ratio:.3}",
                sq.width,
                sq.height
            );
        }
    }

    #[test]
    fn panel_block_padding_sleek_scrollbar_gap_and_media_list_fallback() {
        let mut term = Terminal::new(TestBackend::new(60, 10)).unwrap();
        let rows: Vec<TableRow> = (0..15)
            .map(|i| TableRow::new(vec![format!("Song {i}"), "Artist".into(), "3:20".into()]))
            .collect();
        let cols = [
            ColumnDef {
                title: "Title",
                fixed_width: None,
                align: musick_tui::text::Align::Left,
            },
            ColumnDef {
                title: "Artist",
                fixed_width: Some(12),
                align: musick_tui::text::Align::Left,
            },
            ColumnDef {
                title: "Time",
                fixed_width: Some(5),
                align: musick_tui::text::Align::Right,
            },
        ];

        term.draw(|f| {
            let blk = panel_block("Discover", None, false);
            let inner = blk.inner(Rect::new(0, 0, 60, 10));
            f.render_widget(blk, Rect::new(0, 0, 60, 10));
            f.render_widget(
                TrackTable {
                    columns: &cols,
                    rows: &rows,
                    cursor: 0,
                    focused: true,
                    show_header: false,
                    loading: false,
                    loading_label: "",
                    empty_text: "",
                    icons: UNICODE_ICONS,
                    spinner: BrailleSpinner::new(false),
                },
                inner,
            );
        })
        .unwrap();

        let buf = term.backend().buffer();
        // Neutral border on focused=false
        assert_eq!(buf[(0, 0)].fg, BORDER);
        assert_ne!(buf[(0, 0)].fg, ACCENT);
        // 1-space padding inside left border (x=1 is blank) and right border (x=58 is blank)
        assert_eq!(buf[(1, 1)].symbol(), " ");
        assert_eq!(buf[(58, 1)].symbol(), " ");
        // Scrollbar at x=57 with 1-space gap at x=56 separating content from scrollbar
        assert_eq!(buf[(57, 1)].symbol(), SCROLL_THUMB_GLYPH);
        assert_eq!(buf[(56, 1)].symbol(), " ");
        let has_track = (1..9).any(|y| buf[(57, y)].symbol() == SCROLL_TRACK_GLYPH);
        assert!(has_track);

        // MediaList renders cleanly both with covers enabled and with ImageProtocol::Off fallback
        let items = vec![MediaListItem {
            title: "1. BABYDOLL".into(),
            subtitle: "Ari Abdul · BABYDOLL".into(),
            meta_top: "257M".into(),
            meta_bottom: "3:17".into(),
            cover_url: None,
            glyph: UNICODE_ICONS.song,
            is_playing: true,
            is_dim: false,
        }];
        let mut covers = CoverRenderer::halfblocks_for_test();
        for pref in [ImageProtocol::Halfblock, ImageProtocol::Off] {
            covers.set_protocol(pref);
            let mut t2 = Terminal::new(TestBackend::new(50, 6)).unwrap();
            t2.draw(|f| {
                render_media_list(
                    f,
                    Rect::new(0, 0, 50, 6),
                    &items,
                    0,
                    true,
                    false,
                    "",
                    "",
                    UNICODE_ICONS,
                    BrailleSpinner::new(false),
                    &mut covers,
                );
            })
            .unwrap();
            let rendered = buffer_text(&t2);
            assert!(rendered.contains("1. BABYDOLL"));
            assert!(rendered.contains("257M"));
            assert!(rendered.contains("3:17"));
        }
    }
}
