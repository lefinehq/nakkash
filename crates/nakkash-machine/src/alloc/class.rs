//! The size classes of the blocks [`GivesBack`](super::GivesBack) maps:
//! sixteen to an octave from [`BIG`], so one freed block serves every size
//! of its class.
//!
//! Class `c` holds `(16 + c % 16) << (16 + c / 16)` bytes: 1 MiB, then
//! 1 MiB + 64 KiB, … 2 MiB − 64 KiB, 2 MiB, 2 MiB + 128 KiB, and so on. A
//! class's step is a sixteenth of its octave, so a size is at most a
//! sixteenth (6.25 %) short of its class, and every class is a whole
//! number of 64 KiB, so of every system's pages. Both functions are pure:
//! a block is mapped at its class's bytes, and `dealloc` finds the class
//! again from the layout's size alone.

use super::BIG;

/// Classes to an octave.
const PER_OCTAVE: usize = 16;

/// The shift of [`BIG`]: it is `1 << BIG_SHIFT`.
const BIG_SHIFT: u32 = BIG.trailing_zeros();

/// The shift of [`PER_OCTAVE`].
const PER_OCTAVE_SHIFT: u32 = PER_OCTAVE.trailing_zeros();

/// How many classes there are: enough for the largest size a layout can
/// have, `isize::MAX` bytes (689 on 64-bit systems), so every block of at
/// least [`BIG`] bytes has a class and no class's bytes overflow.
pub(crate) const CLASSES: usize = class_of(isize::MAX as usize) + 1;

/// The smallest class whose bytes hold `size`; class 0 for every size up
/// to [`BIG`].
///
/// With `n = size − 1`, the octave is `n`'s highest bit, and the step in
/// it is the next four bits rounded up, so the class's bytes are the
/// smallest of the form `(16 + step) << shift` above `n`.
pub(crate) const fn class_of(size: usize) -> usize {
    let n = match size > BIG {
        true => size - 1,
        // One below BIG: the top of the octave under it, which is class 0.
        false => BIG - 1,
    };
    let octave = n.ilog2() + 1 - BIG_SHIFT;
    let steps = n >> (octave + BIG_SHIFT - 1 - PER_OCTAVE_SHIFT);
    PER_OCTAVE * octave as usize + steps + 1 - 2 * PER_OCTAVE
}

/// The bytes of a block of class `class`, which is below [`CLASSES`].
pub(crate) const fn class_bytes(class: usize) -> usize {
    let octave = (class / PER_OCTAVE) as u32;
    (PER_OCTAVE + class % PER_OCTAVE) << (BIG_SHIFT - PER_OCTAVE_SHIFT + octave)
}

#[cfg(test)]
#[path = "../../tests/unit/alloc/class.rs"]
mod tests;
