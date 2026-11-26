#![allow(clippy::unwrap_used, clippy::expect_used)]

use image::{DynamicImage, RgbImage};
use serde_json::json;

use musick_core::{
    ArtistRef, LyricLine, SearchCategory, SearchItem, Track, YoutubeTarget, active_line_index,
    crop_square, extract_youtube_target, looks_indonesian, parse_json3_captions, parse_lrc,
    parse_query_prefix, parse_vtt_captions, rank_search_items, should_skip_translation,
};

fn track(id: &str, title: &str, artist: &str, vtype: Option<&str>, plays: Option<&str>) -> Track {
    Track {
        video_id: id.into(),
        title: title.into(),
        artists: vec![ArtistRef {
            name: artist.into(),
            id: None,
        }],
        album: if vtype == Some("MUSIC_VIDEO_TYPE_ATV") {
            Some("Official Album".into())
        } else {
            None
        },
        album_id: None,
        duration: Some(200),
        thumbs: Vec::new(),
        plays: plays.map(str::to_string),
        video_type: vtype.map(str::to_string),
        set_video_id: None,
        is_available: true,
    }
}

#[test]
fn query_prefix_and_youtube_url_extraction() {
    assert_eq!(
        parse_query_prefix("a: The Weeknd", SearchCategory::All),
        (SearchCategory::Artists, "The Weeknd".into())
    );
    assert_eq!(
        parse_query_prefix("pl: Synthwave", SearchCategory::All),
        (SearchCategory::Playlists, "Synthwave".into())
    );
    assert_eq!(
        parse_query_prefix("c: Indie Chill", SearchCategory::All),
        (SearchCategory::Community, "Indie Chill".into())
    );
    assert_eq!(
        parse_query_prefix("plain query", SearchCategory::Songs),
        (SearchCategory::Songs, "plain query".into())
    );

    assert_eq!(
        extract_youtube_target("https://music.youtube.com/watch?v=dQw4w9WgXcQ"),
        Some(YoutubeTarget::Track("dQw4w9WgXcQ".into()))
    );
    assert_eq!(
        extract_youtube_target("youtu.be/abc123XYZ"),
        Some(YoutubeTarget::Track("abc123XYZ".into()))
    );
    assert_eq!(
        extract_youtube_target("https://music.youtube.com/playlist?list=PL12345"),
        Some(YoutubeTarget::Playlist("PL12345".into()))
    );
    assert_eq!(
        extract_youtube_target("https://music.youtube.com/browse/MPREb_abc"),
        Some(YoutubeTarget::Album("MPREb_abc".into()))
    );
    assert_eq!(
        extract_youtube_target("https://music.youtube.com/channel/UC123456"),
        Some(YoutubeTarget::Artist("UC123456".into()))
    );
    assert_eq!(extract_youtube_target("not a url"), None);
}

#[test]
fn ranker_prefers_official_atv_over_slowed_and_respects_semantic_lyric_order() {
    let noise = SearchItem::Track(track(
        "v_noise",
        "Blinding Lights (Slowed + Reverb)",
        "Random Uploader",
        Some("MUSIC_VIDEO_TYPE_UGC"),
        Some("500K"),
    ));
    let cover = SearchItem::Track(track(
        "v_cover",
        "Blinding Lights (Acoustic Cover)",
        "Cover Band",
        Some("MUSIC_VIDEO_TYPE_OMV"),
        Some("2M"),
    ));
    let official = SearchItem::Track(track(
        "v_official",
        "Blinding Lights",
        "The Weeknd",
        Some("MUSIC_VIDEO_TYPE_ATV"),
        Some("1.8B"),
    ));

    let ranked = rank_search_items(
        vec![noise, cover, official],
        "blinding lights",
        SearchCategory::All,
    );
    assert_eq!(ranked[0].dedup_key(), ("t", "v_official"));
    assert_eq!(ranked[2].dedup_key(), ("t", "v_noise"));

    // Semantic/lyric query where YTM returns exact song whose title does NOT contain the lyric words
    let lyric_hit_1 = SearchItem::Track(track(
        "v_hit1",
        "Bohemian Rhapsody",
        "Queen",
        Some("MUSIC_VIDEO_TYPE_ATV"),
        Some("2.1B"),
    ));
    let lyric_spam = SearchItem::Track(track(
        "v_spam",
        "Bohemian Rhapsody (Nightcore Sped Up)",
        "Spam",
        Some("MUSIC_VIDEO_TYPE_UGC"),
        Some("100K"),
    ));
    let lyric_hit_2 = SearchItem::Track(track(
        "v_hit2",
        "Somebody to Love",
        "Queen",
        Some("MUSIC_VIDEO_TYPE_ATV"),
        Some("900M"),
    ));

    let ranked_sem = rank_search_items(
        vec![lyric_spam, lyric_hit_1, lyric_hit_2],
        "scaramouche will you do the fandango",
        SearchCategory::All,
    );
    assert_eq!(ranked_sem[0].dedup_key(), ("t", "v_hit1"));
    assert_eq!(ranked_sem[2].dedup_key(), ("t", "v_spam"));
}

