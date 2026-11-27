//! What a record holds, as serde types: every one refuses keys it does not
//! know, so a record of another form is never half read.

use std::fmt;
use std::str::FromStr;

use nakkash_format::grammar::parse_whole;
use nakkash_format::{Diagnostic, Result};
use nakkash_raster::Image;
use serde::{Deserialize, Serialize};
use winnow::Parser;
use winnow::token::take_while;

use crate::diff::FrameDiff;

/// A 128-bit identity or content hash, written as exactly 32 lowercase
/// hexadecimal digits, so one value has one spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Hex128(pub u128);

/// The one form [`Hex128`] reads.
const HEX128: &str = "32 lowercase hexadecimal digits, e.g. `0000000000000000000000000000beef`";

impl fmt::Display for Hex128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl FromStr for Hex128 {
    type Err = Diagnostic;

    fn from_str(text: &str) -> Result<Hex128> {
        let digits = take_while(32, ('0'..='9', 'a'..='f'));
        // 32 hexadecimal digits always fit in 128 bits.
        let value = digits.verify_map(|digits| u128::from_str_radix(digits, 16).ok());
        parse_whole(text, value, "128-bit hash", HEX128).map(Hex128)
    }
}

impl TryFrom<String> for Hex128 {
    type Error = Diagnostic;

    fn try_from(text: String) -> Result<Hex128> {
        text.parse()
    }
}

impl From<Hex128> for String {
    fn from(hash: Hex128) -> String {
        hash.to_string()
    }
}

/// The form of a record, written first so that a reader of another form
/// refuses it rather than misreading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Schema {
    /// The form this module writes.
    #[default]
    #[serde(rename = "nakkash.FidelityRecord.1")]
    V1,
}

/// The machine a record was drawn on, as far as identities cannot see it:
/// the same display list may draw other bits on another processor or SIMD
/// level, so a record is reused only on the platform it names.
///
/// Footage a machine cannot decode, or fonts it lacks, already change the
/// display list, and so the identity, or fail the sample.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Platform {
    /// The processor and system, such as `aarch64-macos`.
    pub target: String,
    /// The SIMD level native frames are drawn with, such as `neon`.
    pub simd: String,
}

impl Platform {
    /// This machine.
    pub fn current() -> Platform {
        Platform {
            target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
            simd: nakkash_raster::simd_level().to_owned(),
        }
    }
}

/// The build a record was made with, so that both sides of a comparison
/// are known to come from the same code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    /// The commit checked out, as `git rev-parse HEAD` gives it, or
    /// `unknown` when the build had no git.
    pub commit: String,
    /// Whether the tree had changes from that commit.
    pub dirty: bool,
    /// The cargo profile, such as `release`.
    pub profile: String,
}

impl Build {
    /// This build, as the build script named it.
    pub fn current() -> Build {
        Build {
            commit: env!("NAKKASH_BUILD_COMMIT").to_owned(),
            dirty: env!("NAKKASH_BUILD_DIRTY") == "true",
            profile: env!("NAKKASH_BUILD_PROFILE").to_owned(),
        }
    }
}

/// How long a frame's draws took, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Times {
    /// Our frame.
    pub draw: f64,
    /// The coverage: the measured scene with no background.
    pub coverage: f64,
    /// The scene with the ignored layers shown; 0 when none is ignored.
    pub shown: f64,
}

/// What one frame drew and how it compared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameRecord {
    /// The frame's place among the sample's compared frames, from 0.
    pub index: usize,
    /// The reference frame's file name.
    pub file: String,
    /// The scene time it was drawn at, in seconds.
    pub time: f64,
    /// The identity of the list our frame drew.
    pub ours: Hex128,
    /// The identity of the bare list the coverage drew.
    pub bare: Hex128,
    /// The identity of the list with the ignored layers shown; none when no
    /// layer is ignored.
    pub shown: Option<Hex128>,
    /// The content hash of our pixels ([`pixels_hash`](super::pixels_hash)).
    pub ours_pixels: Hex128,
    /// The content hash of the coverage ([`mask_hash`](super::mask_hash)).
    pub coverage: Hex128,
    /// The content hash of the shown mask; none when no layer is ignored.
    pub shown_mask: Option<Hex128>,
    /// The frame's errors, unrounded.
    pub diff: FrameDiff,
    /// How long its draws took.
    pub ms: Times,
}

/// One sample's record: what each of its frames drew, and what it was
/// drawn with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleRecord {
    /// The record's form.
    pub schema: Schema,
    /// The sample's id in the manifest.
    pub sample: String,
    /// The machine it was drawn on.
    pub platform: Platform,
    /// The build it was drawn with.
    pub build: Build,
    /// The content hash of the sample's manifest entry as serialised: a
    /// changed entry is a changed sample.
    pub entry: Hex128,
    /// Every compared frame, in order.
    pub frames: Vec<FrameRecord>,
}

/// What one frame drew: the pixels and masks a record keeps of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawn {
    /// Our frame.
    pub ours: Image,
    /// Whether the measured scene draws each pixel, before the background.
    pub coverage: Vec<bool>,
    /// Whether the scene draws each pixel with the ignored layers shown;
    /// none when no layer is ignored.
    pub shown: Option<Vec<bool>>,
}

#[cfg(test)]
#[path = "../../tests/unit/record/types.rs"]
mod tests;
