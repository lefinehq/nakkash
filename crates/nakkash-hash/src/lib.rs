//! The one hash of nakkash's content: SipHash 1-3 with 128 bits out and a
//! fixed key, so a value is the same in every run and on every machine
//! ([D90]).
//!
//! The rasteriser keys frame identities by it ([`keyed`]); a fidelity record
//! names our pixels and masks by it, the `nakkash` build names its changed
//! tree by it, and the engine's build names its code generation by it
//! ([`ContentHasher`], [`content_hash`]). Both sides of a comparison so hash
//! with the same code. The crate is small and depends on no other nakkash
//! crate (only on the SipHash implementation), so a build script can use it
//! too.
//!
//! [D90]: https://github.com/lefinehq/nakkash/blob/main/docs/design/decisions.md#d90-a-frame-is-drawn-from-a-plan-that-knows-what-each-drawing-is-under-one-memory-budget

use std::hash::Hasher as _;

use siphasher::sip128::{Hasher128, SipHasher13};

/// The fixed key: "nakkash" and "identity" in ASCII.
const KEYS: (u64, u64) = (0x6e61_6b6b_6173_6800, 0x6964_656e_7469_7479);

/// A SipHash 1-3 hasher with the fixed key and nothing written yet.
pub fn keyed() -> SipHasher13 {
    SipHasher13::new_with_keys(KEYS.0, KEYS.1)
}

/// Hashes the parts of a piece of content: its tag first, so contents of
/// different kinds that hold the same bytes hash apart, then each part.
/// Byte parts are written with their lengths, so moving a byte from one
/// part to the next is another hash.
pub struct ContentHasher(SipHasher13);

impl ContentHasher {
    /// A hasher for content of kind `tag`.
    pub fn new(tag: &str) -> ContentHasher {
        let mut hasher = ContentHasher(keyed());
        hasher.bytes(tag.as_bytes());
        hasher
    }

    /// Writes a number.
    pub fn u32(&mut self, n: u32) -> &mut ContentHasher {
        self.0.write_u32(n);
        self
    }

    /// Writes a number.
    pub fn u64(&mut self, n: u64) -> &mut ContentHasher {
        self.0.write_u64(n);
        self
    }

    /// Writes `bytes`, after their length.
    pub fn bytes(&mut self, bytes: &[u8]) -> &mut ContentHasher {
        self.u64(bytes.len() as u64);
        self.0.write(bytes);
        self
    }

    /// The hash of what was written.
    pub fn finish(&self) -> u128 {
        self.0.finish128().as_u128()
    }
}

/// The content hash of `bytes`, tagged `tag`.
pub fn content_hash(tag: &str, bytes: &[u8]) -> u128 {
    ContentHasher::new(tag).bytes(bytes).finish()
}

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;
