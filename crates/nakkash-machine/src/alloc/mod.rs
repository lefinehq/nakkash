//! An allocator that gives a big block back to the system when it is freed
//! ([D90]), for a program to install as its global allocator, with a store
//! that may keep freed blocks for reuse inside a cap.
//!
//! A render holds its memory budget by letting grids go, and the budget
//! counts on what is let go leaving the process. The system allocators do
//! not always do that: on macOS the default allocator keeps freed blocks
//! of a 1080p grid's size (31.6 MiB) resident and charged to the process,
//! however many (1.24 GiB of them, measured on macOS 26), and glibc serves
//! blocks below its moving mapping threshold, up to 32 MiB, from a heap it
//! gives back only from the top. [`GivesBack`] maps every block of at
//! least [`BIG`] bytes from the system on its own, at the bytes of its size
//! class ([`class`]), and unmaps it when it is freed, so what the process
//! holds resident is what it uses; smaller blocks go to the system
//! allocator, which reuses them itself.
//!
//! A freed big block may instead be kept by the block store ([`store`],
//! public as `nakkash_machine::blocks`) and handed out again, when a keep
//! function registered with the store has room for it, so a program that
//! frees and asks for blocks of the same sizes frame after frame does not
//! make the system fault in and zero fresh pages each time. With no keep
//! function, the store keeps nothing and every freed block is unmapped.
//!
//! A block it maps is zeroed by the system, so a zeroed allocation costs
//! nothing more; a kept block is set to zero only when zeros are asked
//! for. A block's contents are never read before they are written, by its
//! contract, so nothing a program computes changes. The store's module
//! documentation has the table of what each request holds.
//!
//! Nothing inside `alloc`, `dealloc` or `realloc` allocates: the store
//! links kept blocks through their own first bytes and guards each class
//! with a spin lock. In debug builds, a thread that enters the allocator
//! from inside it (a keep function that allocates) stops the process with
//! a message, since a panic may not unwind out of an allocator.
//!
//! [D90]: https://github.com/lefinehq/nakkash/blob/main/docs/design/decisions.md#d90-a-frame-is-drawn-from-a-plan-that-knows-what-each-drawing-is-under-one-memory-budget

mod class;
pub mod store;

use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(debug_assertions)]
use std::cell::Cell;

use class::{class_bytes, class_of};

/// The fewest bytes a block has for [`GivesBack`] to map it on its own: a
/// mebibyte, the size of the smallest grid the memory budget pools.
pub const BIG: usize = 1 << 20;

/// The global allocator that maps every block of at least [`BIG`] bytes
/// from the system on its own and gives it back when it is freed, unless
/// the block store keeps it, so what the process holds resident is what it
/// uses; smaller blocks go to the system allocator ([D90]). Installed with
/// `#[global_allocator] static ALLOCATOR: GivesBack = GivesBack;`.
///
/// [D90]: https://github.com/lefinehq/nakkash/blob/main/docs/design/decisions.md#d90-a-frame-is-drawn-from-a-plan-that-knows-what-each-drawing-is-under-one-memory-budget
#[derive(Debug, Clone, Copy, Default)]
pub struct GivesBack;

/// The smallest memory page of the systems it maps on: a mapping is
/// aligned to at least this.
#[cfg(unix)]
const PAGE_LEAST: usize = 4096;

/// Who serves a layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    /// The system allocator: a small block, or one aligned past a page.
    System,
    /// A mapping of this size class's bytes, from the store or the system.
    Class(usize),
}

/// Who serves `layout`: a size class when it is big, and aligned to no
/// more than a page, which a mapping always is; else the system allocator.
#[cfg(unix)]
fn route(layout: Layout) -> Route {
    match layout.size() >= BIG && layout.align() <= PAGE_LEAST {
        true => Route::Class(class_of(layout.size())),
        false => Route::System,
    }
}

#[cfg(not(unix))]
fn route(_: Layout) -> Route {
    Route::System
}

