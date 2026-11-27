use std::collections::HashSet;

use rand::SeedableRng;
use rand::rngs::SmallRng;
use rand::seq::SliceRandom;

use musick_core::{QueueRole, QueueSnapshot, RepeatMode, Track, format_duration};

const MAX_UNDO: usize = 12;

#[derive(Clone)]
struct UndoEntry {
    items: Vec<Track>,
    original: Vec<Track>,
    index: Option<usize>,
    priority_count: usize,
}

/// two-tier playback queue preserving priority next-up tracks over context tracks
pub struct PlayQueue {
    rng: SmallRng,
    items: Vec<Track>,
    original: Vec<Track>,
    index: Option<usize>,
    shuffle: bool,
    repeat: RepeatMode,
    priority_count: usize,
    undo_stack: Vec<UndoEntry>,
}

impl Default for PlayQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayQueue {
    pub fn new() -> Self {
        Self {
            rng: SmallRng::from_rng(&mut rand::rng()),
            items: Vec::new(),
            original: Vec::new(),
            index: None,
            shuffle: false,
            repeat: RepeatMode::Off,
            priority_count: 0,
            undo_stack: Vec::new(),
        }
    }

    pub fn with_seed(seed: u64) -> Self {
        Self {
            rng: SmallRng::seed_from_u64(seed),
            items: Vec::new(),
            original: Vec::new(),
            index: None,
            shuffle: false,
            repeat: RepeatMode::Off,
            priority_count: 0,
            undo_stack: Vec::new(),
        }
    }

    fn snapshot_undo(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.undo_stack.push(UndoEntry {
            items: self.items.clone(),
            original: self.original.clone(),
            index: self.index,
            priority_count: self.priority_count,
        });
        if self.undo_stack.len() > MAX_UNDO {
            self.undo_stack.remove(0);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(entry) = self.undo_stack.pop() else {
            return false;
        };
        self.items = entry.items;
        self.original = entry.original;
        self.index = entry.index;
        self.priority_count = entry.priority_count;
        true
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn tracks(&self) -> &[Track] {
        &self.items
    }

    pub fn index(&self) -> Option<usize> {
        self.index
    }

    pub fn shuffle_enabled(&self) -> bool {
        self.shuffle
    }

    pub fn repeat_mode(&self) -> RepeatMode {
        self.repeat
    }

    pub fn current(&self) -> Option<&Track> {
        self.index.and_then(|i| self.items.get(i))
    }

    pub fn priority_count(&self) -> usize {
        let Some(idx) = self.index else {
            return 0;
        };
        if idx >= self.items.len() {
            return 0;
        }
        let avail = self.items.len().saturating_sub(idx + 1);
        self.priority_count.min(avail)
    }

    pub fn upcoming_tracks(&self) -> &[Track] {
        match self.index {
            None => &self.items,
            Some(idx) if idx + 1 < self.items.len() => &self.items[idx + 1..],
            Some(_) => &[],
        }
    }

    pub fn remaining_seconds(&self) -> u32 {
        self.upcoming_tracks()
            .iter()
            .filter_map(|t| t.duration)
            .fold(0u32, u32::saturating_add)
    }

    pub fn role_at(&self, idx: usize) -> QueueRole {
        match self.index {
            Some(cur) if idx == cur => QueueRole::Playing,
            Some(cur) if idx < cur => QueueRole::Played,
            Some(cur) if idx > cur && idx <= cur + self.priority_count() => QueueRole::Next,
            _ => QueueRole::Queued,
        }
    }

    pub fn summary_text(&self) -> String {
        let total = self.items.len();
        if total == 0 {
            return "Empty".to_string();
        }
        let up_count = self.upcoming_tracks().len();
        let rem_secs = self.remaining_seconds();
        let mut parts = Vec::new();
        if up_count > 0 {
            let prio = self.priority_count();
            if prio > 0 {
                parts.push(format!("{up_count} up next ({prio} prioritized)"));
            } else {
                parts.push(format!("{up_count} up next"));
            }
        } else {
            let plural = if total == 1 { "" } else { "s" };
            parts.push(format!("{total} track{plural}"));
        }
        if rem_secs > 0 {
            parts.push(format_duration(Some(rem_secs)));
        }
        if self.shuffle {
            parts.push("Shuffle".to_string());
        }
        if self.repeat != RepeatMode::Off {
            parts.push(format!("Repeat:{}", self.repeat.as_str()));
        }
        parts.join(" · ")
    }

    pub fn to_snapshot(&self) -> QueueSnapshot {
        QueueSnapshot {
            tracks: self.items.clone(),
            index: self.index,
            priority_count: self.priority_count(),
            can_undo: self.can_undo(),
            summary: self.summary_text(),
        }
    }

    pub fn set_tracks(&mut self, tracks: Vec<Track>, start: usize) -> Option<Track> {
        self.original = tracks.clone();
        self.priority_count = 0;
        if tracks.is_empty() {
            self.items.clear();
            self.index = None;
            return None;
        }
        let start = start.min(tracks.len() - 1);
        if self.shuffle {
            let chosen = tracks[start].clone();
            let mut rest: Vec<Track> = tracks
                .into_iter()
                .enumerate()
                .filter_map(|(i, t)| if i == start { None } else { Some(t) })
                .collect();
            rest.shuffle(&mut self.rng);
            self.items = Vec::with_capacity(rest.len() + 1);
            self.items.push(chosen);
            self.items.extend(rest);
            self.index = Some(0);
        } else {
            self.items = tracks;
            self.index = Some(start);
        }
        self.current().cloned()
    }

    pub fn restore_session(
        &mut self,
        tracks: Vec<Track>,
        index: usize,
        shuffle: bool,
        repeat: RepeatMode,
    ) -> Option<Track> {
        self.original = tracks.clone();
        self.items = tracks;
        self.shuffle = shuffle;
        self.repeat = repeat;
        self.priority_count = 0;
        self.undo_stack.clear();
        if self.items.is_empty() {
            self.index = None;
        } else {
            self.index = Some(index.min(self.items.len() - 1));
        }
        self.current().cloned()
    }

    pub fn append(&mut self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            return;
        }
        self.original.extend(tracks.iter().cloned());
        if self.shuffle {
            let mut pool = tracks;
            pool.shuffle(&mut self.rng);
            self.items.extend(pool);
        } else {
            self.items.extend(tracks);
        }
        if self.index.is_none() && !self.items.is_empty() {
            self.index = Some(0);
        }
    }

    pub fn play_next(&mut self, track: Track) {
        self.play_next_many(vec![track]);
    }

    pub fn play_next_many(&mut self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            return;
        }
        if self.items.is_empty() {
            let extra_prio = tracks.len().saturating_sub(1);
            self.set_tracks(tracks, 0);
            self.priority_count = extra_prio;
            return;
        }
        let pos = self.index.map(|i| i + 1).unwrap_or(0).min(self.items.len());
        let added_len = tracks.len();
        for (offset, trk) in tracks.into_iter().enumerate() {
            self.original.push(trk.clone());
            self.items.insert(pos + offset, trk);
        }
        self.priority_count += added_len;
    }

