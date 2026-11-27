use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::domain::{LyricLine, Lyrics};

static TS_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\[(\d{1,3}):(\d{2})(?:[.:](\d{1,3}))?\]").ok());
static OFFSET_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)^\[offset:([+-]?\d+)\]$").ok());
static META_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^\[[a-zA-Z]+:.*\]$").ok());
static VTT_TS_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?:(\d{1,2}):)?(\d{2}):(\d{2})[.,](\d{3})\s*-->").ok());
static TAG_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"<[^>]+>").ok());

pub fn parse_lrc(raw: &str, source: &str) -> Lyrics {
    if raw.trim().is_empty() {
        return Lyrics::default();
    }

    let mut offset_sec = 0.0_f64;
    let mut entries = Vec::new();
    let mut plain_lines = Vec::new();

    for raw_line in raw.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(off_re) = OFFSET_RE.as_ref()
            && let Some(caps) = off_re.captures(line)
            && let Ok(ms) = caps[1].parse::<i64>()
        {
            offset_sec = ms as f64 / 1000.0;
            continue;
        }
        let Some(ts_re) = TS_RE.as_ref() else {
            continue;
        };
        let matches: Vec<_> = ts_re.captures_iter(line).collect();
        if matches.is_empty() {
            if !META_RE.as_ref().is_some_and(|re| re.is_match(line)) {
                plain_lines.push(line.to_string());
            }
            continue;
        }
        let text = ts_re.replace_all(line, "").trim().to_string();
        for caps in matches {
            let mins: f64 = caps[1].parse().unwrap_or(0.0);
            let secs: f64 = caps[2].parse().unwrap_or(0.0);
            let frac_ms: f64 = caps
                .get(3)
                .map(|m| {
                    let s = m.as_str();
                    let padded = format!("{s:0<3}");
                    padded[..3].parse::<f64>().unwrap_or(0.0)
                })
                .unwrap_or(0.0);
            let ts = (mins * 60.0 + secs + frac_ms / 1000.0 + offset_sec).max(0.0);
            entries.push(LyricLine::new(ts, text.clone()));
        }
    }

    if !entries.is_empty() {
        entries.sort_by(|a, b| a.time.total_cmp(&b.time));
        return Lyrics {
            lines: entries,
            synced: true,
            source: source.to_string(),
            translation_lang: String::new(),
        };
    }

    if !plain_lines.is_empty() {
        return Lyrics {
            lines: plain_lines
                .into_iter()
                .map(|t| LyricLine::new(0.0, t))
                .collect(),
            synced: false,
            source: source.to_string(),
            translation_lang: String::new(),
        };
    }

    Lyrics::default()
}

pub fn parse_vtt_captions(raw: &str, source: &str) -> Lyrics {
    if raw.trim().is_empty() {
        return Lyrics::default();
    }

    let mut entries: Vec<LyricLine> = Vec::new();
    let mut cur_time: Option<f64> = None;
    let mut buf: Vec<String> = Vec::new();

    let flush = |cur_time: &mut Option<f64>,
                 buf: &mut Vec<String>,
                 entries: &mut Vec<LyricLine>| {
        if let Some(t) = cur_time.take()
            && !buf.is_empty()
        {
            let joined = buf.join(" ");
            let stripped = match TAG_RE.as_ref() {
                Some(re) => re.replace_all(&joined, "").into_owned(),
                None => joined,
            };
            let text = html_escape::decode_html_entities(&stripped)
                .trim()
                .to_string();
            if !text.is_empty() && entries.last().map(|e| e.text.as_str()) != Some(text.as_str()) {
                entries.push(LyricLine::new(t, text));
            }
        }
        buf.clear();
    };

    for raw_ln in raw.lines() {
        let ln = raw_ln.trim();
        if ln.is_empty() {
            flush(&mut cur_time, &mut buf, &mut entries);
            continue;
        }
        if ln.starts_with("WEBVTT")
            || ln.starts_with("Kind:")
            || ln.starts_with("Language:")
            || ln.starts_with("NOTE")
        {
            continue;
        }
        if let Some(vtt_re) = VTT_TS_RE.as_ref()
            && let Some(caps) = vtt_re.captures(ln)
        {
            flush(&mut cur_time, &mut buf, &mut entries);
            let hrs: f64 = caps
                .get(1)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0.0);
            let mins: f64 = caps[2].parse().unwrap_or(0.0);
            let secs: f64 = caps[3].parse().unwrap_or(0.0);
            let ms: f64 = caps[4].parse().unwrap_or(0.0);
            cur_time = Some(hrs * 3600.0 + mins * 60.0 + secs + ms / 1000.0);
            continue;
        }
        if cur_time.is_none() && ln.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if cur_time.is_some() {
            buf.push(ln.to_string());
        }
    }
    flush(&mut cur_time, &mut buf, &mut entries);

    if entries.is_empty() {
        Lyrics::default()
    } else {
        Lyrics {
            lines: entries,
            synced: true,
            source: source.to_string(),
            translation_lang: String::new(),
        }
    }
}