// SAFETY: every block is either the system allocator's, for layouts
// `route` gives the system, or a private anonymous mapping of its size
// class's bytes, at least its size and page-aligned, so aligned to the
// layout. `route` reads only the layout, so a block is always freed the way
// it was made: to the system allocator, or to the store, which keeps a
// mapping of a class only to hand it out for that class, and unmaps it with
// its class's bytes. No block is handed out twice: the store hands out a
// kept block once, under its class's lock.
unsafe impl GlobalAlloc for GivesBack {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _inside = Inside::enter();
        // SAFETY: the caller's layout, as `GlobalAlloc::alloc` requires.
        unsafe { alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let _inside = Inside::enter();
        // SAFETY: the caller's layout, as `GlobalAlloc::alloc_zeroed`
        // requires.
        unsafe { alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _inside = Inside::enter();
        // SAFETY: the caller's block and layout, as `GlobalAlloc::dealloc`
        // requires.
        unsafe { dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _inside = Inside::enter();
        // SAFETY: the caller's block, layout and size, as
        // `GlobalAlloc::realloc` requires.
        unsafe { realloc(ptr, layout, new_size) }
    }
}

/// A block of `layout`: the system allocator's, or a block of its class
/// kept by the store or mapped fresh, filled with poison when it is set.
///
/// # Safety
/// As [`GlobalAlloc::alloc`].
unsafe fn alloc(layout: Layout) -> *mut u8 {
    match route(layout) {
        // SAFETY: the caller's layout.
        Route::System => unsafe { System.alloc(layout) },
        Route::Class(class) => {
            let block = store::take(class).unwrap_or_else(|| store::fresh(class));
            if let (Some(byte), false) = (store::poison(), block.is_null()) {
                // SAFETY: a block of the class's bytes, held by no one yet.
                unsafe { fill(block, class_bytes(class), byte) };
            }
            block
        }
    }
}

/// A block of `layout` that reads as zeros: the system allocator's, a kept
/// block set to zero over the size asked for, or a fresh mapping, which
/// the system zeroes.
///
/// # Safety
/// As [`GlobalAlloc::alloc_zeroed`].
unsafe fn alloc_zeroed(layout: Layout) -> *mut u8 {
    match route(layout) {
        // SAFETY: the caller's layout.
        Route::System => unsafe { System.alloc_zeroed(layout) },
        Route::Class(class) => match store::take(class) {
            Some(block) => {
                // SAFETY: a kept block of the class's bytes, at least the
                // size asked for, held by no one yet.
                unsafe { fill(block, layout.size(), 0) };
                block
            }
            // A fresh anonymous mapping reads as zeros.
            None => store::fresh(class),
        },
    }
}

/// Frees `ptr`, made with `layout`: to the system allocator, or to the
/// store, which keeps or unmaps it.
///
/// # Safety
/// As [`GlobalAlloc::dealloc`].
unsafe fn dealloc(ptr: *mut u8, layout: Layout) {
    match route(layout) {
        // SAFETY: the system allocator made `ptr` with this layout.
        Route::System => unsafe { System.dealloc(ptr, layout) },
        // SAFETY: `ptr` is a mapping of the class's bytes, as the caller
        // made it with this layout, and is not used again.
        Route::Class(class) => unsafe { store::release(ptr, class) },
    }
}

/// `ptr`, made with `layout`, grown or shrunk to `new_size`: in place
/// within its class; moved by the system on Linux when the store keeps
/// nothing of the new class; else copied into a new block.
///
/// # Safety
/// As [`GlobalAlloc::realloc`].
unsafe fn realloc(ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    // SAFETY: the caller passes a size that, rounded up to the alignment,
    // does not overflow, as `GlobalAlloc::realloc` requires.
    let new = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
    match (route(layout), route(new)) {
        // SAFETY: the system allocator made `ptr` with `layout`, and keeps
        // it if the new layout is too big for it.
        (Route::System, Route::System) => unsafe { System.realloc(ptr, layout, new_size) },
        // The class's bytes hold the new size too.
        (Route::Class(from), Route::Class(to)) if from == to => ptr,
        #[cfg(target_os = "linux")]
        (Route::Class(from), Route::Class(to)) if !store::has(to) => {
            // SAFETY: `ptr` is a mapping of `from`'s bytes.
            unsafe { remapped(ptr, from, to, layout.size().min(new_size)) }
        }
        // SAFETY: the caller's block and layout, and a valid new layout.
        _ => unsafe { moved(ptr, layout, new) },
    }
}

/// `ptr`, made with `layout`, copied into a new block of `new` from the
/// store or the system, and freed; null, with `ptr` kept, if there is no
/// new block.
///
/// # Safety
/// `ptr` was made by this allocator with `layout`, and `new` is a valid
/// layout of a size above 0; on success `ptr` is not used again.
unsafe fn moved(ptr: *mut u8, layout: Layout, new: Layout) -> *mut u8 {
    // SAFETY: `new` is a valid layout of a size above 0.
    let to = unsafe { alloc(new) };
    if !to.is_null() {
        let kept = layout.size().min(new.size());
        // SAFETY: both blocks hold at least `kept` bytes, and a new block
        // does not overlap one still held.
        unsafe { std::ptr::copy_nonoverlapping(ptr, to, kept) };
        // SAFETY: `ptr` was made by this allocator with `layout`, and is not
        // used again.
        unsafe { dealloc(ptr, layout) };
    }
    to
}

/// The mapping of class `from` at `ptr`, moved by the system to class
/// `to`'s bytes, its first `kept` bytes kept and the rest filled with
/// poison when it is set, as a block from `alloc` would be; null, with the
/// old mapping kept, if the system cannot.
///
/// # Safety
/// `ptr` is a mapping of `from`'s bytes this allocator made, holding at
/// least `kept` bytes; on success it is not used again.
#[cfg(target_os = "linux")]
unsafe fn remapped(ptr: *mut u8, from: usize, to: usize, kept: usize) -> *mut u8 {
    let bytes = class_bytes(to);
    // SAFETY: the caller's mapping of `from`'s bytes.
    let moved = unsafe { remap(ptr, class_bytes(from), bytes) };
    if !moved.is_null() {
        store::count_moved();
        if let Some(byte) = store::poison() {
            // SAFETY: `kept` is at most the new class's bytes.
            let past = unsafe { moved.add(kept) };
            // SAFETY: the new mapping holds `bytes`, so `bytes − kept` past
            // the kept ones, held by no one else.
            unsafe { fill(past, bytes - kept, byte) };
        }
    }
    moved
}

/// Writes `byte` over the first `len` bytes of `block`.
///
/// # Safety
/// `block` is valid for writes of `len` bytes, and nothing else reads or
/// writes them meanwhile.
unsafe fn fill(block: *mut u8, len: usize, byte: u8) {
    // SAFETY: the caller's block, valid for `len` bytes.
    unsafe { std::ptr::write_bytes(block, byte, len) };
}

/// A fresh private anonymous mapping of `size` bytes, read and written;
/// null if the system has none.
#[cfg(unix)]
fn map(size: usize) -> *mut u8 {
    // SAFETY: an anonymous mapping at an address the system picks touches
    // no memory the program holds.
    let at = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        )
    };
    match at == libc::MAP_FAILED {
        true => std::ptr::null_mut(),
        false => at.cast(),
    }
}