#[test]
fn lrc_vtt_json3_parsers_and_active_line_lookup() {
    let lrc = "\
[ar:Daft Punk]
[offset:+500]
[00:10.00][01:00.50] Chorus line
[00:05.00] First line
";
    let parsed = parse_lrc(lrc, "LRCLIB");
    assert!(parsed.synced);
    assert_eq!(parsed.lines.len(), 3);
    assert_eq!(parsed.lines[0].text, "First line");
    assert!((parsed.lines[0].time - 5.5).abs() < 1e-6);
    assert!((parsed.lines[1].time - 10.5).abs() < 1e-6);
    assert!((parsed.lines[2].time - 61.0).abs() < 1e-6);

    assert_eq!(active_line_index(&parsed, 1.0), None);
    assert_eq!(active_line_index(&parsed, 5.5), Some(0));
    assert_eq!(active_line_index(&parsed, 12.0), Some(1));
    assert_eq!(active_line_index(&parsed, 65.0), Some(2));

    let vtt = "\
WEBVTT
Kind: captions

00:00:01.500 --> 00:00:04.000
<i>Hello &amp; welcome</i>

00:00:05.000 --> 00:00:07.000
Second cue
";
    let vtt_lyr = parse_vtt_captions(vtt, "YouTube Captions");
    assert!(vtt_lyr.synced);
    assert_eq!(vtt_lyr.lines.len(), 2);
    assert_eq!(vtt_lyr.lines[0].text, "Hello & welcome");
    assert!((vtt_lyr.lines[0].time - 1.5).abs() < 1e-6);

    let json3 = json!({
        "events": [
            { "tStartMs": 2500, "segs": [{ "utf8": "Line " }, { "utf8": "one &quot;ok&quot;" }] },
            { "tStartMs": 6000, "segs": [{ "utf8": "Line two" }] }
        ]
    });
    let j3 = parse_json3_captions(&json3, "YouTube Captions");
    assert!(j3.synced);
    assert_eq!(j3.lines.len(), 2);
    assert_eq!(j3.lines[0].text, "Line one \"ok\"");
}

#[test]
fn indonesian_detection_skip_heuristic_and_square_crop() {
    let id_lines = vec![
        LyricLine::new(1.0, "Aku dan kamu selalu bersama di malam ini"),
        LyricLine::new(4.0, "Hati yang rindu tidak akan pernah lupa cinta kita"),
    ];
    let en_lines = vec![
        LyricLine::new(1.0, "Around the world around the world"),
        LyricLine::new(4.0, "Dancing through the neon midnight glow"),
    ];
    assert!(looks_indonesian(&id_lines));
    assert!(!looks_indonesian(&en_lines));
    assert!(should_skip_translation("···"));
    assert!(should_skip_translation("[Instrumental]"));
    assert!(!should_skip_translation("Hello world"));

    let rect_img = DynamicImage::ImageRgb8(RgbImage::new(320, 180));
    let sq = crop_square(rect_img);
    assert_eq!(sq.width(), 180);
    assert_eq!(sq.height(), 180);
}
