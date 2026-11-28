//! A loaded shared library named by its content: which of the process's
//! loaded objects holds an address, and the hash of the segments it was
//! loaded from that the process never writes. Kept apart from asking the
//! dynamic loader (`loader.rs`, Linux only), so every platform tests it.
//!
//! The hash covers the library as the process runs it, not the file on
//! disk now: a library a package update replaced after the process loaded
//! it is still named by the code the process calls.

use nakkash_hash::ContentHasher;

/// The tag of a loaded library's content hash.
const TAG: &str = "nakkash.machine.library";

/// One segment of a loaded object, as its program header describes it
/// (an ELF `Phdr`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Segment {
    /// Whether the loader maps it into memory (`PT_LOAD`); the other
    /// headers describe parts of mapped segments, or none.
    pub(crate) loaded: bool,
    /// Where it starts among the object's addresses (`p_vaddr`), to which
    /// the address the object was loaded at is added.
    pub(crate) start: usize,
    /// How long it is in memory (`p_memsz`).
    pub(crate) memory: usize,
    /// How many of its bytes come from the file (`p_filesz`); the rest of
    /// `memory` is zeroed.
    pub(crate) file: usize,
    /// Whether the process may read it (`PF_R`): a segment mapped only
    /// to run (execute-only) cannot be read, and is not hashed.
    pub(crate) readable: bool,
    /// Whether the process may write it (`PF_W`): data that the loader
    /// relocates to the addresses of this run, and that the process
    /// changes as it runs.
    pub(crate) writable: bool,
}

impl Segment {
    /// Whether its bytes are part of the library's content: mapped,
    /// readable and never written, so the same in every process that loads
    /// the same library. Code, constants and the symbol tables are; data is
    /// not.
    pub(crate) fn hashed(&self) -> bool {
        self.loaded && self.readable && !self.writable
    }
}

/// Whether the object loaded at `base` with `segments` holds `address`:
/// whether a mapped segment spans it. A segment whose end does not fit in
/// an address holds nothing.
pub(crate) fn holds(segments: &[Segment], base: usize, address: usize) -> bool {
    segments.iter().filter(|s| s.loaded).any(|segment| {
        let start = base.checked_add(segment.start);
        let end = start.and_then(|start| start.checked_add(segment.memory));
        start
            .zip(end)
            .is_some_and(|(start, end)| (start..end).contains(&address))
    })
}

/// The content hash of a loaded library: each hashed segment's start, then
/// its bytes from the file, in the order the object lists them.
pub(crate) fn content_hash<'a>(segments: impl IntoIterator<Item = (usize, &'a [u8])>) -> u128 {
    let mut hasher = ContentHasher::new(TAG);
    for (start, bytes) in segments {
        hasher.u64(start as u64).bytes(bytes);
    }
    hasher.finish()
}

#[cfg(test)]
#[path = "../tests/unit/library.rs"]
mod tests;
