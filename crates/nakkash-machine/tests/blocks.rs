//! [`GivesBack`]'s block store as the global allocator: what it keeps, what
//! it hands out again and with which bytes, and that it never holds more
//! than its keep function allows.
//!
//! The store, its counters and what is resident are the process's, so the
//! tests take turns. A test that needs a process of its own (no keep
//! function, the clobber variable, the allocator entered from inside
//! itself) runs one of the `child_` tests in a child process.
//!
//! Tests that look at the bytes of a block from `alloc` call [`GivesBack`]'s
//! methods directly, as plain functions, and read through the pointer they
//! return: the one place a test may look at memory it did not write, to
//! show what the store left there.

use std::alloc::{GlobalAlloc, Layout};
use std::process::{Command, Output};
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Mutex, MutexGuard, PoisonError};

use nakkash_machine::blocks::{self, BlockStats};
use nakkash_machine::{BIG, GivesBack, MIB};

#[global_allocator]
static ALLOCATOR: GivesBack = GivesBack;

/// The bytes the store may keep, as the keep function reads them.
static ROOM: AtomicU64 = AtomicU64::new(0);

/// What the keep function does: read [`ROOM`], or allocate, which a keep
/// function must never do.
static MODE: AtomicU64 = AtomicU64::new(READS);

/// The keep function reads [`ROOM`].
const READS: u64 = 0;

/// The keep function allocates, as one must not.
const ALLOCATES: u64 = 1;

/// The keep function every test of this process registers: it reads
/// atomics only, unless a child test asks it to allocate.
fn room() -> u64 {
    match MODE.load(Relaxed) {
        ALLOCATES => std::hint::black_box(vec![1u8; 64]).len() as u64,
        _ => ROOM.load(Relaxed),
    }
}

/// One test at a time, and the store as a test finds it: empty, no
/// poison, nothing kept, the default staleness. Dropped, it leaves the
/// store so again.
struct Turn {
    _turn: MutexGuard<'static, ()>,
}

impl Drop for Turn {
    fn drop(&mut self) {
        ROOM.store(0, Relaxed);
        blocks::settle();
        blocks::set_poison(None);
        blocks::set_stale_ticks(blocks::STALE_TICKS);
    }
}

/// Takes this test's turn with the store keeping up to `room` bytes.
fn turn(room: u64) -> Turn {
    static TURN: Mutex<()> = Mutex::new(());
    let turn = Turn {
        _turn: TURN.lock().unwrap_or_else(PoisonError::into_inner),
    };
    blocks::set_keep(room_fn());
    blocks::set_poison(None);
    blocks::set_stale_ticks(blocks::STALE_TICKS);
    blocks::settle();
    ROOM.store(room, Relaxed);
    turn
}

/// The keep function, as a pointer.
fn room_fn() -> fn() -> u64 {
    room
}

/// A layout of `size` bytes, aligned as bytes are.
fn bytes(size: usize) -> Layout {
    Layout::from_size_align(size, 1).expect("a layout")
}

/// A block of `layout` from the store or the system, through the
/// allocator's own function.
fn alloc(layout: Layout) -> *mut u8 {
    // SAFETY: the layout has a size above 0.
    let block = unsafe { GivesBack.alloc(layout) };
    assert!(!block.is_null(), "{} bytes", layout.size());
    block
}

/// A zeroed block of `layout`.
fn alloc_zeroed(layout: Layout) -> *mut u8 {
    // SAFETY: the layout has a size above 0.
    let block = unsafe { GivesBack.alloc_zeroed(layout) };
    assert!(!block.is_null(), "{} bytes", layout.size());
    block
}

/// Frees `block`, made with `layout`.
fn dealloc(block: *mut u8, layout: Layout) {
    // SAFETY: every caller passes a block `alloc` or `realloc` made with
    // `layout`, and does not use it again.
    unsafe { GivesBack.dealloc(block, layout) };
}

