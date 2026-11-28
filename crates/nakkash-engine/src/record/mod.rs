//! A fidelity record: what each frame of a sample drew, kept so that a later
//! run can reuse a frame instead of drawing it when nothing it draws has
//! changed, and still write the same ledger, byte for byte.
//!
//! A sample's record is a folder:
//!
//! - `record.json`, a [`SampleRecord`]: for each frame, the identities of
//!   the lists it drew, the content hashes of the pixels and masks they gave,
//!   its unrounded errors and its draw times; and the platform, build and
//!   manifest entry it was made with. A recording removes it first
//!   ([`begin_sample`]) and writes it last, whole ([`write_sample`]), so a
//!   folder holds a record only beside the frames of the recording that
//!   wrote it.
//! - for each frame, our pixels (`f0000.ours.lz4`), the coverage
//!   (`f0000.cov`) and, when layers are ignored, the shown mask
//!   (`f0000.shown`) ([`write_frame`]), each written whole. A frame is read
//!   only against the record that names it ([`read_frame`]): a file that
//!   does not hold what the record names is refused, never used.
//!
//! Every hash is the one content hash of the rasteriser's identities
//! (`nakkash_hash`), and covers the frame's size.
//!
//! The record sits after the fidelity measure's draws: a run that records
//! writes what it drew, and a run that reuses reads it back in place of a
//! draw, then measures as always.

mod files;
mod pixels;
mod sample;
mod types;

pub use pixels::{mask_hash, pixels_hash, read_frame, write_frame};
pub use sample::{RECORD, begin_sample, read_sample, write_sample};
pub use types::{Build, Drawn, FrameRecord, Hex128, Platform, SampleRecord, Schema, Times};

#[cfg(test)]
#[path = "../../tests/unit/record/mod.rs"]
pub(crate) mod tests;
