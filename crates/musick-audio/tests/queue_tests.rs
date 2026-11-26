#![allow(clippy::unwrap_used, clippy::expect_used)]

use musick_audio::PlayQueue;
use musick_core::{QueueRole, RepeatMode, Track};

fn tr(id: &str, dur: u32) -> Track {
    let mut t = Track::minimal(id, format!("Track {id}"));
    t.duration = Some(dur);
    t
}

#[test]
fn two_tier_priority_next_up_and_shuffle_preservation() {
    let mut q = PlayQueue::with_seed(42);
    q.set_tracks(vec![tr("a", 100), tr("b", 120), tr("c", 140), tr("d", 160)], 0);
    assert_eq!(q.current().unwrap().video_id, "a");
    assert_eq!(q.role_at(0), QueueRole::Playing);
    assert_eq!(q.role_at(1), QueueRole::Queued);

    q.play_next(tr("p1", 60));
    q.play_next(tr("p2", 60));
    assert_eq!(q.priority_count(), 2);
    assert_eq!(q.role_at(1), QueueRole::Next);
    assert_eq!(q.role_at(2), QueueRole::Next);
    assert_eq!(q.role_at(3), QueueRole::Queued);

    // Shuffle MUST keep current at index 0 AND preserve priority Next Up slice right after it!
    assert!(q.toggle_shuffle());
    assert_eq!(q.index(), Some(0));
    assert_eq!(q.current().unwrap().video_id, "a");
    assert_eq!(q.priority_count(), 2);
    assert_eq!(q.tracks()[1].video_id, "p2");
    assert_eq!(q.tracks()[2].video_id, "p1");

    // Advance consumes priority tier one by one
    assert_eq!(q.advance(false).unwrap().video_id, "p2");
    assert_eq!(q.priority_count(), 1);
    assert_eq!(q.role_at(0), QueueRole::Played);
    assert_eq!(q.role_at(1), QueueRole::Playing);
    assert_eq!(q.role_at(2), QueueRole::Next);
}

#[test]
fn repeat_modes_undo_dedup_move_and_clear_upcoming() {
    let mut q = PlayQueue::new();
    q.set_tracks(
        vec![tr("a", 100), tr("b", 100), tr("a", 100), tr("b", 100), tr("c", 100)],
        0,
    );

    // Deduplicate removes duplicate upcoming tracks (including duplicate of active "a")
    let removed = q.deduplicate();
    assert_eq!(removed, 2);
    assert_eq!(
        q.tracks().iter().map(|t| t.video_id.as_str()).collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );
    assert!(q.can_undo());

    // Undo restores duplicates
    assert!(q.undo());
    assert_eq!(q.len(), 5);
    let _ = q.deduplicate();

    // Move "c" to index 1
    assert!(q.move_item(2, 1));
    assert_eq!(
        q.tracks().iter().map(|t| t.video_id.as_str()).collect::<Vec<_>>(),
        vec!["a", "c", "b"]
    );

    // Repeat cycle Off -> All -> One -> Off
    assert_eq!(q.cycle_repeat(), RepeatMode::All);
    assert_eq!(q.jump(2).unwrap().video_id, "b");
    assert_eq!(q.advance(false).unwrap().video_id, "a"); // wraps around
    assert_eq!(q.retreat().unwrap().video_id, "b"); // wraps backward

    assert_eq!(q.cycle_repeat(), RepeatMode::One);
    assert_eq!(q.advance(false).unwrap().video_id, "b"); // stays on same track automatically
    assert!(q.advance(true).is_none()); // manual advance past last track returns None under Repeat::One
    let _ = q.jump(0);
    assert_eq!(q.advance(false).unwrap().video_id, "a");
    assert_eq!(q.advance(true).unwrap().video_id, "c");

    // Clear upcoming keeps played + active intact
    let _ = q.jump(1);
    assert_eq!(q.clear_upcoming(), 1);
    assert_eq!(
        q.tracks().iter().map(|t| t.video_id.as_str()).collect::<Vec<_>>(),
        vec!["a", "c"]
    );
    assert!(q.undo());
    assert_eq!(q.len(), 3);

    // 12-level undo cap
    for _ in 0..16 {
        q.append(vec![tr("x", 10)]);
        q.remove(q.len() - 1);
    }
    let mut undo_steps = 0;
    while q.undo() {
        undo_steps += 1;
    }
    assert_eq!(undo_steps, 12);
}
