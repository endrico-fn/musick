use std::collections::HashSet;
use std::sync::{Arc, LazyLock, RwLock};

use rapidfuzz::fuzz;
use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::config::Settings;
use crate::domain::{
    SearchCategory, SearchItem, Track, clean_media_title, search_item_key,
};
use crate::error::CoreResult;
use crate::innertube::CatalogApi;
use crate::storage::UserStore;

const PREFIX_MAP: &[(&str, SearchCategory)] = &[
    ("community:", SearchCategory::Community),
    ("playlist:", SearchCategory::Playlists),
    ("artist:", SearchCategory::Artists),
    ("album:", SearchCategory::Albums),
    ("video:", SearchCategory::Videos),
    ("comm:", SearchCategory::Community),
    ("song:", SearchCategory::Songs),
    ("vid:", SearchCategory::Videos),
    ("al:", SearchCategory::Albums),
    ("pl:", SearchCategory::Playlists),
    ("a:", SearchCategory::Artists),
    ("c:", SearchCategory::Community),
    ("v:", SearchCategory::Videos),
    ("s:", SearchCategory::Songs),
];

const STRONG_NOISE_TOKENS: &[&str] = &[
    "slowed",
    "reverb",
    "sped up",
    "speed up",
    "nightcore",
    "tiktok",
    "mashup",
    "8d audio",
    "bass boosted",
    "karaoke",
    "instrumental",
    "1 hour",
    "reaction",
];

const MILD_VARIANT_TOKENS: &[&str] = &[
    "remix",
    "cover",
    "live",
    "acoustic",
    "lyric video",
    "official video",
    "music video",
    "lyrics",
];

const CONTENT_STOPWORDS: &[&str] = &[
    "i", "m", "im", "s", "t", "re", "ve", "ll", "d", "a", "an", "the", "you", "me", "my", "your",
    "to", "in", "on", "of", "is", "it", "and", "or", "for", "that", "this",
];

static PAREN_WITHOUT_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)[\(\[][^\)\]]*\bwithout\b[^\)\]]*[\)\]]").ok());

static SCHEMELESS_YT_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:(?:www|m|music)\.)?(?:youtube\.com|youtu\.be)/").ok());

static NON_WORD_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"[^\w\s]").ok());

static STRONG_NOISE_PATTERNS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    STRONG_NOISE_TOKENS
        .iter()
        .filter_map(|&tok| {
            Regex::new(&format!(r"(?i)\b{}\b", regex::escape(tok)))
                .ok()
                .map(|re| (tok, re))
        })
        .collect()
});

