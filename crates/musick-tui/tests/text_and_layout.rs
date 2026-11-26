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

    let (left, right, gap) = split_row_cells("夜に駆ける - Long Title Name", "337M · 4:21", 30, 2, 0.4);
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
