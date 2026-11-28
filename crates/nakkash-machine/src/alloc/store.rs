//! The store of freed big blocks inside [`GivesBack`]: it keeps a freed
//! block of [`BIG`] bytes or more, by size class, while its keep function
//! allows, and hands it out again instead of asking the system for fresh
//! pages, which the system must fault in and zero one by one.
//!
//! Re-exported as `nakkash_machine::blocks`. With no keep function the
//! store keeps nothing, and every freed block leaves the process as soon
//! as it is freed, as [`GivesBack`] always did ([D90]).
//!
//! # What it keeps
//!
//! - **The cap.** [`set_keep`] registers the function that says how many
//!   bytes the store may hold. A freed block is kept only if a
//!   compare-and-swap can raise [`kept`] by the block's class bytes without
//!   passing what that function returns, so the store never holds more,
//!   on any number of threads.
//! - **Classes.** Sixteen to an octave from 1 MiB, so a freed 1080p pixmap
//!   serves a 1080p mask of the same bytes. Each class has a stack of its
//!   blocks, newest on top, linked through the blocks' own first bytes and
//!   guarded by a spin lock, so keeping and handing out never allocate.
//! - **Stale blocks.** [`tick`] marks a frame's start. A block left in the
//!   store for more than [`STALE_TICKS`] ticks (or what
//!   [`set_stale_ticks`] set) is let go then, so the store keeps what the
//!   next frames ask for, not all the cap allows.
//! - **Trim and settle.** [`trim`] lets blocks go, largest class first,
//!   until the store holds no more than asked; [`settle`] lets every block
//!   go at the end of a job. Both run from the caller's code, never inside
//!   the allocator, and give blocks back with `munmap`.
//!
//! # What a block holds
//!
//! By the allocator's contract, a block from `alloc` holds anything and a
//! block from `alloc_zeroed` holds zeros, so a kept block is zeroed only
//! when zeros are asked for. Poison ([`set_poison`]) fills blocks with a
//! byte, so code that reads what it never wrote shows:
//!
//! | Request | Fresh mapping | Block from the store | Poison set |
//! |---|---|---|---|
//! | `alloc` | the system's zero pages | as it was left | every byte of the block filled with poison, fresh or kept |
//! | `alloc_zeroed` | the system's zero pages, not written | set to zero over the size asked for | never poisoned |
//! | `realloc`, same class | – | in place: the bytes asked to be kept are kept, the rest is undefined | – |
//! | `realloc`, another class | a new block, then the kept bytes copied | as `alloc`, then the kept bytes copied | as `alloc`: past the kept bytes, poison |
//! | `dealloc` | – | kept if the cap allows it, else unmapped | a kept block filled with poison |
//!
//! [D90]: https://github.com/lefinehq/nakkash/blob/main/docs/design/decisions.md#d90-a-frame-is-drawn-from-a-plan-that-knows-what-each-drawing-is-under-one-memory-budget

use std::ptr::NonNull;
use std::sync::OnceLock;
use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed};
use std::sync::atomic::{AtomicU16, AtomicU64};

use super::chain::{Chain, Node};
use super::class::{CLASSES, class_bytes};
use super::spin::Spin;
#[cfg(doc)]
use super::{BIG, GivesBack};

/// How many ticks a block may stay in the store unused before [`tick`]
/// lets it go, until [`set_stale_ticks`] says otherwise: twice the frames
/// drawn at once, for two frames at once, the job [D90]'s default budget
/// was sized for. A caller that knows how many frames it draws at once
/// sets twice that.
///
/// [D90]: https://github.com/lefinehq/nakkash/blob/main/docs/design/decisions.md#d90-a-frame-is-drawn-from-a-plan-that-knows-what-each-drawing-is-under-one-memory-budget
pub const STALE_TICKS: u64 = 4;

/// What the store has done since the process started, and what it holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlockStats {
    /// Blocks the system mapped fresh, a block moved by the system to
    /// another class (Linux's `mremap`) included.
    pub mapped: u64,
    /// Blocks handed out again from the store.
    pub reused: u64,
    /// Blocks given back to the system, a block moved to another class
    /// included: freed beyond the cap, trimmed, settled or let go stale.
    pub unmapped: u64,
    /// Of those, the blocks let go because they stayed unused for more than
    /// the stale ticks.
    pub stale: u64,
    /// The bytes the store holds now.
    pub kept: u64,
    /// The most bytes the store held at once.
    pub kept_peak: u64,
}