static MILD_VARIANT_PATTERNS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    MILD_VARIANT_TOKENS
        .iter()
        .filter_map(|&tok| {
            Regex::new(&format!(r"(?i)\b{}\b", regex::escape(tok)))
                .ok()
                .map(|re| (tok, re))
        })
        .collect()
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YoutubeTarget {
    Track(String),
    Playlist(String),
    Album(String),
    Artist(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub text: String,
    pub kind_hint: String,
    pub from_history: bool,
}

// ── RapidFuzz Helpers (0.0..=100.0 scale matching Python rapidfuzz) ─────────

fn ratio_100(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    fuzz::ratio(a.chars(), b.chars()) * 100.0
}

pub fn token_sort_ratio(a: &str, b: &str) -> f64 {
    let mut ta: Vec<&str> = a.split_whitespace().collect();
    let mut tb: Vec<&str> = b.split_whitespace().collect();
    ta.sort_unstable();
    tb.sort_unstable();
    ratio_100(&ta.join(" "), &tb.join(" "))
}

pub fn token_set_ratio(a: &str, b: &str) -> f64 {
    let set_a: HashSet<&str> = a.split_whitespace().collect();
    let set_b: HashSet<&str> = b.split_whitespace().collect();
    if set_a.is_empty() || set_b.is_empty() {
        return 0.0;
    }
    let mut inter: Vec<&str> = set_a.intersection(&set_b).copied().collect();
    let mut diff_a: Vec<&str> = set_a.difference(&set_b).copied().collect();
    let mut diff_b: Vec<&str> = set_b.difference(&set_a).copied().collect();
    inter.sort_unstable();
    diff_a.sort_unstable();
    diff_b.sort_unstable();

    let inter_s = inter.join(" ");
    let comb_a = [inter_s.as_str(), &diff_a.join(" ")]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let comb_b = [inter_s.as_str(), &diff_b.join(" ")]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    let mut best = ratio_100(&comb_a, &comb_b);
    if !inter_s.is_empty() {
        best = best
            .max(ratio_100(&inter_s, &comb_a))
            .max(ratio_100(&inter_s, &comb_b));
    }
    best
}

pub fn wratio(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let base = ratio_100(a, b);
    let len_a = a.chars().count();
    let len_b = b.chars().count();
    let min_len = len_a.min(len_b).max(1) as f64;
    let max_len = len_a.max(len_b) as f64;
    let len_ratio = max_len / min_len;

    let ts = token_sort_ratio(a, b) * 0.95;
    let tset = token_set_ratio(a, b) * 0.95;

    let mut best = base.max(ts).max(tset);
    if len_ratio >= 1.5 && (a.contains(b) || b.contains(a)) {
        let partial_scale = if len_ratio > 8.0 { 0.6 } else { 0.9 };
        best = best.max(100.0 * partial_scale);
    }
    best
}

// ── Text Normalization & URL/Prefix Parsing ─────────────────────────────────

pub fn norm_text(text: &str) -> String {
    let folded: String = text
        .to_lowercase()
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .collect();
    let cleaned = match NON_WORD_RE.as_ref() {
        Some(re) => re.replace_all(&folded, " ").into_owned(),
        None => folded,
    };
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_combining_mark(c: char) -> bool {
    matches!(
        c as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F
    )
}

fn clean_song_title(title: &str) -> String {
    let stripped = clean_media_title(title, true);
    let n = norm_text(&stripped);
    if n.is_empty() { norm_text(title) } else { n }
}

pub fn parse_query_prefix(
    raw: &str,
    default_kind: SearchCategory,
) -> (SearchCategory, String) {
    let stripped = raw.trim_start();
    let low = stripped.to_ascii_lowercase();
    for &(prefix, cat) in PREFIX_MAP {
        if low.starts_with(prefix) {
            return (cat, stripped[prefix.len()..].trim().to_string());
        }
    }
    (default_kind, raw.trim().to_string())
}

pub fn extract_youtube_target(raw: &str) -> Option<YoutubeTarget> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains(' ') {
        return None;
    }

    let candidate = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else if SCHEMELESS_YT_RE
        .as_ref()
        .is_some_and(|re| re.is_match(trimmed))
    {
        format!("https://{trimmed}")
    } else {
        return None;
    };

    let parsed = url::Url::parse(&candidate).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if !host.contains("youtube.com") && !host.contains("youtu.be") {
        return None;
    }

    if host.contains("youtu.be") {
        let vid = parsed.path().trim_matches('/').split('/').next()?;
        if !vid.is_empty() {
            return Some(YoutubeTarget::Track(vid.to_string()));
        }
    }

    for (k, v) in parsed.query_pairs() {
        if k == "v" && !v.is_empty() {
            return Some(YoutubeTarget::Track(v.into_owned()));
        }
    }
    for (k, v) in parsed.query_pairs() {
        if k == "list" && !v.is_empty() {
            return Some(YoutubeTarget::Playlist(v.into_owned()));
        }
    }

    let parts: Vec<&str> = parsed.path().split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 && matches!(parts[0], "shorts" | "live") {
        return Some(YoutubeTarget::Track(parts[1].to_string()));
    }
    if parts.len() >= 2 && matches!(parts[0], "channel" | "browse") {
        let bid = parts[1];
        if bid.starts_with("MPRE") {
            return Some(YoutubeTarget::Album(bid.to_string()));
        }
        if bid.starts_with("UC") {
            return Some(YoutubeTarget::Artist(bid.to_string()));
        }
        if bid.starts_with("VL")
            || bid.starts_with("PL")
            || bid.starts_with("OLAK")
            || bid.starts_with("RD")
        {
            return Some(YoutubeTarget::Playlist(bid.to_string()));
        }
    }
    None
}

// ── Play-Count Bonus & Semantic/Lyric Ranking ───────────────────────────────

fn parse_play_count(plays: Option<&str>) -> f64 {
    let Some(raw) = plays else {
        return 0.0;
    };
    let mut s = raw
        .trim()
        .to_ascii_lowercase()
        .replace([',', ' '], "");
    let mut mult = 1.0;
    for (suffix, factor) in [
        ("rb", 1e3),
        ("jt", 1e6),
        ("b", 1e9),
        ("m", 1e6),
        ("k", 1e3),
    ] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            s = stripped.to_string();
            mult = factor;
            break;
        }
    }
    s.parse::<f64>().map(|v| (v * mult).max(0.0)).unwrap_or(0.0)
}

