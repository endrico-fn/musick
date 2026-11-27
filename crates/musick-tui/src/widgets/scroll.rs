use std::cell::Cell;

pub const DEFAULT_SCROLLOFF: usize = 2;

/// stateful viewport offset with smooth scrolloff hysteresis
#[derive(Debug, Clone, Default)]
pub struct ScrollState {
    offset: Cell<usize>,
}

impl ScrollState {
    pub fn new() -> Self {
        Self {
            offset: Cell::new(0),
        }
    }

    pub fn offset(&self) -> usize {
        self.offset.get()
    }

    pub fn reset(&self) {
        self.offset.set(0);
    }

    pub fn ensure_visible(
        &self,
        cursor: usize,
        visible: usize,
        total: usize,
        scrolloff: usize,
    ) -> usize {
        let next = compute_scroll_offset(self.offset.get(), cursor, visible, total, scrolloff);
        self.offset.set(next);
        next
    }
}

pub fn compute_scroll_offset(
    prev_offset: usize,
    cursor: usize,
    visible: usize,
    total: usize,
    scrolloff: usize,
) -> usize {
    if total == 0 || visible == 0 || total <= visible {
        return 0;
    }
    let max_off = total - visible;
    let clamped_cursor = cursor.min(total - 1);
    let margin = scrolloff.min(visible.saturating_sub(1) / 2);
    let mut off = prev_offset.min(max_off);

    if clamped_cursor < off.saturating_add(margin) {
        off = clamped_cursor.saturating_sub(margin);
    } else if clamped_cursor.saturating_add(margin) >= off.saturating_add(visible) {
        off = clamped_cursor
            .saturating_add(margin)
            .saturating_add(1)
            .saturating_sub(visible)
            .min(max_off);
    }
    off
}