#[cfg(not(unix))]
fn map(_: usize) -> *mut u8 {
    std::ptr::null_mut()
}

/// Gives the mapping of `size` bytes at `ptr` back to the system.
///
/// # Safety
/// `ptr` and `size` are a mapping [`map`] made, not used again.
#[cfg(unix)]
unsafe fn unmap(ptr: *mut u8, size: usize) {
    // SAFETY: the caller's mapping, which nothing reads again. Unmapping a
    // mapping the process made cannot fail but for arguments it did not
    // make, so the result says nothing to act on.
    let _ = unsafe { libc::munmap(ptr.cast(), size) };
}

#[cfg(not(unix))]
unsafe fn unmap(_: *mut u8, _: usize) {}

/// The mapping of `old` bytes at `ptr` grown or shrunk to `new`, moved if
/// it must be; null, with the old mapping kept, if the system cannot.
///
/// # Safety
/// `ptr` and `old` are a mapping [`map`] made; on success it is not used
/// again.
#[cfg(target_os = "linux")]
unsafe fn remap(ptr: *mut u8, old: usize, new: usize) -> *mut u8 {
    // SAFETY: the caller's mapping; the system moves its pages, so no
    // byte is lost or read twice.
    let at = unsafe { libc::mremap(ptr.cast(), old, new, libc::MREMAP_MAYMOVE) };
    match at == libc::MAP_FAILED {
        true => std::ptr::null_mut(),
        false => at.cast(),
    }
}

#[cfg(debug_assertions)]
thread_local! {
    /// Whether this thread runs inside the allocator now (debug builds
    /// only).
    static INSIDE: Cell<bool> = const { Cell::new(false) };
}

/// Marks `inside` entered and says whether it already was: an entry from
/// inside.
#[cfg(debug_assertions)]
fn entered(inside: &Cell<bool>) -> bool {
    inside.replace(true)
}

/// This thread inside the allocator, until dropped. In debug builds, an
/// entry while already inside stops the process; in release builds it is
/// nothing.
struct Inside;

impl Inside {
    /// Enters the allocator on this thread.
    fn enter() -> Inside {
        #[cfg(debug_assertions)]
        INSIDE.with(|inside| {
            if entered(inside) {
                reentered()
            }
        });
        Inside
    }
}

impl Drop for Inside {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        INSIDE.with(|inside| inside.set(false));
    }
}

/// Stops the process, in debug builds, if this thread is inside the
/// allocator: the store's trim, tick and settle run from a caller's code,
/// never from inside the allocator.
fn outside() {
    #[cfg(debug_assertions)]
    INSIDE.with(|inside| {
        if inside.get() {
            reentered()
        }
    });
}

/// What a debug build says when a thread enters the allocator from inside
/// it.
#[cfg_attr(
    not(debug_assertions),
    allow(dead_code, reason = "said in debug builds, pinned by a test in both")
)]
const REENTERED: &[u8] = b"nakkash-machine: the allocator was entered from inside itself: \
a keep function allocated, or the block store was trimmed from inside the allocator\n";

/// Says [`REENTERED`] on the standard error, without allocating, and stops
/// the process: a panic would allocate, and may not unwind out of an
/// allocator.
#[cfg(debug_assertions)]
fn reentered() -> ! {
    #[cfg(unix)]
    // SAFETY: writes the bytes of a static to the standard error. A failed
    // write leaves nothing to do but stop, which follows.
    let _ = unsafe {
        libc::write(
            libc::STDERR_FILENO,
            REENTERED.as_ptr().cast(),
            REENTERED.len(),
        )
    };
    std::process::abort()
}

#[cfg(test)]
#[path = "../../tests/unit/alloc/mod.rs"]
mod tests;