fn play_bonus(item: &SearchItem) -> f64 {
    let plays = match item {
        SearchItem::Track(t) => t.plays.as_deref(),
        SearchItem::Artist(a) => a.subtitle.as_deref(),
        _ => None,
    };
    let count = parse_play_count(plays);
    if count < 100.0 {
        0.0
    } else {
        count.log10().min(10.0)
    }
}

fn meaningful_tokens(q_norm: &str) -> HashSet<&str> {
    let toks: Vec<&str> = q_norm.split_whitespace().collect();
    if toks.len() < 3 {
        return toks.into_iter().collect();
    }
    let content: HashSet<&str> = toks
        .iter()
        .copied()
        .filter(|t| !CONTENT_STOPWORDS.contains(t))
        .collect();
    if content.is_empty() {
        toks.into_iter().collect()
    } else {
        content
    }
}

fn contains_word_token(haystack: &str, token: &str) -> bool {
    if token.contains(' ') {
        haystack.contains(token)
    } else {
        haystack.split_whitespace().any(|w| w == token)
    }
}

fn has_requested_variant(q_norm: &str) -> bool {
    STRONG_NOISE_TOKENS
        .iter()
        .chain(MILD_VARIANT_TOKENS.iter())
        .any(|&tok| contains_word_token(q_norm, tok))
}

fn has_strong_noise(raw_lower: &str, q_norm: &str) -> bool {
    let pat_hit = STRONG_NOISE_PATTERNS
        .iter()
        .any(|(tok, pat)| pat.is_match(raw_lower) && !contains_word_token(q_norm, tok));
    let without_hit = PAREN_WITHOUT_RE
        .as_ref()
        .is_some_and(|re| re.is_match(raw_lower))
        && !contains_word_token(q_norm, "without");
    pat_hit || without_hit
}

fn mild_variant_hits(raw_lower: &str, q_norm: &str) -> usize {
    MILD_VARIANT_PATTERNS
        .iter()
        .filter(|(tok, pat)| pat.is_match(raw_lower) && !contains_word_token(q_norm, tok))
        .count()
}

