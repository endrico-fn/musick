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
        AuthMode, CavaPalette, LyricLine, Lyrics, PlaybackSnapshot, PositionTick, Track,
    };
    use musick_tui::theme::{UNICODE_ICONS, WARN_GOLD};
    use musick_tui::widgets::{BrailleSpinner, CavaWidget, LyricsBox, NowPlayingBar, TopBar};

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
}