/// `block`, made with `layout`, grown or shrunk to `size`.
fn realloc(block: *mut u8, layout: Layout, size: usize) -> *mut u8 {
    // SAFETY: every caller passes a block made with `layout` and a size
    // above 0, and uses only what this returns afterwards.
    let moved = unsafe { GivesBack.realloc(block, layout, size) };
    assert!(!moved.is_null(), "{size} bytes");
    moved
}

/// Writes `byte` over the first `len` bytes of `block`.
fn fill(block: *mut u8, len: usize, byte: u8) {
    // SAFETY: every caller's block holds at least `len` bytes.
    unsafe { std::ptr::write_bytes(block, byte, len) };
}

/// Writes `i % 251` at every byte `i` of the first `len` bytes of `block`.
fn pattern(block: *mut u8, len: usize) {
    // SAFETY: every caller's block holds at least `len` bytes, and the
    // test alone holds it.
    let bytes = unsafe { std::slice::from_raw_parts_mut(block, len) };
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
}

/// The first `len` bytes of `block`.
fn read(block: *mut u8, len: usize) -> &'static [u8] {
    // SAFETY: every caller's block holds at least `len` bytes, all of them
    // written by the system, the store or the test, and stays held while
    // the slice is read.
    unsafe { std::slice::from_raw_parts(block, len) }
}

/// The counters' change from `before` to now.
fn since(before: BlockStats) -> BlockStats {
    let now = blocks::stats();
    BlockStats {
        mapped: now.mapped - before.mapped,
        reused: now.reused - before.reused,
        unmapped: now.unmapped - before.unmapped,
        stale: now.stale - before.stale,
        kept: now.kept,
        kept_peak: now.kept_peak,
    }
}

