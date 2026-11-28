use rstest::rstest;

use super::*;

const MIB: u64 = 1 << 20;

#[rstest]
#[case::room_for_it(0, MIB, 2 * MIB, Some(MIB))]
#[case::exactly_the_room(MIB, MIB, 2 * MIB, Some(2 * MIB))]
#[case::one_byte_past(MIB, MIB + 1, 2 * MIB, None)]
#[case::no_room_at_all(0, MIB, 0, None)]
#[case::already_past_a_lowered_room(3 * MIB, MIB, 2 * MIB, None)]
#[case::no_overflow(u64::MAX - 1, MIB, u64::MAX, None)]
fn a_block_is_kept_only_inside_the_room(
    #[case] kept: u64,
    #[case] bytes: u64,
    #[case] room: u64,
    #[case] raised_to: Option<u64>,
) {
    assert_eq!(raised(kept, bytes, room), raised_to);
}

#[rstest]
#[case::freed_now(5, 5, 2, false)]
#[case::idle_the_ticks(7, 5, 2, false)]
#[case::idle_past_them(8, 5, 2, true)]
#[case::no_ticks_kept(6, 5, 0, true)]
#[case::freed_after_a_tick_read(5, 6, 2, false)]
fn a_block_is_stale_once_idle_past_the_ticks(
    #[case] now: u64,
    #[case] freed: u64,
    #[case] after: u64,
    #[case] is_stale: bool,
) {
    assert_eq!(stale(now, freed, after), is_stale);
}

#[test]
fn poison_is_none_until_set_then_a_byte_or_none() {
    let _turn = turn();
    // No variable of the environment is read: none until set.
    assert_eq!(poison(), None);
    for byte in [Some(0x00), Some(0x7F), Some(0xFF), None] {
        set_poison(byte);
        assert_eq!(poison(), byte);
    }
}

#[test]
fn a_store_without_a_keep_function_has_no_room() {
    // The library's own tests never register one.
    assert_eq!(room(), 0);
}

#[test]
fn every_class_starts_empty() {
    let _turn = turn();
    assert!(FREE.iter().all(|class| class.lock().is_empty()));
}

/// One test at a time for the tests that read or set the store's own
/// state.
fn turn() -> std::sync::MutexGuard<'static, ()> {
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    TURN.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