    pub fn remove(&mut self, idx: usize) -> bool {
        if idx >= self.items.len() {
            return false;
        }
        self.snapshot_undo();
        let prio_end = self.index.map(|c| c + self.priority_count());
        let removed = self.items.remove(idx);
        if let Some(orig_pos) = self
            .original
            .iter()
            .position(|t| t.video_id == removed.video_id)
        {
            self.original.remove(orig_pos);
        }

        if let (Some(cur), Some(end)) = (self.index, prio_end)
            && idx > cur
            && idx <= end
        {
            self.priority_count = self.priority_count.saturating_sub(1);
        }

        if self.items.is_empty() {
            self.index = None;
            self.priority_count = 0;
        } else if let Some(cur) = self.index {
            if idx < cur {
                self.index = Some(cur - 1);
            } else if idx == cur && cur >= self.items.len() {
                self.index = Some(self.items.len() - 1);
                self.priority_count = 0;
            }
        }
        true
    }

    pub fn move_to_next(&mut self, idx: usize) -> bool {
        let Some(cur) = self.index else {
            return false;
        };
        if idx >= self.items.len() || idx == cur {
            return false;
        }
        let dst = if idx > cur { cur + 1 } else { cur };
        self.move_item(idx, dst)
    }

    pub fn move_item(&mut self, src: usize, dst: usize) -> bool {
        let n = self.items.len();
        if src >= n || dst >= n || src == dst {
            return false;
        }
        let old_cur = self.index;
        let old_prio_end = old_cur.map(|c| c + self.priority_count());
        let was_src_prio =
            matches!((old_cur, old_prio_end), (Some(c), Some(end)) if src > c && src <= end);

        let item = self.items.remove(src);
        self.items.insert(dst, item);

        if let Some(cur) = self.index {
            if cur == src {
                self.index = Some(dst);
                self.priority_count = 0;
            } else if src < cur && cur <= dst {
                self.index = Some(cur - 1);
            } else if dst <= cur && cur < src {
                self.index = Some(cur + 1);
            }
        }

        if let Some(new_cur) = self.index
            && old_cur != Some(src)
        {
            let prev_end = old_prio_end.unwrap_or(new_cur);
            if was_src_prio && (dst <= new_cur || dst > prev_end) {
                self.priority_count = self.priority_count.saturating_sub(1);
            } else if !was_src_prio && dst > new_cur && dst <= new_cur + self.priority_count + 1 {
                self.priority_count += 1;
            }
        }

        if !self.shuffle {
            self.original = self.items.clone();
        }
        true
    }