fn item_lexical_overlap(item: &SearchItem, q_norm: &str, q_tokens: &HashSet<&str>) -> f64 {
    if q_norm.is_empty() || q_tokens.is_empty() {
        return 0.0;
    }
    match item {
        SearchItem::Artist(a) => {
            let a_norm = norm_text(&a.name);
            if a_norm == q_norm || a_norm.contains(q_norm) {
                return 1.0;
            }
            let toks: HashSet<&str> = a_norm.split_whitespace().collect();
            if toks.is_empty() {
                0.0
            } else {
                q_tokens.intersection(&toks).count() as f64 / q_tokens.len() as f64
            }
        }
        SearchItem::Album(al) => {
            let al_norm = norm_text(&al.title);
            if al_norm == q_norm || al_norm.contains(q_norm) {
                return 1.0;
            }
            let art_norm = norm_text(
                &al.artists
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            let combo = format!("{art_norm} {al_norm}");
            let toks: HashSet<&str> = combo.split_whitespace().collect();
            if toks.is_empty() {
                0.0
            } else {
                q_tokens.intersection(&toks).count() as f64 / q_tokens.len() as f64
            }
        }
        SearchItem::Playlist(pl) => {
            let p_norm = norm_text(&pl.title);
            let toks: HashSet<&str> = p_norm.split_whitespace().collect();
            if toks.is_empty() {
                0.0
            } else {
                q_tokens.intersection(&toks).count() as f64 / q_tokens.len() as f64
            }
        }
        SearchItem::Track(tr) => {
            let clean_title = clean_song_title(&tr.title);
            let raw_norm = norm_text(&tr.title);
            let art_norm = if tr.artists.is_empty() {
                String::new()
            } else {
                norm_text(&tr.artist_names())
            };
            if clean_title == q_norm
                || clean_title.contains(q_norm)
                || (!art_norm.is_empty() && art_norm == q_norm)
            {
                return 1.0;
            }
            let combo = format!("{art_norm} {raw_norm}");
            let toks: HashSet<&str> = combo.split_whitespace().collect();
            if toks.is_empty() {
                0.0
            } else {
                q_tokens.intersection(&toks).count() as f64 / q_tokens.len() as f64
            }
        }
    }
}

pub fn is_semantic_result_set(items: &[SearchItem], q_norm: &str) -> bool {
    if has_requested_variant(q_norm) {
        return false;
    }
    let q_tokens = meaningful_tokens(q_norm);
    if q_tokens.len() < 2 {
        return false;
    }
    let mut clean_head = Vec::with_capacity(4);
    for it in items {
        if let SearchItem::Track(t) = it
            && has_strong_noise(&t.title.to_lowercase(), q_norm)
        {
            continue;
        }
        clean_head.push(it);
        if clean_head.len() >= 4 {
            break;
        }
    }
    if clean_head.is_empty() {
        return false;
    }
    let max_overlap = clean_head
        .iter()
        .take(3)
        .map(|it| item_lexical_overlap(it, q_norm, &q_tokens))
        .fold(0.0_f64, f64::max);
    max_overlap < 0.5
}

pub fn score_search_item(
    item: &SearchItem,
    query: &str,
    idx: usize,
    kind: SearchCategory,
    semantic: bool,
) -> f64 {
    let q_norm = norm_text(query);
    if q_norm.is_empty() {
        return -(idx as f64);
    }

    if semantic {
        let mut score = (90.0 - (idx as f64) * 3.0).max(0.0) + play_bonus(item);
        match item {
            SearchItem::Track(tr) => {
                let raw_lower = tr.title.to_lowercase();
                if tr.video_type.as_deref() == Some("MUSIC_VIDEO_TYPE_ATV") {
                    score += 10.0;
                } else if tr.album.is_some() {
                    score += 4.0;
                } else if matches!(
                    tr.video_type.as_deref(),
                    Some("MUSIC_VIDEO_TYPE_OMV" | "MUSIC_VIDEO_TYPE_VIDEO")
                ) {
                    score -= 4.0;
                }

                if has_strong_noise(&raw_lower, &q_norm) {
                    score -= 100.0;
                } else {
                    match mild_variant_hits(&raw_lower, &q_norm) {
                        0 => {}
                        1 => score -= 28.0,
                        _ => score -= 42.0,
                    }
                }
            }
            SearchItem::Artist(_) => score += 4.0,
            SearchItem::Album(_) => score -= 8.0,
            SearchItem::Playlist(_) => score -= 6.0,
        }
        return score;
    }

    let mut score = (18.0 - (idx as f64) * 0.55).max(0.0) + play_bonus(item);
    match item {
        SearchItem::Artist(ar) => {
            let a_norm = norm_text(&ar.name);
            let ratio = wratio(&q_norm, &a_norm);
            if a_norm == q_norm {
                score += 88.0;
            } else if a_norm.starts_with(&q_norm) || a_norm.contains(&q_norm) {
                score += 46.0;
            } else if ratio >= 75.0 {
                score += ratio * 0.42;
            } else if kind == SearchCategory::All {
                score += 10.0;
            }
            score
        }
        SearchItem::Album(al) => {
            let al_norm = norm_text(&al.title);
            let art_norm = norm_text(
                &al.artists
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            let combo = format!("{art_norm} {al_norm}").trim().to_string();
            if al_norm == q_norm {
                score += 50.0;
            } else {
                let r = wratio(&q_norm, &al_norm).max(token_set_ratio(&q_norm, &combo));
                if r >= 70.0 {
                    score += r * 0.34;
                }
            }
            if kind == SearchCategory::All {
                score -= 6.0;
            }
            score
        }
        SearchItem::Playlist(pl) => {
            let pl_norm = norm_text(&pl.title);
            let r = wratio(&q_norm, &pl_norm);
            if pl_norm == q_norm {
                score += 36.0;
            } else if r >= 70.0 {
                score += r * 0.28;
            }
            if kind == SearchCategory::All {
                score -= 6.0;
            }
            score
        }
        SearchItem::Track(tr) => {
            let raw_lower = tr.title.to_lowercase();
            let clean_title = clean_song_title(&tr.title);
            let art_norm = if tr.artists.is_empty() {
                String::new()
            } else {
                norm_text(&tr.artist_names())
            };
            let combo = format!("{art_norm} {clean_title}").trim().to_string();

            let q_tokens: HashSet<&str> = q_norm.split_whitespace().collect();
            let title_tokens: HashSet<&str> = clean_title.split_whitespace().collect();
            let combo_tokens: HashSet<&str> = combo.split_whitespace().collect();

            let mut matched = false;
            if clean_title == q_norm {
                score += 55.0;
                matched = true;
            } else if clean_title.starts_with(&q_norm)
                || (!q_tokens.is_empty() && q_tokens.is_subset(&title_tokens))
            {
                score += 38.0;
                matched = true;
            } else if !art_norm.is_empty() && q_norm == art_norm {
                score += 14.0;
                matched = true;
            } else if !combo_tokens.is_empty() && q_tokens.is_subset(&combo_tokens) {
                score += 34.0;
                matched = true;
            } else if clean_title.contains(&q_norm) {
                score += 18.0;
                matched = true;
            } else {
                let r = token_sort_ratio(&q_norm, &clean_title)
                    .max(token_sort_ratio(&q_norm, &combo));
                if r >= 60.0 {
                    score += r * 0.35;
                    matched = true;
                }
            }

            if matched {
                if tr.video_type.as_deref() == Some("MUSIC_VIDEO_TYPE_ATV") {
                    score += 30.0;
                } else if tr.album.is_some() {
                    score += 22.0;
                } else if tr.video_type.as_deref() == Some("MUSIC_VIDEO_TYPE_OMV") {
                    score += 4.0;
                } else {
                    score -= 10.0;
                }
            }

            for (tok, pat) in STRONG_NOISE_PATTERNS
                .iter()
                .chain(MILD_VARIANT_PATTERNS.iter())
            {
                if contains_word_token(&q_norm, tok) && pat.is_match(&raw_lower) {
                    score += 26.0;
                }
            }

            if has_strong_noise(&raw_lower, &q_norm) {
                score -= 60.0;
            } else {
                match mild_variant_hits(&raw_lower, &q_norm) {
                    0 => {}
                    1 => score -= 26.0,
                    _ => score -= 40.0,
                }
            }
            score
        }
    }
}

pub fn rank_search_items(
    items: Vec<SearchItem>,
    query: &str,
    kind: SearchCategory,
) -> Vec<SearchItem> {
    let mut deduped = Vec::with_capacity(items.len());
    let mut seen: HashSet<(&'static str, String)> = HashSet::new();
    for it in items {
        let (k, id) = search_item_key(&it);
        if seen.insert((k, id.to_string())) {
            deduped.push(it);
        }
    }

    if !matches!(kind, SearchCategory::All | SearchCategory::Songs) || query.trim().is_empty() {
        return deduped;
    }

    let q_norm = norm_text(query);
    let semantic = is_semantic_result_set(&deduped, &q_norm);
    let mut indexed: Vec<(usize, f64, SearchItem)> = deduped
        .into_iter()
        .enumerate()
        .map(|(i, item)| {
            let s = score_search_item(&item, query, i, kind, semantic);
            (i, s, item)
        })
        .collect();
    indexed.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    indexed.into_iter().map(|(_, _, item)| item).collect()
}

// ── SearchService ───────────────────────────────────────────────────────────

pub struct SearchService {
    catalog: Arc<dyn CatalogApi>,
    store: Arc<UserStore>,
    settings: RwLock<Settings>,
}

impl SearchService {
    pub fn new(catalog: Arc<dyn CatalogApi>, store: Arc<UserStore>, settings: Settings) -> Self {
        Self {
            catalog,
            store,
            settings: RwLock::new(settings),
        }
    }

    pub fn update_settings(&self, settings: Settings) {
        if let Ok(mut guard) = self.settings.write() {
            *guard = settings;
        }
    }

    fn current_settings(&self) -> Settings {
        self.settings
            .read()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub async fn search(
        &self,
        category: SearchCategory,
        raw_query: &str,
        limit: usize,
    ) -> CoreResult<Vec<SearchItem>> {
        let settings = self.current_settings();
        let trimmed = raw_query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        if settings.privacy.save_history {
            let _ = self.store.record_search_query(trimmed);
        }

        if let Some(target) = extract_youtube_target(trimmed) {
            return match target {
                YoutubeTarget::Track(vid) => {
                    if let Ok(batch) = self.catalog.radio(Some(&vid), None, None, 5).await {
                        let mut out = Vec::new();
                        if let Some(exact) =
                            batch.tracks.iter().find(|t| t.video_id == vid).cloned()
                        {
                            out.push(SearchItem::Track(exact));
                            for t in batch.tracks {
                                if t.video_id != vid {
                                    out.push(SearchItem::Track(t));
                                }
                            }
                            return Ok(out);
                        }
                        if !batch.tracks.is_empty() {
                            out.push(SearchItem::Track(Track::minimal(
                                vid.clone(),
                                format!("YouTube Track ({vid})"),
                            )));
                            out.extend(batch.tracks.into_iter().map(SearchItem::Track));
                            return Ok(out);
                        }
                    }
                    Ok(vec![SearchItem::Track(Track::minimal(
                        vid.clone(),
                        format!("YouTube Track ({vid})"),
                    ))])
                }
                YoutubeTarget::Playlist(pid) => {
                    let pl = self.catalog.playlist(&pid, 50).await?;
                    let mut out = vec![SearchItem::Playlist(pl.summary)];
                    out.extend(pl.tracks.into_iter().take(15).map(SearchItem::Track));
                    Ok(out)
                }
                YoutubeTarget::Album(aid) => {
                    let alb = self.catalog.album(&aid).await?;
                    let mut out = vec![SearchItem::Album(alb.summary)];
                    out.extend(alb.tracks.into_iter().map(SearchItem::Track));
                    Ok(out)
                }
                YoutubeTarget::Artist(cid) => {
                    let art = self.catalog.artist(&cid).await?;
                    let mut out = vec![SearchItem::Artist(art.summary)];
                    out.extend(art.songs.into_iter().take(15).map(SearchItem::Track));
                    Ok(out)
                }
            };
        }

        let (resolved_cat, clean_query) = parse_query_prefix(trimmed, category);
        if clean_query.is_empty() {
            return Ok(Vec::new());
        }
        let raw_items = self.catalog.search(resolved_cat, &clean_query, limit).await?;
        let mut ranked = rank_search_items(raw_items, &clean_query, resolved_cat);
        ranked.truncate(limit);
        Ok(ranked)
    }

    pub fn fuzzy_history(&self, query: &str, limit: usize) -> Vec<Suggestion> {
        let settings = self.current_settings();
        if !settings.privacy.save_history {
            return Vec::new();
        }
        let hist = self.store.search_history(40).unwrap_or_default();
        if hist.is_empty() {
            return Vec::new();
        }
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return hist
                .into_iter()
                .take(limit)
                .map(|text| Suggestion {
                    text,
                    kind_hint: "History".to_string(),
                    from_history: true,
                })
                .collect();
        }

        let mut scored: Vec<(f64, String)> = hist
            .into_iter()
            .filter_map(|h| {
                let s = wratio(&q, &h.to_lowercase());
                if s >= 55.0 { Some((s, h)) } else { None }
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored
            .into_iter()
            .take(limit)
            .map(|(_, text)| Suggestion {
                text,
                kind_hint: "History".to_string(),
                from_history: true,
            })
            .collect()
    }

    pub async fn autocomplete(&self, raw_query: &str, limit: usize) -> Vec<Suggestion> {
        let settings = self.current_settings();
        let (cat, clean_q) = parse_query_prefix(raw_query, SearchCategory::All);
        let hist = self.fuzzy_history(&clean_q, 3);
        if clean_q.is_empty() || !settings.privacy.remote_suggestions {
            return hist.into_iter().take(limit).collect();
        }

        let remote = self
            .catalog
            .suggestions(&clean_q)
            .await
            .unwrap_or_default();

        let mut seen: HashSet<String> = hist.iter().map(|s| s.text.to_lowercase()).collect();
        let hint = if cat == SearchCategory::All {
            String::new()
        } else {
            cat.label().trim_end_matches('s').to_string()
        };

        let mut out = hist;
        for item in remote {
            if seen.insert(item.to_lowercase()) {
                out.push(Suggestion {
                    text: item,
                    kind_hint: hint.clone(),
                    from_history: false,
                });
                if out.len() >= limit {
                    break;
                }
            }
        }
        out
    }
}
