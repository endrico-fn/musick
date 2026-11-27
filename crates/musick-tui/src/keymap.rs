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
    Z(Instant),
    CtrlW(Instant),
}

impl PendingChord {
    fn started_at(self) -> Instant {
        match self {
            Self::G(t) | Self::D(t) | Self::Y(t) | Self::Z(t) | Self::CtrlW(t) => t,
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
    CenterLyrics,
    LyricsOffsetEarlier,
    LyricsOffsetLater,
    SyncNow,
    ToggleTranslation,
    TogglePause,
    ToggleMute,
    NextTrack,
    PrevTrack,
    SeekForward,
    SeekBackward,
    SeekForwardFast,
    SeekBackwardFast,
    VolumeUp,
    VolumeDown,
    ToggleShuffle,
    CycleRepeat,
    StartSearch,
    GlobalSearch,
    RefreshDiscover,
    GoBack,
    Escape,
    ToggleHelp,
    Quit,

    // Cross-Entity Jumps
    GoArtist,
    GoAlbum,
    GoPlayingArtist,
    GoPlayingAlbum,

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
    ToggleArtistFollow,
    ShowArtistBio,
    ArtistShuffle,
    StartRadio,
    CycleSort,
    CreatePlaylist,
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
    CopyCollectionUrl,

    // Text Input (Insert / Modal modes)
    InputChar(char),
    InputBackspace,
    InputDelete,
    InputHome,
    InputEnd,
    InputClearLine,
    InputDeleteWord,
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
            Some(PendingChord::Z(_)) => Some("z"),
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
        if let Some(chord) = self.pending
            && now.saturating_duration_since(chord.started_at()) >= CHORD_TIMEOUT
        {
            self.pending = None;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.pending = None;
            return Some(Action::Quit);
        }

        // isolate insert and modal inputs from global bindings
        if matches!(mode, InputMode::Insert | InputMode::Modal) {
            self.pending = None;
            return self.handle_text_mode(key);
        }

        self.handle_normal_mode(key, now)
    }

    fn handle_text_mode(&self, key: KeyEvent) -> Option<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return match key.code {
                KeyCode::Char('u') => Some(Action::InputClearLine),
                KeyCode::Char('w') => Some(Action::InputDeleteWord),
                KeyCode::Char('a') => Some(Action::InputHome),
                KeyCode::Char('e') => Some(Action::InputEnd),
                _ => None,
            };
        }
        match key.code {
            KeyCode::Esc => Some(Action::InputCancel),
            KeyCode::Enter => Some(Action::InputSubmit),
            KeyCode::Backspace => Some(Action::InputBackspace),
            KeyCode::Delete => Some(Action::InputDelete),
            KeyCode::Home => Some(Action::InputHome),
            KeyCode::End => Some(Action::InputEnd),
            KeyCode::Tab => Some(Action::InputTab),
            KeyCode::BackTab => Some(Action::InputBackTab),
            KeyCode::Up => Some(Action::InputUp),
            KeyCode::Down => Some(Action::InputDown),
            KeyCode::Left => Some(Action::InputLeft),
            KeyCode::Right => Some(Action::InputRight),
            KeyCode::Char(ch) => Some(Action::InputChar(ch)),
            _ => None,
        }
    }

    fn handle_normal_mode(&mut self, key: KeyEvent, now: Instant) -> Option<Action> {
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
                (PendingChord::G(_), KeyCode::Char('a'), m)
                    if !m.contains(KeyModifiers::CONTROL) =>
                {
                    return Some(Action::GoArtist);
                }
                (PendingChord::G(_), KeyCode::Char('l'), m)
                    if !m.contains(KeyModifiers::CONTROL) =>
                {
                    return Some(Action::GoAlbum);
                }
                (PendingChord::G(_), KeyCode::Char('A'), m)
                    if !m.contains(KeyModifiers::CONTROL) =>
                {
                    return Some(Action::GoPlayingArtist);
                }
                (PendingChord::G(_), KeyCode::Char('L'), m)
                    if !m.contains(KeyModifiers::CONTROL) =>
                {
                    return Some(Action::GoPlayingAlbum);
                }
                (PendingChord::G(_), KeyCode::Char('r'), m)
                    if !m.contains(KeyModifiers::CONTROL) =>
                {
                    return Some(Action::RefreshDiscover);
                }
                (PendingChord::G(_), KeyCode::Char('/'), m)
                    if !m.contains(KeyModifiers::CONTROL) =>
                {
                    return Some(Action::GlobalSearch);
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
                (PendingChord::Z(_), KeyCode::Char('z'), m)
                    if !m.contains(KeyModifiers::CONTROL) =>
                {
                    return Some(Action::CenterLyrics);
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
                    return None;
                }
                _ => {
                    // invalid second key resets chord and falls through
                }
            }
        }

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
            KeyCode::Char('z') => {
                self.pending = Some(PendingChord::Z(now));
                None
            }

            // root tabs 1..5
            KeyCode::Char('1') => Some(Action::SwitchTab(0)),
            KeyCode::Char('2') => Some(Action::SwitchTab(1)),
            KeyCode::Char('3') => Some(Action::SwitchTab(2)),
            KeyCode::Char('4') => Some(Action::SwitchTab(3)),
            KeyCode::Char('5') => Some(Action::SwitchTab(4)),
            KeyCode::Char('0') => Some(Action::CenterLyrics),

            // sub-tabs
            KeyCode::Tab => Some(Action::NextSubTab),
            KeyCode::BackTab => Some(Action::PrevSubTab),

            // global playback, lyrics, and sync
            KeyCode::Char('L') => Some(Action::ToggleLyrics),
            KeyCode::Char('S') => Some(Action::SyncNow),
            KeyCode::Char('t') => Some(Action::ToggleTranslation),
            KeyCode::Char(',') => Some(Action::LyricsOffsetEarlier),
            KeyCode::Char('.') => Some(Action::LyricsOffsetLater),
            KeyCode::Char(' ') => Some(Action::TogglePause),
            KeyCode::Char('M') => Some(Action::ToggleMute),
            KeyCode::Char('n') => Some(Action::NextTrack),
            KeyCode::Char('b') => Some(Action::PrevTrack),
            KeyCode::Char('>' | ']') => Some(Action::SeekForward),
            KeyCode::Char('<' | '[') => Some(Action::SeekBackward),
            KeyCode::Char('}') => Some(Action::SeekForwardFast),
            KeyCode::Char('{') => Some(Action::SeekBackwardFast),
            KeyCode::Char('+' | '=') => Some(Action::VolumeUp),
            KeyCode::Char('-') => Some(Action::VolumeDown),
            KeyCode::Char('s') => Some(Action::ToggleShuffle),
            KeyCode::Char('m') => Some(Action::CycleRepeat),
            KeyCode::Char('/') => Some(Action::StartSearch),
            KeyCode::Backspace | KeyCode::Char('H') => Some(Action::GoBack),
            KeyCode::Esc => Some(Action::Escape),
            KeyCode::Char('?') => Some(Action::ToggleHelp),
            KeyCode::Char('q') => Some(Action::Quit),

            // navigation
            KeyCode::Char('h') | KeyCode::Left => Some(Action::MoveLeft),
            KeyCode::Char('j') | KeyCode::Down => Some(Action::MoveDown),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::MoveUp),
            KeyCode::Char('l') | KeyCode::Right => Some(Action::MoveRight),
            KeyCode::Char('G') => Some(Action::JumpBottom),
            KeyCode::PageDown => Some(Action::HalfPageDown),
            KeyCode::PageUp => Some(Action::HalfPageUp),

            // item actions
            KeyCode::Enter => Some(Action::Activate),
            KeyCode::Char('p') => Some(Action::PlayDirect),
            KeyCode::Char('N') => Some(Action::PlayNext),
            KeyCode::Char('a') => Some(Action::AppendQueue),
            KeyCode::Char('P') => Some(Action::AddToPlaylist),
            KeyCode::Char('f') => Some(Action::ToggleLike),
            KeyCode::Char('w') => Some(Action::ToggleSave),
            KeyCode::Char('W') => Some(Action::ToggleArtistFollow),
            KeyCode::Char('i') => Some(Action::ShowArtistBio),
            KeyCode::Char('x') => Some(Action::ArtistShuffle),
            KeyCode::Char('r') => Some(Action::StartRadio),
            KeyCode::Char('o') => Some(Action::CycleSort),
            KeyCode::Char('c') => Some(Action::CreatePlaylist),

            // edit actions
            KeyCode::Delete => Some(Action::DeleteItem),
            KeyCode::Char('J') => Some(Action::MoveItemDown),
            KeyCode::Char('K') => Some(Action::MoveItemUp),
            KeyCode::Char('u') => Some(Action::Undo),
            KeyCode::Char('D') => Some(Action::Deduplicate),
            KeyCode::Char('C') => Some(Action::ClearUpcoming),

            // playlist management & copy
            KeyCode::Char('R') => Some(Action::RenamePlaylist),
            KeyCode::Char('I') => Some(Action::ImportPlaylist),
            KeyCode::Char('E') => Some(Action::ExportPlaylist),
            KeyCode::Char('U') => Some(Action::UploadPlaylist),
            KeyCode::Char('Y') => Some(Action::CopyCollectionUrl),

            _ => None,
        }
    }
}
