use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const CHORD_TIMEOUT: Duration = Duration::from_millis(600);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    Normal,
    Insert,
    Modal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingChord {
    G(Instant),
    D(Instant),
    Y(Instant),
    CtrlW(Instant),
}

impl PendingChord {
    fn started_at(self) -> Instant {
        match self {
            Self::G(t) | Self::D(t) | Self::Y(t) | Self::CtrlW(t) => t,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // Root Tabs & Sub-tabs & Panes
    SwitchTab(usize),
    NextTab,
    PrevTab,
    NextSubTab,
    PrevSubTab,
    FocusPaneLeft,
    FocusPaneDown,
    FocusPaneUp,
    FocusPaneRight,

    // Global Playback & Lyrics & Sync
    ToggleLyrics,
    SyncNow,
    ToggleTranslation,
    TogglePause,
    NextTrack,
    PrevTrack,
    SeekForward,
    SeekBackward,
    VolumeUp,
    VolumeDown,
    ToggleShuffle,
    CycleRepeat,
    StartSearch,
    GoBack,
    Escape,
    ToggleHelp,
    Quit,

    // Navigation
    MoveLeft,
    MoveDown,
    MoveUp,
    MoveRight,
    JumpTop,
    JumpBottom,
    HalfPageDown,
    HalfPageUp,

    // Item & Edit Actions
    Activate,
    PlayDirect,
    PlayNext,
    AppendQueue,
    AddToPlaylist,
    ToggleLike,
    ToggleSave,
    StartRadio,
    CycleSort,
    ToggleGridList,
    CycleCoverSize,
    DeleteItem,
    MoveItemDown,
    MoveItemUp,
    Undo,
    Deduplicate,
    ClearUpcoming,

    // Playlist Management & Utility
    RenamePlaylist,
    ImportPlaylist,
    ExportPlaylist,
    UploadPlaylist,
    CopyUrl,

    // Text Input (Insert / Modal modes)
    InputChar(char),
    InputBackspace,
    InputSubmit,
    InputTab,
    InputBackTab,
    InputCancel,
    InputUp,
    InputDown,
    InputLeft,
    InputRight,
}

#[derive(Debug, Default)]
pub struct KeymapEngine {
    pending: Option<PendingChord>,
}

impl KeymapEngine {
    pub fn new() -> Self {
        Self { pending: None }
    }

    pub fn pending_label(&self) -> Option<&'static str> {
        match self.pending {
            Some(PendingChord::G(_)) => Some("g"),
            Some(PendingChord::D(_)) => Some("d"),
            Some(PendingChord::Y(_)) => Some("y"),
            Some(PendingChord::CtrlW(_)) => Some("C-w"),
            None => None,
        }
    }

    pub fn clear_pending(&mut self) {
        self.pending = None;
    }

    pub fn handle_key(&mut self, mode: InputMode, key: KeyEvent) -> Option<Action> {
        self.handle_key_at(mode, key, Instant::now())
    }

    pub fn handle_key_at(
        &mut self,
        mode: InputMode,
        key: KeyEvent,
        now: Instant,
    ) -> Option<Action> {
        // Expire stale chords cleanly after 600ms
        if let Some(chord) = self.pending
            && now.saturating_duration_since(chord.started_at()) >= CHORD_TIMEOUT
        {
            self.pending = None;
        }

        // Ctrl+C always cleanly quits
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.pending = None;
            return Some(Action::Quit);
        }

        // Strict Insert / Modal isolation (Review Focus #3)
        if matches!(mode, InputMode::Insert | InputMode::Modal) {
            self.pending = None;
            return self.handle_text_mode(key);
        }

        self.handle_normal_mode(key, now)
    }