pub fn parse_json3_captions(data: &Value, source: &str) -> Lyrics {
    let Some(events) = data.get("events").and_then(Value::as_array) else {
        return Lyrics::default();
    };
    let mut entries: Vec<LyricLine> = Vec::new();
    for ev in events {
        let Some(segs) = ev.get("segs").and_then(Value::as_array) else {
            continue;
        };
        let mut raw_text = String::new();
        for s in segs {
            if let Some(u) = s.get("utf8").and_then(Value::as_str) {
                raw_text.push_str(u);
            }
        }
        let decoded = html_escape::decode_html_entities(&raw_text);
        let text = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        let t_ms = ev
            .get("tStartMs")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .max(0.0);
        if entries.last().map(|e| e.text.as_str()) != Some(text.as_str()) {
            entries.push(LyricLine::new(t_ms / 1000.0, text));
        }
    }

    if entries.is_empty() {
        Lyrics::default()
    } else {
        Lyrics {
            lines: entries,
            synced: true,
            source: source.to_string(),
            translation_lang: String::new(),
        }
    }
}

pub fn active_line_index(lyrics: &Lyrics, position_secs: f64) -> Option<usize> {
    lyrics.active_line_index(position_secs)
}

/// score lrclib `/search` candidates by duration closeness, title/artist match, and synced lyrics availability
pub fn pick_best_lrclib_candidate(
    results: &[Value],
    title: &str,
    artist: Option<&str>,
    duration: Option<u32>,
) -> Option<Value> {
    if results.is_empty() {
        return None;
    }
    let norm_t = crate::search::norm_text(title);
    let norm_a = artist.map(crate::search::norm_text);
    let mut best: Option<(f64, &Value)> = None;

    for cand in results {
        let has_synced = cand
            .get("syncedLyrics")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty());
        let has_plain = cand
            .get("plainLyrics")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty());
        if !has_synced && !has_plain {
            continue;
        }

        let mut score = if has_synced { 20.0 } else { 0.0 };
        if let Some(ct) = cand.get("trackName").and_then(Value::as_str) {
            let ct_norm = crate::search::norm_text(ct);
            if ct_norm == norm_t {
                score += 45.0;
            } else {
                score += crate::search::wratio(&norm_t, &ct_norm) * 0.35;
            }
        }
        if let (Some(na), Some(ca)) = (&norm_a, cand.get("artistName").and_then(Value::as_str)) {
            let ca_norm = crate::search::norm_text(ca);
            if ca_norm == *na || ca_norm.contains(na.as_str()) || na.contains(ca_norm.as_str()) {
                score += 30.0;
            } else {
                score += crate::search::wratio(na, &ca_norm) * 0.2;
            }
        }
        if let (Some(target_dur), Some(cand_dur)) = (
            duration.filter(|d| *d > 0),
            cand.get("duration").and_then(Value::as_f64),
        ) {
            let diff = (cand_dur - f64::from(target_dur)).abs();
            if diff <= 2.5 {
                score += 40.0;
            } else if diff <= 5.5 {
                score += 25.0;
            } else if diff <= 10.5 {
                score += 5.0;
            } else if diff > 30.0 {
                score -= 100.0;
            } else if diff > 15.0 {
                score -= 50.0;
            }
        }

        if best.is_none_or(|(bs, _)| score > bs) {
            best = Some((score, cand));
        }
    }

    best.map(|(_, v)| v.clone())
}