/// Registers the function that says how many bytes the store may keep.
/// The first function registered stays; later calls change nothing.
///
/// The allocator calls it on every free of a big block, from inside
/// itself, so it must read atomics only: it must never allocate, lock,
/// call into the system or panic (a panic may not unwind out of an
/// allocator, and would allocate). In debug builds, an allocation from it,
/// or a call to [`trim`], [`tick`] or [`settle`], stops the process with a
/// message.
pub fn set_keep(room: fn() -> u64) {
    // Taken only if none was set: the first one stays.
    let _ = KEEP.set(room);
}

/// The bytes the store holds now: the class bytes of every block it keeps.
pub fn kept() -> u64 {
    STATS.kept.load(Acquire)
}

/// Lets kept blocks go, largest class first (the fewest blocks for the
/// room made), until the store holds at most `to` bytes; returns the bytes
/// let go. Blocks being kept or handed out by other threads at the same
/// time may leave it above `to` until they are done.
pub fn trim(to: u64) -> u64 {
    super::outside();
    let mut let_go = 0;
    for class in (0..CLASSES).rev() {
        while kept() > to {
            let Some(node) = FREE[class].lock().pop() else {
                break;
            };
            // SAFETY: popped from its class's chain, so a kept block of
            // that class that nothing else holds.
            let_go += unsafe { unmap(node, class) };
        }
    }
    let_go
}

/// Marks the start of a frame: advances the store's clock by a tick and
/// lets go every block left unused for more than the stale ticks.
pub fn tick() {
    super::outside();
    let now = EPOCH.fetch_add(1, Relaxed) + 1;
    let after = STALE_AFTER.load(Relaxed);
    for (class, free) in FREE.iter().enumerate() {
        let mut gone = free.lock().split_off(|node| stale(now, node.freed, after));
        while let Some(node) = gone.pop() {
            // SAFETY: split off its class's chain, so a kept block of that
            // class that nothing else holds.
            unsafe { unmap(node, class) };
            STATS.stale.fetch_add(1, Relaxed);
        }
    }
}

/// Lets every kept block go, at the end of a job, so a process that lives
/// on holds nothing between jobs.
pub fn settle() {
    trim(0);
}

/// Sets how many ticks a block may stay unused in the store before
/// [`tick`] lets it go ([`STALE_TICKS`] until set).
pub fn set_stale_ticks(ticks: u64) {
    STALE_AFTER.store(ticks, Relaxed);
}

/// Sets the byte that fills every block `alloc` hands out and every block
/// the store keeps, or none; none until set. The store reads no variable of
/// the environment: the program that wants poison sets it here, outside the
/// allocator.
pub fn set_poison(byte: Option<u8>) {
    POISON.store(byte.map_or(NO_POISON, u16::from), Relaxed);
}

/// What the store has done and what it holds.
pub fn stats() -> BlockStats {
    BlockStats {
        mapped: STATS.mapped.load(Relaxed),
        reused: STATS.reused.load(Relaxed),
        unmapped: STATS.unmapped.load(Relaxed),
        stale: STATS.stale.load(Relaxed),
        kept: STATS.kept.load(Relaxed),
        kept_peak: STATS.kept_peak.load(Relaxed),
    }
}

/// A kept block of `class`, taken off its stack, or `None` when the store
/// keeps none of it.
pub(super) fn take(class: usize) -> Option<*mut u8> {
    let node = FREE[class].lock().pop()?;
    STATS.kept.fetch_sub(class_bytes(class) as u64, AcqRel);
    STATS.reused.fetch_add(1, Relaxed);
    Some(node.as_ptr().cast())
}

/// A fresh mapping of `class`'s bytes, counted; null if the system has
/// none.
pub(super) fn fresh(class: usize) -> *mut u8 {
    let block = super::map(class_bytes(class));
    count_mapped(!block.is_null());
    block
}

/// Counts a block the system mapped, when it did.
fn count_mapped(did: bool) {
    STATS.mapped.fetch_add(u64::from(did), Relaxed);
}

/// Counts a block of one class the system moved to another: one block
/// unmapped, one mapped.
#[cfg(target_os = "linux")]
pub(super) fn count_moved() {
    STATS.unmapped.fetch_add(1, Relaxed);
    STATS.mapped.fetch_add(1, Relaxed);
}

/// Whether the store keeps a block of `class`.
#[cfg(target_os = "linux")]
pub(super) fn has(class: usize) -> bool {
    !FREE[class].lock().is_empty()
}

