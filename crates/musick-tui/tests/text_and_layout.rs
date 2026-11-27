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
        AuthMode, ImageProtocol, LyricLine, Lyrics, PlaybackSnapshot, PositionTick, ThemeAccent,
        Track,
    };
    use musick_tui::theme::{
        ACCENT, BORDER, UNICODE_ICONS, WARN_GOLD, cover_panel_block, panel_block,
    };
    use musick_tui::widgets::cover::center_square_canvas;
    use musick_tui::widgets::{
        BrailleSpinner, CavaWidget, ColumnDef, CoverRenderer, LyricsBox, MediaListItem,
        NowPlayingBar, RootTab, SCROLL_THUMB_GLYPH, SCROLL_TRACK_GLYPH, TableRow, TopBar,
        TrackTable, render_media_list,
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
                        active_tab: RootTab::Player,
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
            assert!(rendered.contains("●"));
            if w >= 100 {
                assert!(rendered.contains("1 Player"));
                assert!(rendered.contains("2 Playlists"));
                assert!(rendered.contains("3 Library"));
                assert!(rendered.contains("4 Queue"));
                assert!(rendered.contains("5 Settings"));
                assert!(rendered.contains("no cookies"));
            } else {
                assert!(rendered.contains("1 Play"));
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
        assert!(rendered.contains("Now Playing"));
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
                    loading: false,
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
                    accent: ThemeAccent::Teal,
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
        for (fw, fh) in [(7u16, 17u16), (8, 16), (9, 20), (10, 22)] {
            let covers = CoverRenderer::with_font_size((fw, fh));
            let panel_h = 17u16;
            let panel_w = covers.square_panel_width(panel_h, 70);

            // Border stroke-to-stroke pixel aspect `(W - 1)*fw / (H - 1)*fh` must be within ±2% of 1:1
            let stroke_w = f64::from(panel_w - 1) * f64::from(fw);
            let stroke_h = f64::from(panel_h - 1) * f64::from(fh);
            let border_ratio = stroke_w / stroke_h;
            assert!(
                (0.98..=1.02).contains(&border_ratio),
                "font ({fw}x{fh}): border {panel_w}x{panel_h} has stroke ratio {border_ratio:.4}"
            );

            // cover_panel_block has zero extra horizontal padding so inner canvas centers exact 1:1 cover with symmetric 4-side gaps
            let block = cover_panel_block("Cover", None, false);
            let inner = block.inner(Rect::new(0, 0, panel_w, panel_h));
            assert_eq!(inner.width, panel_w - 2);
            assert_eq!(inner.height, panel_h - 2);

            let canvas_w = u32::from(inner.width) * u32::from(fw);
            let canvas_h = u32::from(inner.height) * u32::from(fh);
            let side = canvas_w.min(canvas_h);
            let ox = (canvas_w - side) / 2;
            let right_slack = canvas_w - side - ox;
            let left_gap = f64::from(fw) / 2.0 + ox as f64;
            let right_gap = f64::from(fw) / 2.0 + right_slack as f64;
            let vert_gap = f64::from(fh) / 2.0;
            assert!(
                (left_gap - right_gap).abs() <= 1.0,
                "font ({fw}x{fh}): left_gap={left_gap} right_gap={right_gap}"
            );
            assert!(
                (left_gap - vert_gap).abs() <= f64::from(fw) / 2.0 + 1.0,
                "font ({fw}x{fh}): left_gap={left_gap} vert_gap={vert_gap}"
            );

            // Verify center_square_canvas outputs exact canvas_w x canvas_h with centered 1:1 opaque pixels
            let src = image::DynamicImage::ImageRgba8(image::ImageBuffer::from_pixel(
                100,
                100,
                image::Rgba([255, 0, 0, 255]),
            ));
            let centered = center_square_canvas(&src, canvas_w, canvas_h).to_rgba8();
            assert_eq!(centered.width(), canvas_w);
            assert_eq!(centered.height(), canvas_h);
            let mut min_x = canvas_w;
            let mut max_x = 0u32;
            let mut min_y = canvas_h;
            let mut max_y = 0u32;
            for (x, y, p) in centered.enumerate_pixels() {
                if p[3] > 0 {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
            assert_eq!(max_x - min_x + 1, side);
            assert_eq!(max_y - min_y + 1, side);
            assert_eq!(min_x, ox);
            assert_eq!(canvas_w - 1 - max_x, right_slack);
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
                    scroll: None,
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
                    None,
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

    #[test]
    fn scroll_state_text_input_lyrics_loading_spinner_and_detail_hero_collapse() {
        use musick_tui::keymap::Action;
        use musick_tui::text::cell_width;
        use musick_tui::views::detail_hero::split_detail_hero;
        use musick_tui::widgets::{ScrollState, TextInput};

        // 1. ScrollState maintains smooth scrolloff hysteresis in both directions
        let scroll = ScrollState::new();
        assert_eq!(scroll.ensure_visible(0, 10, 30, 2), 0);
        assert_eq!(scroll.ensure_visible(7, 10, 30, 2), 0);
        // cursor=8 enters bottom 2-row scrolloff margin -> scrolls viewport down to 1
        assert_eq!(scroll.ensure_visible(8, 10, 30, 2), 1);
        assert_eq!(scroll.ensure_visible(12, 10, 30, 2), 5);
        // moving slightly back up stays sticky until entering top 2-row margin
        assert_eq!(scroll.ensure_visible(8, 10, 30, 2), 5);
        assert_eq!(scroll.ensure_visible(6, 10, 30, 2), 4);
        scroll.reset();
        assert_eq!(scroll.offset(), 0);

        // 2. TextInput supports mid-string cursor editing, Ctrl+W, Home/End, Delete, and CJK viewport slicing
        let mut ti = TextInput::from_text("hello world");
        assert_eq!(ti.cursor(), 11);
        ti.handle_action(&Action::InputLeft);
        ti.handle_action(&Action::InputLeft);
        ti.handle_action(&Action::InputChar('X'));
        assert_eq!(ti.as_str(), "hello worXld");
        ti.handle_action(&Action::InputDelete);
        assert_eq!(ti.as_str(), "hello worXd");
        ti.handle_action(&Action::InputEnd);
        ti.handle_action(&Action::InputDeleteWord);
        assert_eq!(ti.as_str(), "hello ");
        ti.handle_action(&Action::InputHome);
        ti.handle_action(&Action::InputChar('>'));
        assert_eq!(ti.as_str(), ">hello ");
        ti.set_text("夜に駆ける YOASOBI long query string");
        let (before, cur, after) = ti.visible_slice(12);
        assert!(cell_width(&before) + cell_width(&cur) + cell_width(&after) <= 12);

        // 3. LyricsBox renders Braille spinner while loading lyrics
        let mut term = Terminal::new(TestBackend::new(40, 6)).unwrap();
        let empty = Lyrics::default();
        term.draw(|f| {
            f.render_widget(
                LyricsBox {
                    lyrics: &empty,
                    active_idx: None,
                    has_track: true,
                    loading: true,
                    show_translation: false,
                    translating: false,
                    blank_between: false,
                    pad_left: 0,
                    scroll_offset: 0,
                    spinner: BrailleSpinner::new(false),
                },
                Rect::new(0, 0, 40, 6),
            );
        })
        .unwrap();
        assert!(buffer_text(&term).contains("⠋ Loading lyrics"));

        // 4. split_detail_hero collapses cover panel completely when ImageProtocol::Off
        let mut covers = CoverRenderer::halfblocks_for_test();
        let area = Rect::new(0, 0, 100, 26);
        let with_cover = split_detail_hero(area, &covers);
        assert!(with_cover.cover_area.is_some());
        assert!(with_cover.info_area.width < area.width);

        covers.set_protocol(ImageProtocol::Off);
        let without_cover = split_detail_hero(area, &covers);
        assert!(without_cover.cover_area.is_none());
        assert_eq!(without_cover.info_area.width, area.width);
    }

    #[test]
    fn media_list_has_one_blank_row_gap_with_preview_and_compact_single_row_when_off() {
        let items = vec![
            MediaListItem {
                title: "First Track".into(),
                subtitle: "Artist A".into(),
                meta_top: "12M".into(),
                meta_bottom: "3:10".into(),
                cover_url: None,
                glyph: UNICODE_ICONS.song,
                is_playing: false,
                is_dim: false,
            },
            MediaListItem {
                title: "Second Track".into(),
                subtitle: "Artist B".into(),
                meta_top: "8M".into(),
                meta_bottom: "2:45".into(),
                cover_url: None,
                glyph: UNICODE_ICONS.song,
                is_playing: false,
                is_dim: false,
            },
        ];

        // 1. Preview enabled: 2-row item + 1 blank separator row (row 2 is blank, item 2 starts at row 3)
        let mut covers = CoverRenderer::halfblocks_for_test();
        let mut term = Terminal::new(TestBackend::new(60, 6)).unwrap();
        term.draw(|f| {
            render_media_list(
                f,
                Rect::new(0, 0, 60, 6),
                &items,
                0,
                None,
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
        let rendered = buffer_text(&term);
        let lines: Vec<&str> = rendered.lines().collect();
        assert!(lines[0].contains("First Track"));
        assert!(lines[1].contains("Artist A"));
        assert!(
            lines[2].trim().is_empty(),
            "Expected 1 blank gap row between preview items, got: {:?}",
            lines[2]
        );
        assert!(lines[3].contains("Second Track"));
        assert!(lines[4].contains("Artist B"));

        // 2. Preview disabled (Off): compact 1-row mode (item 0 on row 0, item 1 on row 1)
        covers.set_protocol(ImageProtocol::Off);
        let mut term_compact = Terminal::new(TestBackend::new(60, 6)).unwrap();
        term_compact
            .draw(|f| {
                render_media_list(
                    f,
                    Rect::new(0, 0, 60, 6),
                    &items,
                    0,
                    None,
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
        let compact_text = buffer_text(&term_compact);
        let compact_lines: Vec<&str> = compact_text.lines().collect();
        assert!(compact_lines[0].contains("First Track"));
        assert!(compact_lines[0].contains("3:10"));
        assert!(compact_lines[1].contains("Second Track"));
        assert!(compact_lines[1].contains("2:45"));
    }

    #[test]
    fn global_magenta_theme_rethemes_panels_and_cava() {
        use musick_tui::theme::{MAGENTA_THEME, TEAL_THEME, set_active_accent};

        set_active_accent(ThemeAccent::Magenta);
        let mut term = Terminal::new(TestBackend::new(40, 8)).unwrap();
        term.draw(|f| {
            f.render_widget(panel_block("Active", None, true), Rect::new(0, 0, 20, 8));
            f.render_widget(
                CavaWidget {
                    bars: &[0.9, 0.8, 0.7, 0.9],
                    accent: ThemeAccent::Magenta,
                    idle: false,
                },
                Rect::new(20, 0, 20, 8),
            );
        })
        .unwrap();
        let buf = term.backend().buffer();
        assert_eq!(buf[(0, 0)].fg, MAGENTA_THEME.accent);
        assert_ne!(buf[(0, 0)].fg, TEAL_THEME.accent);
        // Verify Cava bars rendered magenta hues (r > g)
        let has_magenta_bar = (20..40).any(|x| {
            (0..8).any(|y| match buf[(x, y)].fg {
                ratatui::style::Color::Rgb(r, g, b) => r > g && b > g && buf[(x, y)].symbol() != " ",
                _ => false,
            })
        });
        assert!(has_magenta_bar);
        set_active_accent(ThemeAccent::Teal);
    }
}