    pub fn deduplicate(&mut self) -> usize {
        if self.items.is_empty() {
            return 0;
        }
        self.snapshot_undo();
        let mut seen: HashSet<String> = HashSet::new();
        if let Some(cur) = self.current() {
            seen.insert(cur.video_id.clone());
        }
        let head_len = self.index.map(|i| i + 1).unwrap_or(0).min(self.items.len());
        let prio_end = self.index.map(|c| c + self.priority_count());

        let mut new_items: Vec<Track> = self.items[..head_len].to_vec();
        let mut new_prio = 0usize;
        let mut removed = 0usize;

        for (offset, trk) in self.items[head_len..].iter().enumerate() {
            let orig_idx = head_len + offset;
            if !seen.insert(trk.video_id.clone()) {
                removed += 1;
                if let Some(pos) = self
                    .original
                    .iter()
                    .rposition(|t| t.video_id == trk.video_id)
                {
                    self.original.remove(pos);
                }
                continue;
            }
            new_items.push(trk.clone());
            if let Some(end) = prio_end
                && orig_idx <= end
            {
                new_prio += 1;
            }
        }
        self.items = new_items;
        self.priority_count = new_prio;
        removed
    }

    pub fn clear_upcoming(&mut self) -> usize {
        let Some(cur) = self.index else {
            return 0;
        };
        if cur + 1 >= self.items.len() {
            return 0;
        }
        self.snapshot_undo();
        let removed = self.items.len() - (cur + 1);
        self.items.truncate(cur + 1);
        if !self.shuffle {
            self.original = self.items.clone();
        } else {
            let kept_ids: HashSet<String> = self.items.iter().map(|t| t.video_id.clone()).collect();
            self.original.retain(|t| kept_ids.contains(&t.video_id));
        }
        self.priority_count = 0;
        removed
    }

    pub fn clear(&mut self) {
        self.snapshot_undo();
        self.items.clear();
        self.original.clear();
        self.index = None;
        self.priority_count = 0;
    }

    pub fn jump(&mut self, idx: usize) -> Option<Track> {
        if idx >= self.items.len() {
            return None;
        }
        if let Some(cur) = self.index {
            let prio_end = cur + self.priority_count();
            if idx > cur && idx <= prio_end {
                self.priority_count = prio_end - idx;
            } else {
                self.priority_count = 0;
            }
        } else {
            self.priority_count = 0;
        }
        self.index = Some(idx);
        self.current().cloned()
    }

    pub fn peek_next(&self) -> Option<&Track> {
        if self.items.is_empty() {
            return None;
        }
        if self.repeat == RepeatMode::One {
            return self.current();
        }
        match self.index {
            Some(cur) if cur + 1 < self.items.len() => self.items.get(cur + 1),
            _ if self.repeat == RepeatMode::All => self.items.first(),
            None => self.items.first(),
            _ => None,
        }
    }

    pub fn advance(&mut self, manual: bool) -> Option<Track> {
        if self.items.is_empty() {
            return None;
        }
        if self.repeat == RepeatMode::One && !manual {
            return self.current().cloned();
        }
        if let Some(cur) = self.index {
            if cur + 1 < self.items.len() {
                self.index = Some(cur + 1);
                self.priority_count = self.priority_count.saturating_sub(1);
                return self.current().cloned();
            }
            if self.repeat == RepeatMode::All {
                self.index = Some(0);
                self.priority_count = 0;
                return self.current().cloned();
            }
            None
        } else {
            self.index = Some(0);
            self.current().cloned()
        }
    }

    pub fn retreat(&mut self) -> Option<Track> {
        if self.items.is_empty() {
            return None;
        }
        match self.index {
            Some(cur) if cur > 0 => {
                self.index = Some(cur - 1);
                self.priority_count = 0;
            }
            Some(0) if self.repeat == RepeatMode::All => {
                self.index = Some(self.items.len() - 1);
                self.priority_count = 0;
            }
            _ => {
                self.index = Some(0);
                self.priority_count = 0;
            }
        }
        self.current().cloned()
    }

    pub fn toggle_shuffle(&mut self) -> bool {
        self.shuffle = !self.shuffle;
        if self.items.is_empty() {
            return self.shuffle;
        }
        let cur = self.current().cloned();
        if self.shuffle {
            if let (Some(cur_idx), Some(cur_track)) = (self.index, cur) {
                let prio_n = self.priority_count();
                let prio_slice = self.items[cur_idx + 1..cur_idx + 1 + prio_n].to_vec();
                let mut rest: Vec<Track> = self
                    .items
                    .iter()
                    .enumerate()
                    .filter_map(|(i, t)| {
                        if i != cur_idx && !(i > cur_idx && i <= cur_idx + prio_n) {
                            Some(t.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                rest.shuffle(&mut self.rng);
                let mut rebuilt = Vec::with_capacity(self.items.len());
                rebuilt.push(cur_track);
                rebuilt.extend(prio_slice);
                rebuilt.extend(rest);
                self.items = rebuilt;
                self.index = Some(0);
                self.priority_count = prio_n;
            } else {
                self.items.shuffle(&mut self.rng);
            }
        } else {
            self.items = self.original.clone();
            self.priority_count = 0;
            if let Some(cur_track) = cur {
                self.index = self
                    .items
                    .iter()
                    .position(|t| t.video_id == cur_track.video_id)
                    .or(Some(0));
            }
        }
        self.shuffle
    }

    pub fn cycle_repeat(&mut self) -> RepeatMode {
        self.repeat = self.repeat.cycle();
        self.repeat
    }
}
