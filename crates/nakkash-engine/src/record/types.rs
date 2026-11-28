//! What a record holds, as serde types: every one refuses keys it does not
//! know, and requires every key it has, those whose value may be null too,
//! so a record of another form is never half read.

use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

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
/// level, with another system maths library, or from machine code compiled
/// by another toolchain or with other flags, so a record is reused only on
/// the platform it names, every part known and equal ([`Platform::unlike`]).
///
/// Footage a machine cannot decode, or fonts it lacks, already change the
/// display list, and so the identity, or fail the sample.
///
/// Platforms are compared only by [`Platform::unlike`], never by `==`:
/// equality would take two unknown systems for the same one.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Platform {
    /// The processor and system, such as `aarch64-macos`.
    pub target: String,
    /// The SIMD level native frames are drawn with, such as `neon`.
    pub simd: String,
    /// The system's version, whose library holds the maths functions the
    /// rasteriser calls ([`nakkash_machine::system`]), such as `macOS 26.3,
    /// build 25D125`, or on Linux `glibc 2.36, libm <32 hex digits>, Linux
    /// 6.1.0-28-amd64`, with the content hash of the maths library loaded;
    /// none when the system does not say. Required, null or not.
    #[serde(deserialize_with = "Option::deserialize")]
    pub system: Option<String>,
    /// The hash of the compiler's version and the flags the engine was
    /// compiled with (the engine's `build/codegen.rs`), as its build script
    /// named them; none when the compiler could not say. Required, null or
    /// not.
    #[serde(deserialize_with = "Option::deserialize")]
    pub codegen: Option<Hex128>,
}

/// One part of a platform, as [`Platform::unlike`] compares it: its name
/// and its value, none when unknown.
type Part = (&'static str, Option<String>);

impl Platform {
    /// This machine and build.
    pub fn current() -> Platform {
        static CURRENT: OnceLock<Platform> = OnceLock::new();
        CURRENT
            .get_or_init(|| Platform {
                target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
                simd: nakkash_raster::simd_level().to_owned(),
                system: nakkash_machine::system(),
                codegen: codegen_from_script(env!("NAKKASH_CODEGEN")),
            })
            .clone()
    }

    /// Why a record drawn on this platform cannot stand for a draw on
    /// `here`: the first part that differs, or that either side does not
    /// know, since an unknown part matches nothing, not even another
    /// unknown; none when every part is known and equal.
    pub fn unlike(&self, here: &Platform) -> Option<String> {
        let mut parts = self.parts().into_iter().zip(here.parts());
        parts.find_map(|((name, theirs), (_, ours))| match (theirs, ours) {
            (Some(theirs), Some(ours)) if theirs == ours => None,
            (Some(theirs), Some(ours)) => Some(format!(
                "its {name} is {theirs}, and this machine's is {ours}"
            )),
            (None, _) => Some(format!(
                "its {name} is unknown, and an unknown {name} matches none"
            )),
            (Some(_), None) => Some(format!(
                "this machine's {name} is unknown, and an unknown {name} matches none"
            )),
        })
    }

    /// Every part, in the order [`Platform::unlike`] names them.
    fn parts(&self) -> [Part; 4] {
        [
            ("target", Some(self.target.clone())),
            ("SIMD level", Some(self.simd.clone())),
            ("system", self.system.clone()),
            ("code generation", self.codegen.map(|hash| hash.to_string())),
        ]
    }
}

/// The code generation the build script wrote (`NAKKASH_CODEGEN`): a hash,
/// or empty when it could not tell. Anything else is unknown too, never a
/// hash.
pub(crate) fn codegen_from_script(script: &str) -> Option<Hex128> {
    script.parse().ok()
}

/// The build a record was made with, so that both sides of a comparison
/// are known to come from the same code. The binary names it, from its
/// build script, and hands it to the engine; the engine only keeps it.
///
/// A build is identified by its commit and, when the tree had changes, by
/// the hash of those changes. A build made without git, or outside its own
/// checkout, is unidentified ([`Build::identified`]): nothing says what
/// code drew its record, so the record must never be reused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    /// The commit checked out, as `git rev-parse HEAD` gives it; none when
    /// the build could not tell.
    #[serde(deserialize_with = "Option::deserialize")]
    pub commit: Option<String>,
    /// The content hash of the tree's changes from that commit, tracked and
    /// untracked; none when the tree was clean. Required, null or not: a
    /// record without it does not say whether its tree was clean.
    #[serde(deserialize_with = "Option::deserialize")]
    pub tree: Option<Hex128>,
    /// The cargo profile, such as `release`.
    pub profile: String,
}

impl Build {
    /// Whether the build knows what code it is: a record made by a build
    /// that does not is never reused.
    pub fn identified(&self) -> bool {
        self.commit.is_some()
    }

    /// Whether the tree had changes from its commit.
    pub fn dirty(&self) -> bool {
        self.tree.is_some()
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
    #[serde(deserialize_with = "Option::deserialize")]
    pub shown: Option<Hex128>,
    /// The content hash of our pixels ([`pixels_hash`](super::pixels_hash)).
    pub ours_pixels: Hex128,
    /// The content hash of the coverage ([`mask_hash`](super::mask_hash)).
    pub coverage: Hex128,
    /// The content hash of the shown mask; none when no layer is ignored.
    #[serde(deserialize_with = "Option::deserialize")]
    pub shown_mask: Option<Hex128>,
    /// The frame's errors, unrounded.
    pub diff: FrameDiff,
    /// How long its draws took.
    pub ms: Times,
}

/// One sample's record: what each of its frames drew, and what it was
/// drawn with. Not comparable by `==`, as its [`Platform`] is not.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// The content hash of the bytes of the sample's `scene.json`
    /// ([`write_scene`](super::write_scene)): the scene `nakkash frames`
    /// draws its frames again from, refused when it is not this one.
    pub scene: Hex128,
    /// The content hash of the bytes of the sample's `sample.json`
    /// ([`Replay::write`](super::Replay::write)), refused as `scene` is.
    pub replay: Hex128,
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