/// Keeps the freed `block` of `class` if the cap has room for it, filled
/// with poison when it is set; else gives it back to the system.
///
/// # Safety
/// `block` is a mapping of `class`'s bytes that [`fresh`] or the store
/// handed out, freed by its holder and not used again.
pub(super) unsafe fn release(block: *mut u8, class: usize) {
    let bytes = class_bytes(class);
    match reserve(bytes as u64) {
        false => {
            // SAFETY: the caller's block, a mapping of `bytes`, not used
            // again.
            unsafe { super::unmap(block, bytes) };
            STATS.unmapped.fetch_add(1, Relaxed);
        }
        true => {
            if let Some(byte) = poison() {
                // SAFETY: the caller's block holds `bytes` and nothing else
                // holds it now.
                unsafe { super::fill(block, bytes, byte) };
            }
            // SAFETY: the block is page-aligned and holds far more than a
            // node, and the store alone holds it from here.
            let node = unsafe { Node::at(block, EPOCH.load(Relaxed)) };
            // SAFETY: a node no chain holds, valid until it is popped.
            unsafe { FREE[class].lock().push(node) };
        }
    }
}

/// Raises [`kept`] by `bytes` if that stays inside the keep function's
/// room, and says whether it did.
fn reserve(bytes: u64) -> bool {
    let room = room();
    match STATS
        .kept
        .fetch_update(AcqRel, Acquire, |kept| raised(kept, bytes, room))
    {
        Ok(before) => {
            STATS.kept_peak.fetch_max(before + bytes, Relaxed);
            true
        }
        Err(_) => false,
    }
}

/// What the store may keep now: what the keep function says, or 0 when
/// none is registered.
fn room() -> u64 {
    KEEP.get().map_or(0, |room| room())
}

/// `kept` raised by `bytes`, if that is still inside `room`.
fn raised(kept: u64, bytes: u64, room: u64) -> Option<u64> {
    kept.checked_add(bytes).filter(|&raised| raised <= room)
}

/// Whether a block freed at tick `freed` has stayed unused for more than
/// `after` ticks at tick `now`.
fn stale(now: u64, freed: u64, after: u64) -> bool {
    now.saturating_sub(freed) > after
}

/// Gives a kept block of `class` back to the system, uncounts it, and
/// returns its bytes.
///
/// # Safety
/// `node` heads a kept block of `class` that no chain and no holder has.
unsafe fn unmap(node: NonNull<Node>, class: usize) -> u64 {
    let bytes = class_bytes(class);
    // SAFETY: the caller's block, a mapping of `bytes`, not used again.
    unsafe { super::unmap(node.as_ptr().cast(), bytes) };
    STATS.unmapped.fetch_add(1, Relaxed);
    STATS.kept.fetch_sub(bytes as u64, AcqRel);
    bytes as u64
}

/// The poison byte, or `None`.
pub(super) fn poison() -> Option<u8> {
    u8::try_from(POISON.load(Relaxed)).ok()
}

/// [`POISON`] when no poison is set.
const NO_POISON: u16 = 0x100;

/// The poison: a byte (0–255) or [`NO_POISON`].
static POISON: AtomicU16 = AtomicU16::new(NO_POISON);

/// The keep function; none until [`set_keep`].
static KEEP: OnceLock<fn() -> u64> = OnceLock::new();

/// The store's clock: [`tick`]s so far.
static EPOCH: AtomicU64 = AtomicU64::new(0);

/// The stale ticks: how long a block may stay unused in the store.
static STALE_AFTER: AtomicU64 = AtomicU64::new(STALE_TICKS);

/// Each class's stack of kept blocks.
static FREE: [Spin<Chain>; CLASSES] = [const { Spin::new(Chain::new()) }; CLASSES];

/// The store's counters.
static STATS: Stats = Stats {
    mapped: AtomicU64::new(0),
    reused: AtomicU64::new(0),
    unmapped: AtomicU64::new(0),
    stale: AtomicU64::new(0),
    kept: AtomicU64::new(0),
    kept_peak: AtomicU64::new(0),
};

/// The store's counters, as [`BlockStats`] reads them.
struct Stats {
    mapped: AtomicU64,
    reused: AtomicU64,
    unmapped: AtomicU64,
    stale: AtomicU64,
    kept: AtomicU64,
    kept_peak: AtomicU64,
}

#[cfg(test)]
#[path = "../../tests/unit/alloc/store.rs"]
mod tests;
