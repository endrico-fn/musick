use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde_json::Value;

use crate::domain::{LyricLine, Lyrics};

use super::{LyricsService, USER_AGENT};

const TRANSLATE_ENDPOINT: &str = "https://translate.googleapis.com/translate_a/single";

static PUNCT_ONLY_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"^[\s·•♪♫\-_=~.,!?…()\[\]"']+$"#).ok());
static WORD_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"[a-zA-Z]{2,}").ok());

const ID_STOPWORDS: &[&str] = &[
    "aku", "kamu", "kau", "ku", "mu", "nya", "yang", "dan", "di", "ke", "dari", "untuk", "dengan",
    "tidak", "tak", "bukan", "jangan", "kita", "kami", "mereka", "ini", "itu", "sudah", "telah",
    "akan", "bisa", "dapat", "ada", "jadi", "hanya", "saja", "juga", "lagi", "kalau", "jika",
    "bila", "saat", "ketika", "karena", "atau", "tapi", "tetapi", "selalu", "pernah", "cinta",
    "hati", "rindu", "malam", "hari", "hidup", "dalam", "semua", "sendiri", "bersama", "mungkin",
    "ingin",
];

pub fn should_skip_translation(text: &str) -> bool {
    let clean = text.trim();
    if clean.is_empty() {
        return true;
    }
    if PUNCT_ONLY_RE.as_ref().is_some_and(|re| re.is_match(clean)) {
        return true;
    }
    clean.starts_with('[') && clean.ends_with(']')
}

pub fn looks_indonesian(lines: &[LyricLine]) -> bool {
    let Some(re) = WORD_RE.as_ref() else {
        return false;
    };
    let mut total = 0usize;
    let mut id_hits = 0usize;
    for ln in lines {
        for m in re.find_iter(&ln.text) {
            total += 1;
            let lower = m.as_str().to_ascii_lowercase();
            if ID_STOPWORDS.contains(&lower.as_str()) {
                id_hits += 1;
            }
        }
    }
    if total < 6 {
        return false;
    }
    (id_hits as f64 / total as f64) >= 0.30
}

fn parse_translate_payload(data: &Value) -> (String, String) {
    let Some(arr) = data.as_array() else {
        return (String::new(), String::new());
    };
    let src_lang = arr.get(2).and_then(Value::as_str).unwrap_or("").to_string();
    let Some(chunks) = arr.first().and_then(Value::as_array) else {
        return (String::new(), src_lang);
    };
    let mut out = String::new();
    for seg in chunks {
        if let Some(part) = seg
            .as_array()
            .and_then(|s| s.first())
            .and_then(Value::as_str)
        {
            out.push_str(part);
        }
    }
    (out, src_lang)
}

impl LyricsService {
    pub async fn translate_lyrics(
        &self,
        mut lyrics: Lyrics,
        video_id: Option<&str>,
        target_lang: &str,
    ) -> Lyrics {
        if lyrics.is_empty() {
            return lyrics;
        }
        if lyrics.translation_lang == target_lang
            && (lyrics.has_translations() || looks_indonesian(&lyrics.lines))
        {
            return lyrics;
        }

        if target_lang == "id" && looks_indonesian(&lyrics.lines) {
            lyrics.translation_lang = target_lang.to_string();
        } else {
            let texts: Vec<String> = lyrics.lines.iter().map(|l| l.text.clone()).collect();
            let (tr_map, ok) = self.translate_texts(&texts, target_lang).await;
            if !ok {
                return lyrics;
            }
            for line in &mut lyrics.lines {
                if let Some(tr) = tr_map.get(&line.text) {
                    line.translation = tr.clone();
                }
            }
            lyrics.translation_lang = target_lang.to_string();
        }

        if let Some(vid) = video_id {
            let source = self.current_settings().lyrics.source;
            self.cache
                .put_lyrics(&Self::cache_key(vid, source), &lyrics);
        }
        lyrics
    }

    async fn translate_texts(
        &self,
        texts: &[String],
        target_lang: &str,
    ) -> (HashMap<String, String>, bool) {
        let mut unique = Vec::new();
        let mut seen = HashSet::new();
        for t in texts {
            if !should_skip_translation(t) && seen.insert(t.as_str()) {
                unique.push(t.clone());
            }
        }
        if unique.is_empty() {
            return (HashMap::new(), true);
        }

        let mut batches: Vec<Vec<String>> = Vec::new();
        let mut cur: Vec<String> = Vec::new();
        let mut cur_chars = 0usize;
        for item in unique {
            if !cur.is_empty() && (cur.len() >= 25 || cur_chars + item.len() + 1 > 1200) {
                batches.push(cur);
                cur = vec![item.clone()];
                cur_chars = item.len();
            } else {
                cur_chars += item.len() + 1;
                cur.push(item);
            }
        }
        if !cur.is_empty() {
            batches.push(cur);
        }

        let mut mapping = HashMap::new();
        for batch in batches {
            let Ok((parts, src_lang)) = self.translate_chunk(&batch, target_lang).await else {
                return (HashMap::new(), false);
            };
            if src_lang
                .split('-')
                .next()
                .is_some_and(|s| s.eq_ignore_ascii_case(target_lang))
            {
                return (HashMap::new(), true);
            }

            if parts.len() == batch.len() {
                for (orig, tr) in batch.into_iter().zip(parts) {
                    let clean = tr.trim();
                    if !clean.is_empty() && !clean.eq_ignore_ascii_case(orig.trim()) {
                        mapping.insert(orig, clean.to_string());
                    }
                }
            } else {
                for orig in batch {
                    let Ok((single, _)) = self
                        .translate_chunk(std::slice::from_ref(&orig), target_lang)
                        .await
                    else {
                        return (HashMap::new(), false);
                    };
                    let joined = single.join(" ");
                    let clean = joined.trim();
                    if !clean.is_empty() && !clean.eq_ignore_ascii_case(orig.trim()) {
                        mapping.insert(orig, clean.to_string());
                    }
                }
            }
        }

        (mapping, true)
    }

    async fn translate_chunk(
        &self,
        lines: &[String],
        target_lang: &str,
    ) -> Result<(Vec<String>, String), ()> {
        let q = lines.join("\n");
        let resp = self
            .http
            .get(TRANSLATE_ENDPOINT)
            .header("User-Agent", USER_AGENT)
            .query(&[
                ("client", "gtx"),
                ("sl", "auto"),
                ("tl", target_lang),
                ("dt", "t"),
                ("q", q.as_str()),
            ])
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .map_err(|_| ())?;
        if !resp.status().is_success() {
            return Err(());
        }
        let val: Value = resp.json().await.map_err(|_| ())?;
        let (joined, src_lang) = parse_translate_payload(&val);
        let parts: Vec<String> = if joined.is_empty() {
            Vec::new()
        } else {
            joined.lines().map(str::to_string).collect()
        };
        Ok((parts, src_lang))
    }
}