    fn handle_text_mode(&self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => Some(Action::InputCancel),
            KeyCode::Enter => Some(Action::InputSubmit),
            KeyCode::Backspace => Some(Action::InputBackspace),
            KeyCode::Tab => Some(Action::InputTab),
            KeyCode::BackTab => Some(Action::InputBackTab),
            KeyCode::Up => Some(Action::InputUp),
            KeyCode::Down => Some(Action::InputDown),
            KeyCode::Left => Some(Action::InputLeft),
            KeyCode::Right => Some(Action::InputRight),
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::InputChar(ch))
            }
            _ => None,
        }
    }

    fn handle_normal_mode(&mut self, key: KeyEvent, now: Instant) -> Option<Action> {
        // 1. Check active chord prefix first
        if let Some(chord) = self.pending.take() {
            match (chord, key.code, key.modifiers) {
                (PendingChord::G(_), KeyCode::Char('g'), m)
                    if !m.contains(KeyModifiers::CONTROL) =>
                {
                    return Some(Action::JumpTop);
                }
                (PendingChord::G(_), KeyCode::Char('t'), m)
                    if !m.contains(KeyModifiers::CONTROL) =>
                {
                    return Some(Action::NextTab);
                }
                (PendingChord::G(_), KeyCode::Char('T'), m)
                    if !m.contains(KeyModifiers::CONTROL) =>
                {
                    return Some(Action::PrevTab);
                }
                (PendingChord::D(_), KeyCode::Char('d'), m)
                    if !m.contains(KeyModifiers::CONTROL) =>
                {
                    return Some(Action::DeleteItem);
                }
                (PendingChord::Y(_), KeyCode::Char('y'), m)
                    if !m.contains(KeyModifiers::CONTROL) =>
                {
                    return Some(Action::CopyUrl);
                }
                (PendingChord::CtrlW(_), KeyCode::Char('h') | KeyCode::Left, _) => {
                    return Some(Action::FocusPaneLeft);
                }
                (PendingChord::CtrlW(_), KeyCode::Char('j') | KeyCode::Down, _) => {
                    return Some(Action::FocusPaneDown);
                }
                (PendingChord::CtrlW(_), KeyCode::Char('k') | KeyCode::Up, _) => {
                    return Some(Action::FocusPaneUp);
                }
                (PendingChord::CtrlW(_), KeyCode::Char('l') | KeyCode::Right, _) => {
                    return Some(Action::FocusPaneRight);
                }
                (_, KeyCode::Esc, _) => {
                    // Esc cancels chord cleanly
                    return None;
                }
                _ => {
                    // Invalid second key: reset chord AND fall through to dispatch the key!
                }
            }
        }

        // 2. Ctrl-modified bindings
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return match key.code {
                KeyCode::Char('w') => {
                    self.pending = Some(PendingChord::CtrlW(now));
                    None
                }
                KeyCode::Char('d') => Some(Action::HalfPageDown),
                KeyCode::Char('u') => Some(Action::HalfPageUp),
                _ => None,
            };
        }

        // 3. Single-key bindings and chord starters
        match key.code {
            KeyCode::Char('g') => {
                self.pending = Some(PendingChord::G(now));
                None
            }
            KeyCode::Char('d') => {
                self.pending = Some(PendingChord::D(now));
                None
            }
            KeyCode::Char('y') => {
                self.pending = Some(PendingChord::Y(now));
                None
            }

            // Root tabs 1..5
            KeyCode::Char('1') => Some(Action::SwitchTab(0)),
            KeyCode::Char('2') => Some(Action::SwitchTab(1)),
            KeyCode::Char('3') => Some(Action::SwitchTab(2)),
            KeyCode::Char('4') => Some(Action::SwitchTab(3)),
            KeyCode::Char('5') => Some(Action::SwitchTab(4)),

            // Sub-tabs
            KeyCode::Tab => Some(Action::NextSubTab),
            KeyCode::BackTab => Some(Action::PrevSubTab),

            // Global actions (strictly collision-free!)
            KeyCode::Char('L') => Some(Action::ToggleLyrics),
            KeyCode::Char('S') => Some(Action::SyncNow),
            KeyCode::Char('t') => Some(Action::ToggleTranslation),
            KeyCode::Char(' ') => Some(Action::TogglePause),
            KeyCode::Char('n') => Some(Action::NextTrack),
            KeyCode::Char('b') => Some(Action::PrevTrack),
            KeyCode::Char('>' | ']') => Some(Action::SeekForward),
            KeyCode::Char('<' | '[') => Some(Action::SeekBackward),
            KeyCode::Char('+' | '=') => Some(Action::VolumeUp),
            KeyCode::Char('-') => Some(Action::VolumeDown),
            KeyCode::Char('s') => Some(Action::ToggleShuffle),
            KeyCode::Char('m') => Some(Action::CycleRepeat),
            KeyCode::Char('/') => Some(Action::StartSearch),
            KeyCode::Backspace | KeyCode::Char('H') => Some(Action::GoBack),
            KeyCode::Esc => Some(Action::Escape),
            KeyCode::Char('?') => Some(Action::ToggleHelp),
            KeyCode::Char('q') => Some(Action::Quit),

            // Vim & Arrow Navigation
            KeyCode::Char('h') | KeyCode::Left => Some(Action::MoveLeft),
            KeyCode::Char('j') | KeyCode::Down => Some(Action::MoveDown),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::MoveUp),
            KeyCode::Char('l') | KeyCode::Right => Some(Action::MoveRight),
            KeyCode::Char('G') => Some(Action::JumpBottom),
            KeyCode::PageDown => Some(Action::HalfPageDown),
            KeyCode::PageUp => Some(Action::HalfPageUp),

            // Item Actions
            KeyCode::Enter => Some(Action::Activate),
            KeyCode::Char('p') => Some(Action::PlayDirect),
            KeyCode::Char('N') => Some(Action::PlayNext),
            KeyCode::Char('a') => Some(Action::AppendQueue),
            KeyCode::Char('P') => Some(Action::AddToPlaylist),
            KeyCode::Char('f') => Some(Action::ToggleLike),
            KeyCode::Char('w') => Some(Action::ToggleSave),
            KeyCode::Char('r') => Some(Action::StartRadio),
            KeyCode::Char('o') => Some(Action::CycleSort),
            KeyCode::Char('v') => Some(Action::ToggleGridList),
            KeyCode::Char('c') => Some(Action::CycleCoverSize),

            // Edit Actions
            KeyCode::Delete => Some(Action::DeleteItem),
            KeyCode::Char('J') => Some(Action::MoveItemDown),
            KeyCode::Char('K') => Some(Action::MoveItemUp),
            KeyCode::Char('u') => Some(Action::Undo),
            KeyCode::Char('D') => Some(Action::Deduplicate),
            KeyCode::Char('C') => Some(Action::ClearUpcoming),

            // Playlist Management
            KeyCode::Char('R') => Some(Action::RenamePlaylist),
            KeyCode::Char('I') => Some(Action::ImportPlaylist),
            KeyCode::Char('E') => Some(Action::ExportPlaylist),
            KeyCode::Char('U') => Some(Action::UploadPlaylist),

            _ => None,
        }
    }
}
