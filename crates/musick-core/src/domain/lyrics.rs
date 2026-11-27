use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricLine {
    pub time: f64,
    pub text: String,
    #[serde(default)]
    pub translation: String,
}

impl LyricLine {
    pub fn new(time: f64, text: impl Into<String>) -> Self {
        Self {
            time,
            text: text.into(),
            translation: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Lyrics {
    #[serde(default)]
    pub lines: Vec<LyricLine>,
    #[serde(default)]
    pub synced: bool,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub translation_lang: String,
}

impl Lyrics {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn has_translations(&self) -> bool {
        self.lines.iter().any(|l| !l.translation.trim().is_empty())
    }

    pub fn active_line_index(&self, position_secs: f64) -> Option<usize> {
        if self.lines.is_empty() || !self.synced {
            return None;
        }
        let target = position_secs + 0.05;
        let idx = self.lines.partition_point(|line| line.time <= target);
        idx.checked_sub(1)
    }
}