/// Runs the child test `name` of this binary in a process of its own,
/// with `env` added and the clobber variable unset unless `env` sets it.
fn child(name: &str, env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(std::env::current_exe().expect("this test binary"));
    command
        .args([
            name,
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_remove("NAKKASH_CLOBBER")
        .env(CHILD, name);
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("the child ran")
}

/// Asserts the child test `name` passed.
fn passes(name: &str, env: &[(&str, &str)]) {
    let out = child(name, env);
    assert!(
        out.status.success(),
        "{name}: {}\n{}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("1 passed"),
        "{name} ran: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// The variable that tells a child test it runs as a child.
const CHILD: &str = "NAKKASH_BLOCKS_CHILD";

/// Whether this process runs the child test `name`.
fn as_child(name: &str) -> bool {
    std::env::var(CHILD).is_ok_and(|child| child == name)
}

/// The bytes of a 1080p grid of float pixels.
const GRID: usize = 1920 * 1080 * 16;

// 1. A zeroed block is zeros, even one the store kept dirty.

#[test]
fn a_zeroed_block_from_the_store_is_all_zeros() {
    let _turn = turn(64 * MIB);
    let dirty = bytes(3 * BIG);
    let block = alloc(dirty);
    fill(block, dirty.size(), 0xAB);
    dealloc(block, dirty);
    assert_eq!(blocks::kept(), 3 * MIB);
    let before = blocks::stats();
    // Smaller, in the same class: the same block, memset over what was asked.
    let asked = bytes(3 * BIG - 1000);
    let zeroed = alloc_zeroed(asked);
    assert_eq!(zeroed, block, "the kept block is handed out again");
    assert_eq!(since(before).reused, 1);
    assert_eq!(since(before).mapped, 0);
    assert!(read(zeroed, asked.size()).iter().all(|&b| b == 0));
    assert_eq!(blocks::kept(), 0);
    dealloc(zeroed, asked);
}

#[test]
fn a_zeroed_block_is_zeros_and_unpoisoned_whether_fresh_or_kept() {
    let _turn = turn(64 * MIB);
    blocks::set_poison(Some(0xFF));
    let layout = bytes(2 * BIG + 3);
    // Fresh: the system's zero pages.
    let fresh = alloc_zeroed(layout);
    assert!(read(fresh, layout.size()).iter().all(|&b| b == 0));
    // Kept, poisoned on free, then asked for zeroed.
    fill(fresh, layout.size(), 0x5A);
    dealloc(fresh, layout);
    let kept = alloc_zeroed(layout);
    assert_eq!(kept, fresh);
    assert!(read(kept, layout.size()).iter().all(|&b| b == 0));
    dealloc(kept, layout);
}

// 2. Poison: every block `alloc` hands out, and every block kept on free.

#[test]
fn poison_fills_a_fresh_block_and_a_kept_one_handed_out_again() {
    let _turn = turn(64 * MIB);
    blocks::set_poison(Some(0x7F));
    let layout = bytes(5 * BIG);
    let before = blocks::stats();
    let fresh = alloc(layout);
    assert_eq!(since(before).mapped, 1);
    assert!(read(fresh, layout.size()).iter().all(|&b| b == 0x7F));
    fill(fresh, layout.size(), 0x11);
    dealloc(fresh, layout);
    let again = alloc(layout);
    assert_eq!(again, fresh);
    assert_eq!(since(before).reused, 1);
    assert!(
        read(again, layout.size()).iter().all(|&b| b == 0x7F),
        "poison, not the old contents"
    );
    dealloc(again, layout);
}

#[test]
fn a_kept_block_is_poisoned_when_it_is_freed() {
    let _turn = turn(64 * MIB);
    let layout = bytes(4 * BIG);
    let block = alloc(layout);
    fill(block, layout.size(), 0x11);
    blocks::set_poison(Some(0xFF));
    dealloc(block, layout);
    // Handed out unpoisoned: what is there is what the free left.
    blocks::set_poison(None);
    let again = alloc(layout);
    assert_eq!(again, block);
    // The store's link may sit in the first bytes; everything after it is
    // the poison of the free.
    assert!(read(again, layout.size())[64..].iter().all(|&b| b == 0xFF));
    dealloc(again, layout);
}

#[test]
fn without_poison_a_kept_block_comes_back_as_it_was_left() {
    let _turn = turn(64 * MIB);
    let layout = bytes(4 * BIG);
    let block = alloc(layout);
    fill(block, layout.size(), 0x33);
    dealloc(block, layout);
    let again = alloc(layout);
    assert_eq!(again, block);
    assert!(read(again, layout.size())[64..].iter().all(|&b| b == 0x33));
    dealloc(again, layout);
}

#[test]
fn the_clobber_variable_sets_the_poison() {
    passes(
        "child_poison_from_the_variable",
        &[("NAKKASH_CLOBBER", "7f")],
    );
    passes(
        "child_poison_from_the_variable",
        &[("NAKKASH_CLOBBER", "1")],
    );
    passes(
        "child_poison_from_the_variable",
        &[("NAKKASH_CLOBBER", "FF")],
    );
    passes("child_poison_from_the_variable", &[]);
    passes(
        "child_poison_set_overrides_the_variable",
        &[("NAKKASH_CLOBBER", "7f")],
    );
}

#[test]
#[ignore = "run in a child process by the_clobber_variable_sets_the_poison"]
fn child_poison_from_the_variable() {
    if !as_child("child_poison_from_the_variable") {
        return;
    }
    let expected = match std::env::var("NAKKASH_CLOBBER").as_deref() {
        Ok("7f") => 0x7F,
        Ok("1" | "FF") => 0xFF,
        Ok(other) => panic!("no case for {other}"),
        Err(_) => 0,
    };
    let layout = bytes(2 * BIG);
    let block = alloc(layout);
    assert!(
        read(block, layout.size()).iter().all(|&b| b == expected),
        "every byte {expected:#04x}"
    );
    dealloc(block, layout);
}

#[test]
#[ignore = "run in a child process by the_clobber_variable_sets_the_poison"]
fn child_poison_set_overrides_the_variable() {
    if !as_child("child_poison_set_overrides_the_variable") {
        return;
    }
    blocks::set_poison(None);
    let layout = bytes(2 * BIG);
    let block = alloc(layout);
    assert!(read(block, layout.size()).iter().all(|&b| b == 0));
    dealloc(block, layout);
}

// 3. No keep function: every free unmaps. A cap: never passed.

#[test]
fn without_a_keep_function_every_freed_block_leaves_the_process() {
    passes("child_without_a_keep_function", &[]);
}

#[test]
#[ignore = "run in a child process by without_a_keep_function_every_freed_block_leaves_the_process"]
fn child_without_a_keep_function() {
    if !as_child("child_without_a_keep_function") {
        return;
    }
    let before = blocks::stats();
    let resident = nakkash_machine::resident();
    for round in 0..2 {
        let grids: Vec<Vec<u8>> = (0..8).map(|i| vec![i as u8 + 1; GRID]).collect();
        assert!(
            grids
                .iter()
                .enumerate()
                .all(|(i, g)| g[GRID - 1] == i as u8 + 1)
        );
        drop(grids);
        assert_eq!(blocks::kept(), 0, "round {round}");
        if let (Some(before), Some(after)) = (resident, nakkash_machine::resident()) {
            assert!(
                after <= before + 16 * MIB,
                "round {round}: {} MiB resident once the grids were freed, {} MiB before",
                after / MIB,
                before / MIB
            );
        }
    }
    let delta = since(before);
    assert!(delta.mapped >= 16, "{delta:?}");
    assert_eq!(delta.reused, 0, "{delta:?}");
    assert_eq!(delta.unmapped, delta.mapped, "{delta:?}");
    assert_eq!((delta.kept, delta.kept_peak), (0, 0), "{delta:?}");
    // Trimming, ticking and settling an empty store does nothing.
    assert_eq!(blocks::trim(0), 0);
    blocks::tick();
    blocks::settle();
    assert_eq!(since(before).unmapped, delta.unmapped);
}

#[test]
fn with_a_room_of_zero_every_freed_block_is_unmapped() {
    let _turn = turn(0);
    let before = blocks::stats();
    let layout = bytes(2 * BIG);
    let block = alloc(layout);
    dealloc(block, layout);
    let delta = since(before);
    assert_eq!((delta.mapped, delta.unmapped, delta.kept), (1, 1, 0));
}

#[test]
fn kept_never_passes_the_cap_under_eight_threads() {
    // A process of its own, so the most ever kept is this run's.
    passes("child_kept_never_passes_the_cap", &[]);
}

#[test]
#[ignore = "run in a child process by kept_never_passes_the_cap_under_eight_threads"]
fn child_kept_never_passes_the_cap() {
    if !as_child("child_kept_never_passes_the_cap") {
        return;
    }
    const CAP: u64 = 16 * MIB;
    const SIZES: [usize; 7] = [
        BIG,
        BIG + 1,
        BIG + BIG / 2,
        2 * BIG,
        3 * BIG + 5,
        5 * BIG,
        8 * BIG,
    ];
    let _turn = turn(CAP);
    let before = blocks::stats();
    std::thread::scope(|scope| {
        for thread in 0..8u64 {
            scope.spawn(move || {
                // A small generator of its own per thread: the same
                // operations every run.
                let mut seed = 0x9E37_79B9_7F4A_7C15 ^ (thread + 1);
                let mut next = move || {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed
                };
                let mut live: Vec<(*mut u8, Layout, u64)> = Vec::with_capacity(4);
                for op in 0..1500u64 {
                    let tag = (thread << 32) | op;
                    let choice = next() % 4;
                    match (choice, live.len()) {
                        (0 | 1, 0..4) | (_, 0) => {
                            let layout = bytes(SIZES[next() as usize % SIZES.len()]);
                            let block = alloc(layout);
                            stamp(block, layout, tag);
                            live.push((block, layout, tag));
                        }
                        (2, _) => {
                            let at = next() as usize % live.len();
                            let (block, layout, tag) = live.swap_remove(at);
                            check(block, layout, tag);
                            dealloc(block, layout);
                        }
                        _ => {
                            let at = next() as usize % live.len();
                            let (block, layout, old) = live[at];
                            check(block, layout, old);
                            let size = SIZES[next() as usize % SIZES.len()];
                            let moved = realloc(block, layout, size);
                            // The prefix is kept: the first stamp survives.
                            assert_eq!(read(moved, 8), &old.to_le_bytes());
                            let layout = bytes(size);
                            stamp(moved, layout, tag);
                            live[at] = (moved, layout, tag);
                        }
                    }
                    let kept = blocks::kept();
                    assert!(kept <= CAP, "{kept} bytes kept, cap {CAP}");
                }
                for (block, layout, tag) in live {
                    check(block, layout, tag);
                    dealloc(block, layout);
                }
            });
        }
    });
    let delta = since(before);
    assert!(delta.kept_peak <= CAP, "{delta:?}");
    assert!(delta.reused > 0, "the store served blocks: {delta:?}");
    assert!(delta.unmapped > 0, "the cap turned blocks away: {delta:?}");
    assert!(blocks::kept() <= CAP);
}

/// Writes `tag` at both ends of `block`.
fn stamp(block: *mut u8, layout: Layout, tag: u64) {
    let size = layout.size();
    // SAFETY: the block holds `size` ≥ 16 bytes.
    let end = unsafe { block.add(size - 8) };
    for at in [block, end] {
        // SAFETY: eight bytes inside the block, written unaligned.
        unsafe { at.cast::<u64>().write_unaligned(tag) };
    }
}

/// Asserts `block` still holds `tag` at both ends: no other thread was
/// handed it while this one held it.
fn check(block: *mut u8, layout: Layout, tag: u64) {
    let size = layout.size();
    assert_eq!(
        read(block, 8),
        &tag.to_le_bytes(),
        "the start of a held block"
    );
    assert_eq!(
        &read(block, size)[size - 8..],
        &tag.to_le_bytes(),
        "the end of a held block"
    );
}

// 4. Trim, settle and stale blocks.

#[test]
fn trim_lets_the_largest_classes_go_first_and_settle_lets_everything_go() {
    let _turn = turn(64 * MIB);
    let layouts = [1, 2, 4, 8].map(|mib| bytes(mib * BIG));
    let held = layouts.map(alloc);
    for (block, layout) in held.into_iter().zip(layouts) {
        dealloc(block, layout);
    }
    assert_eq!(blocks::kept(), 15 * MIB);
    let before = blocks::stats();
    // One block of 8 MiB makes the room, not one of 1 MiB.
    assert_eq!(blocks::trim(14 * MIB), 8 * MIB);
    assert_eq!(blocks::kept(), 7 * MIB);
    assert_eq!(since(before).unmapped, 1);
    // Down to 3 MiB: the 4 MiB block.
    assert_eq!(blocks::trim(3 * MIB), 4 * MIB);
    assert_eq!(blocks::kept(), 3 * MIB);
    // Nothing to do when the store is already inside.
    assert_eq!(blocks::trim(3 * MIB), 0);
    // The rest are kept and handed out again.
    let again = alloc(layouts[1]);
    assert_eq!(since(before).reused, 1);
    dealloc(again, layouts[1]);
    blocks::settle();
    assert_eq!(blocks::kept(), 0);
    assert_eq!(since(before).unmapped, 4);
}

#[test]
fn a_block_idle_past_the_stale_ticks_is_let_go_and_a_reused_one_is_kept() {
    let _turn = turn(64 * MIB);
    blocks::set_stale_ticks(2);
    let layout = bytes(BIG);
    let before = blocks::stats();
    let block = alloc(layout);
    dealloc(block, layout);
    blocks::tick();
    blocks::tick();
    assert_eq!(blocks::kept(), MIB, "idle two ticks: kept");
    blocks::tick();
    assert_eq!(blocks::kept(), 0, "idle three ticks: let go");
    assert_eq!(since(before).stale, 1);
    // Reused within the ticks, it starts its idle time again.
    let block = alloc(layout);
    dealloc(block, layout);
    blocks::tick();
    blocks::tick();
    let again = alloc(layout);
    assert_eq!(again, block);
    dealloc(again, layout);
    blocks::tick();
    blocks::tick();
    assert_eq!(blocks::kept(), MIB, "reused within the ticks: kept");
    blocks::tick();
    assert_eq!(blocks::kept(), 0);
    assert_eq!(since(before).stale, 2);
}

#[test]
fn the_default_stale_ticks_keep_a_block_that_long() {
    let _turn = turn(64 * MIB);
    let layout = bytes(BIG);
    let block = alloc(layout);
    dealloc(block, layout);
    for _ in 0..blocks::STALE_TICKS {
        blocks::tick();
    }
    assert_eq!(blocks::kept(), MIB);
    blocks::tick();
    assert_eq!(blocks::kept(), 0);
}

// 5. realloc with the store on.

#[test]
fn realloc_within_a_class_keeps_the_block_in_place() {
    let _turn = turn(64 * MIB);
    let small = bytes(BIG + BIG / 2 - 60_000);
    let block = alloc(small);
    pattern(block, small.size());
    let before = blocks::stats();
    let grown = realloc(block, small, BIG + BIG / 2);
    assert_eq!(grown, block, "the same class: in place");
    let delta = since(before);
    assert_eq!((delta.mapped, delta.unmapped, delta.reused), (0, 0, 0));
    let kept = read(grown, small.size());
    assert!(kept.iter().enumerate().all(|(i, &b)| b == (i % 251) as u8));
    dealloc(grown, bytes(BIG + BIG / 2));
}

#[test]
fn realloc_across_classes_keeps_the_prefix_and_takes_from_the_store() {
    let _turn = turn(64 * MIB);
    let first = bytes(BIG + 7);
    let block = alloc(first);
    pattern(block, first.size());
    let holds = |block: *mut u8, len: usize| {
        read(block, len)
            .iter()
            .enumerate()
            .all(|(i, &b)| b == (i % 251) as u8)
    };
    // Grown with nothing of the new class kept: from the system.
    let grown = realloc(block, first, 5 * BIG);
    assert!(holds(grown, first.size()));
    // A block of the next size kept, then grown into it: from the store.
    let spare = alloc(bytes(9 * BIG));
    dealloc(spare, bytes(9 * BIG));
    let before = blocks::stats();
    let grown = realloc(grown, bytes(5 * BIG), 9 * BIG);
    assert_eq!(grown, spare, "the kept block of the new class");
    assert_eq!(since(before).reused, 1);
    assert!(holds(grown, first.size()));
    // Shrunk to another class, then below the size the store serves.
    let shrunk = realloc(grown, bytes(9 * BIG), 2 * BIG);
    assert!(holds(shrunk, first.size()));
    let tiny = realloc(shrunk, bytes(2 * BIG), 1000);
    assert!(holds(tiny, 1000));
    dealloc(tiny, bytes(1000));
}

#[test]
fn realloc_across_classes_poisons_past_the_prefix() {
    let _turn = turn(64 * MIB);
    blocks::set_poison(Some(0x7F));
    let first = bytes(BIG + 7);
    let block = alloc(first);
    fill(block, first.size(), 0x01);
    let grown = realloc(block, first, 3 * BIG);
    let bytes_now = read(grown, 3 * BIG);
    assert!(bytes_now[..first.size()].iter().all(|&b| b == 0x01));
    assert!(bytes_now[first.size()..].iter().all(|&b| b == 0x7F));
    dealloc(grown, bytes(3 * BIG));
}

#[test]
fn vectors_keep_their_bytes_as_they_grow_and_shrink_with_the_store_on() {
    let _turn = turn(256 * MIB);
    for round in 0..2 {
        let zeros = vec![0u8; 3 * BIG];
        assert!(zeros.iter().all(|&b| b == 0), "round {round}");
        drop(zeros);
        let mut grown = Vec::new();
        for i in 0..3 * BIG + 7 {
            grown.push((i % 251) as u8);
        }
        assert!(grown.iter().enumerate().all(|(i, &b)| b == (i % 251) as u8));
        grown.reserve_exact(5 * BIG);
        assert!(grown.capacity() >= 8 * BIG);
        assert!(grown.iter().enumerate().all(|(i, &b)| b == (i % 251) as u8));
        grown.truncate(1000);
        grown.shrink_to_fit();
        assert!(grown.capacity() < BIG);
        assert!(grown.iter().enumerate().all(|(i, &b)| b == (i % 251) as u8));
    }
    assert!(blocks::stats().reused > 0);
}

// 6. Size classes, as the allocator sees them: a block freed with one size
// serves any size of its class, and none of another.

#[test]
fn a_freed_block_serves_every_size_of_its_class_and_no_other() {
    let _turn = turn(64 * MIB);
    let one = bytes(BIG + BIG / 2);
    let block = alloc(one);
    dealloc(block, one);
    // 1.5 MiB is a class's top: one byte more is the next class.
    let before = blocks::stats();
    let other = alloc(bytes(BIG + BIG / 2 + 1));
    assert_ne!(other, block);
    assert_eq!(since(before).reused, 0);
    dealloc(other, bytes(BIG + BIG / 2 + 1));
    let same = alloc(bytes(BIG + BIG / 2 - 65_535));
    assert_eq!(same, block);
    assert_eq!(since(before).reused, 1);
    dealloc(same, bytes(BIG + BIG / 2 - 65_535));
    // The ledger counts class bytes: 1.5 MiB kept, whatever size freed it.
    assert_eq!(blocks::kept(), MIB + MIB / 2 + (MIB + MIB / 2 + MIB / 16));
}

#[test]
fn blocks_below_big_or_aligned_past_a_page_never_reach_the_store() {
    let _turn = turn(64 * MIB);
    let before = blocks::stats();
    let small = vec![7u8; BIG - 1];
    drop(small);
    #[repr(align(8192))]
    struct Paged([u8; 8192]);
    let paged: Vec<Paged> = (0..BIG / 8192 + 1).map(|_| Paged([9; 8192])).collect();
    assert!(paged.iter().all(|p| p.0[8191] == 9));
    drop(paged);
    let delta = since(before);
    assert_eq!((delta.mapped, delta.unmapped, delta.kept), (0, 0, 0));
}

// 7. Nothing allocates inside the allocator.

#[cfg(debug_assertions)]
#[test]
fn a_keep_function_that_allocates_stops_the_process() {
    let out = child("child_keep_function_allocates", &[]);
    assert!(!out.status.success(), "the child stopped");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("the allocator was entered from inside itself"),
        "{said}"
    );
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("reached"),
        "stopped at the free"
    );
}

#[test]
#[ignore = "run in a child process by a_keep_function_that_allocates_stops_the_process"]
fn child_keep_function_allocates() {
    if !as_child("child_keep_function_allocates") {
        return;
    }
    blocks::set_keep(room_fn());
    ROOM.store(64 * MIB, Relaxed);
    MODE.store(ALLOCATES, Relaxed);
    let layout = bytes(2 * BIG);
    let block = alloc(layout);
    // The free asks the keep function for the room, which allocates.
    dealloc(block, layout);
    println!("reached");
}

#[test]
fn the_first_keep_function_set_stays() {
    let _turn = turn(8 * MIB);
    fn never() -> u64 {
        u64::MAX
    }
    blocks::set_keep(never);
    let layout = bytes(10 * BIG);
    let block = alloc(layout);
    dealloc(block, layout);
    assert_eq!(
        blocks::kept(),
        0,
        "the first function's 8 MiB, not the second's"
    );
}
