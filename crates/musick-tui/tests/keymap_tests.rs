#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use musick_tui::keymap::{Action, InputMode, KeymapEngine};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
}

#[test]
fn collision_free_vim_keys_and_chords_work_cleanly() {
    let mut km = KeymapEngine::new();
    let t0 = Instant::now();

    // L = ToggleLyrics, l = MoveRight, S = SyncNow, c = CreatePlaylist, v = unbound (no CoverGrid)
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('L')), t0),
        Some(Action::ToggleLyrics)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('l')), t0),
        Some(Action::MoveRight)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('S')), t0),
        Some(Action::SyncNow)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('c')), t0),
        Some(Action::CreatePlaylist)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('v')), t0),
        None
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('0')), t0),
        Some(Action::CenterLyrics)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('M')), t0),
        Some(Action::ToggleMute)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('}')), t0),
        Some(Action::SeekForwardFast)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('{')), t0),
        Some(Action::SeekBackwardFast)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('z')), t0),
        None
    );
    assert_eq!(
        km.handle_key_at(
            InputMode::Normal,
            key(KeyCode::Char('z')),
            t0 + Duration::from_millis(50)
        ),
        Some(Action::CenterLyrics)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('g')), t0),
        None
    );
    assert_eq!(
        km.handle_key_at(
            InputMode::Normal,
            key(KeyCode::Char('r')),
            t0 + Duration::from_millis(50)
        ),
        Some(Action::RefreshDiscover)
    );

    // Chords: gg, gt, gT, dd, yy, Ctrl+w h/j/k/l
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('g')), t0),
        None
    );
    assert_eq!(
        km.handle_key_at(
            InputMode::Normal,
            key(KeyCode::Char('g')),
            t0 + Duration::from_millis(50)
        ),
        Some(Action::JumpTop)
    );

    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('g')), t0),
        None
    );
    assert_eq!(
        km.handle_key_at(
            InputMode::Normal,
            key(KeyCode::Char('t')),
            t0 + Duration::from_millis(50)
        ),
        Some(Action::NextTab)
    );

    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('g')), t0),
        None
    );
    assert_eq!(
        km.handle_key_at(
            InputMode::Normal,
            key(KeyCode::Char('T')),
            t0 + Duration::from_millis(50)
        ),
        Some(Action::PrevTab)
    );

    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('d')), t0),
        None
    );
    assert_eq!(
        km.handle_key_at(
            InputMode::Normal,
            key(KeyCode::Char('d')),
            t0 + Duration::from_millis(50)
        ),
        Some(Action::DeleteItem)
    );

    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('y')), t0),
        None
    );
    assert_eq!(
        km.handle_key_at(
            InputMode::Normal,
            key(KeyCode::Char('y')),
            t0 + Duration::from_millis(50)
        ),
        Some(Action::CopyUrl)
    );

    assert_eq!(km.handle_key_at(InputMode::Normal, ctrl('w'), t0), None);
    assert_eq!(
        km.handle_key_at(
            InputMode::Normal,
            key(KeyCode::Char('h')),
            t0 + Duration::from_millis(50)
        ),
        Some(Action::FocusPaneLeft)
    );
    assert_eq!(km.handle_key_at(InputMode::Normal, ctrl('w'), t0), None);
    assert_eq!(
        km.handle_key_at(
            InputMode::Normal,
            key(KeyCode::Char('l')),
            t0 + Duration::from_millis(50)
        ),
        Some(Action::FocusPaneRight)
    );
}

#[test]
fn chord_resets_on_invalid_second_key_and_600ms_timeout_without_swallowing_valid_key() {
    let mut km = KeymapEngine::new();
    let t0 = Instant::now();

    // Invalid second key: `g` then `j` resets chord and dispatches MoveDown (does not swallow `j`!)
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('g')), t0),
        None
    );
    assert_eq!(
        km.handle_key_at(
            InputMode::Normal,
            key(KeyCode::Char('j')),
            t0 + Duration::from_millis(100)
        ),
        Some(Action::MoveDown)
    );
    assert!(km.pending_label().is_none());

    // 600ms timeout: `g` then 700ms later `g` starts a fresh chord rather than firing JumpTop!
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('g')), t0),
        None
    );
    assert_eq!(
        km.handle_key_at(
            InputMode::Normal,
            key(KeyCode::Char('g')),
            t0 + Duration::from_millis(700)
        ),
        None
    );
    assert_eq!(km.pending_label(), Some("g"));
    assert_eq!(
        km.handle_key_at(
            InputMode::Normal,
            key(KeyCode::Char('g')),
            t0 + Duration::from_millis(750)
        ),
        Some(Action::JumpTop)
    );
}

#[test]
fn insert_and_modal_modes_isolate_all_global_and_vim_keys() {
    let mut km = KeymapEngine::new();
    let t0 = Instant::now();

    for mode in [InputMode::Insert, InputMode::Modal] {
        for ch in [
            'L', 'S', 'q', ' ', '1', '2', '3', '4', '5', 'h', 'j', 'k', 'l', 't', '/',
        ] {
            assert_eq!(
                km.handle_key_at(mode, key(KeyCode::Char(ch)), t0),
                Some(Action::InputChar(ch)),
                "mode={mode:?} ch={ch:?} must emit InputChar, never global command"
            );
        }
        assert_eq!(
            km.handle_key_at(mode, key(KeyCode::Esc), t0),
            Some(Action::InputCancel)
        );
        assert_eq!(
            km.handle_key_at(mode, key(KeyCode::Enter), t0),
            Some(Action::InputSubmit)
        );
        assert_eq!(
            km.handle_key_at(mode, key(KeyCode::Backspace), t0),
            Some(Action::InputBackspace)
        );
    }
}

#[test]
fn quick_context_keys_q_queue_shift_q_quit_shift_a_artist_a_album_and_e_menu() {
    let mut km = KeymapEngine::new();
    let t0 = Instant::now();

    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('Q')), t0),
        Some(Action::Quit)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, ctrl('c'), t0),
        Some(Action::Quit)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('q')), t0),
        Some(Action::AppendQueue)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('A')), t0),
        Some(Action::GoArtist)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('a')), t0),
        Some(Action::GoAlbum)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('e')), t0),
        Some(Action::OpenContextMenu)
    );
    assert_eq!(
        km.handle_key_at(InputMode::Normal, key(KeyCode::Char('X')), t0),
        Some(Action::ClearAllQueue)
    );
}
