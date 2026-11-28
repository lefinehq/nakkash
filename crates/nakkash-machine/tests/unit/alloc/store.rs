use std::ptr::NonNull;

use proptest::prelude::*;
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

#[rstest]
#[case::one_means_all_bits(b"1", 0xFF)]
#[case::lower_hex(b"7f", 0x7F)]
#[case::upper_hex(b"FF", 0xFF)]
#[case::one_hex_digit(b"a", 0x0A)]
#[case::zero_is_a_byte(b"00", 0x00)]
#[case::spaces_around(b" a5\n", 0xA5)]
#[case::empty_turns_it_on(b"", 0xFF)]
#[case::a_word_turns_it_on(b"yes", 0xFF)]
#[case::past_a_byte(b"100", 0xFF)]
#[case::not_text(b"\xFF\xFE", 0xFF)]
fn the_clobber_variable_names_a_byte(#[case] value: &[u8], #[case] byte: u8) {
    assert_eq!(poison_named(value), byte);
}

proptest! {
    #[test]
    fn any_clobber_value_names_a_byte_without_panicking(value in any::<Vec<u8>>()) {
        let _ = poison_named(&value);
    }

    #[test]
    fn a_two_digit_hex_byte_names_itself(byte in any::<u8>()) {
        prop_assert_eq!(poison_named(format!("{byte:02x}").as_bytes()), byte);
        prop_assert_eq!(poison_named(format!("{byte:02X}").as_bytes()), byte);
    }
}

#[test]
fn poison_is_none_or_a_byte_once_set() {
    let _turn = turn();
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

/// Nodes on the test's heap, freed at the epochs given: what a chain links
/// in the store's blocks.
fn nodes(freed: &[u64]) -> Vec<Node> {
    freed
        .iter()
        .map(|&freed| Node {
            next: std::ptr::null_mut(),
            freed,
        })
        .collect()
}

/// The epochs of a chain's nodes, top first, popping them all.
fn epochs(chain: &mut Chain) -> Vec<u64> {
    std::iter::from_fn(|| chain.pop())
        // SAFETY: the test's nodes outlive the chain.
        .map(|node| unsafe { node.as_ref() }.freed)
        .collect()
}

#[test]
fn a_chain_is_last_in_first_out() {
    let mut held = nodes(&[1, 2, 3]);
    let mut chain = Chain::new();
    assert!(chain.is_empty());
    for node in &mut held {
        // SAFETY: each node is pushed once and outlives the chain.
        unsafe { chain.push(NonNull::from(node)) };
    }
    assert!(!chain.is_empty());
    assert_eq!(epochs(&mut chain), [3, 2, 1]);
    assert!(chain.is_empty());
}

#[test]
fn splitting_off_stale_nodes_keeps_the_rest_in_order() {
    let mut held = nodes(&[1, 5, 2, 6, 3]);
    let mut chain = Chain::new();
    for node in &mut held {
        // SAFETY: each node is pushed once and outlives the chain.
        unsafe { chain.push(NonNull::from(node)) };
    }
    let mut gone = chain.split_off(|node| node.freed < 4);
    assert_eq!(epochs(&mut chain), [6, 5]);
    let mut gone_epochs = epochs(&mut gone);
    gone_epochs.sort_unstable();
    assert_eq!(gone_epochs, [1, 2, 3]);
}

#[test]
fn splitting_an_empty_chain_or_keeping_everything_changes_nothing() {
    let mut chain = Chain::new();
    assert!(chain.split_off(|_| true).is_empty());
    let mut held = nodes(&[1, 2]);
    for node in &mut held {
        // SAFETY: each node is pushed once and outlives the chain.
        unsafe { chain.push(NonNull::from(node)) };
    }
    assert!(chain.split_off(|_| false).is_empty());
    assert_eq!(epochs(&mut chain), [2, 1]);
}

#[test]
fn a_node_is_written_at_the_start_of_its_block() {
    let mut block = vec![0xABu64; 8];
    let at = block.as_mut_ptr().cast::<u8>();
    // SAFETY: the block holds 64 bytes, aligned to 8, and nothing else
    // reads it while the node is there.
    let node = unsafe { Node::at(at, 42) };
    assert_eq!(node.as_ptr().cast::<u8>(), at);
    // SAFETY: the node was just written.
    let node = unsafe { node.as_ref() };
    assert_eq!((node.freed, node.next), (42, std::ptr::null_mut()));
    assert_eq!(block[2..], [0xABu64; 6], "only the node's bytes written");
}

#[test]
fn the_spin_lock_lets_one_thread_in_at_a_time() {
    let count = Spin::new(0u64);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..10_000 {
                    let mut held = count.lock();
                    // A read and a write apart: a second thread inside would
                    // lose counts.
                    let seen = *held;
                    std::hint::spin_loop();
                    *held = seen + 1;
                }
            });
        }
    });
    assert_eq!(*count.lock(), 80_000);
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
