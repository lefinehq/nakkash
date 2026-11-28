//! Reusing a recorded frame in place of drawing it.
//!
//! Three parties must agree before a frame is read rather than drawn:
//!
//! - the runner, which vouches that the code a frame runs has not changed
//!   since its record, and lists the frame with the identities of the lists
//!   it expects it to draw ([`SampleReuse`], in a [`Reuse`] list);
//! - the record, which names the identities of the lists the frame drew
//!   ([`FrameRecord`]);
//! - this run, which computes the identities of the lists the frame draws
//!   now.
//!
//! [`Reusing::frame`] holds the rule. A reused frame's pixels are then read
//! only against the record that names them, so a file that does not hold
//! them is an error, never a wrong pixel.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use nakkash_format::{Diagnostic, Kind, Place, Result};
use serde::{Deserialize, Serialize};

use super::sample::{RECORD, begin_sample, read_pinned};
use super::{FrameRecord, Hex128, Platform, SampleRecord};

/// The identities of the lists one frame draws, each its
/// [`frame_identity`](nakkash_raster::frame_identity) on the scene's canvas
/// with the reference's render options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identities {
    /// The list our frame draws.
    pub ours: Hex128,
    /// That list without its background, which the coverage draws.
    pub bare: Hex128,
    /// The bare list with the ignored layers shown; none when no layer is
    /// ignored. Required, null or not.
    #[serde(deserialize_with = "Option::deserialize")]
    pub shown: Option<Hex128>,
}

impl FrameRecord {
    /// The identities of the lists this frame drew.
    pub fn identities(&self) -> Identities {
        Identities {
            ours: self.ours,
            bare: self.bare,
            shown: self.shown,
        }
    }
}

/// The form of a reuse list, written first so that a reader of another
/// form refuses it rather than misreading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ReuseSchema {
    /// The form this module reads.
    #[default]
    #[serde(rename = "nakkash.FidelityReuse.1")]
    V1,
}

/// The frames of one sample the runner allows to be reused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleReuse {
    /// The folder of the sample's record; in a reuse list, relative to the
    /// list's folder.
    pub dir: PathBuf,
    /// The content hash of the bytes of the record the runner judged
    /// ([`read_pinned`]): a record written over it since, by another build
    /// that may draw the same lists to other bits, is refused.
    pub record: Hex128,
    /// Each frame it vouches for, by index, with the identities of the
    /// lists it expects the frame to draw.
    pub frames: BTreeMap<usize, Identities>,
}

impl SampleReuse {
    /// Vouches for every frame of the record in `dir`, as it was recorded,
    /// pinned to that record: what a runner writes for a build whose
    /// drawing code is unchanged.
    ///
    /// # Errors
    /// As [`read_pinned`].
    pub fn every_frame(dir: PathBuf) -> Result<SampleReuse> {
        let (record, pin) = read_pinned(&dir)?;
        let frames = (record.frames.iter())
            .map(|frame| (frame.index, frame.identities()))
            .collect();
        Ok(SampleReuse {
            dir,
            record: pin,
            frames,
        })
    }
}

/// The frames a runner allows a fidelity run to reuse, by sample id
/// (`nakkash.FidelityReuse.1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reuse {
    /// The list's form.
    pub schema: ReuseSchema,
    /// The machine the list was made for: its records were drawn there.
    pub platform: Platform,
    /// Each sample's frames, by the sample's id.
    pub samples: BTreeMap<String, SampleReuse>,
}

/// The accepted form of a reuse list.
const REUSE: &str = r#"a reuse list: {"schema":"nakkash.FidelityReuse.1","platform":{…},"samples":{"m001":{"dir":"records/m001","frames":{"0":{"ours":…,"bare":…,"shown":null}}}}}"#;

impl Reuse {
    /// Reads the reuse list at `path`; a relative record folder in it is
    /// relative to its folder.
    ///
    /// # Errors
    /// An I/O diagnostic when the file cannot be read, or an invalid one,
    /// with its path, when it is not a reuse list of this form or was made
    /// on another platform.
    pub fn read(path: &Path) -> Result<Reuse> {
        let invalid = |message: String, expected: &'static str| {
            Diagnostic::new(Kind::Invalid, message)
                .at(Place::Other(path.display().to_string()))
                .expected(expected)
        };
        let text = fs::read_to_string(path).map_err(|e| Diagnostic::io("read", path, &e))?;
        let mut reuse: Reuse =
            serde_json::from_str(&text).map_err(|e| invalid(e.to_string(), REUSE))?;
        let here = Platform::current();
        if reuse.platform != here {
            return Err(invalid(
                format!(
                    "the reuse list was made on {}, and this machine is {here}",
                    reuse.platform
                ),
                "a reuse list made on this platform",
            ));
        }
        let base = path.parent().unwrap_or(Path::new("."));
        for sample in reuse.samples.values_mut() {
            sample.dir = base.join(&sample.dir);
        }
        Ok(reuse)
    }
}

/// A sample's record opened for reuse: read once, and only when it can
/// vouch for its pixels, with the frames the runner allows to be reused.
#[derive(Debug, Clone, PartialEq)]
pub struct Reusing {
    /// The record's folder.
    dir: PathBuf,
    /// The frames the runner vouches for, with the identities it expects.
    vouched: BTreeMap<usize, Identities>,
    /// The record.
    record: SampleRecord,
}

/// The accepted form of the record a reuse list names.
const PINNED: &str =
    "the record the runner judged; make the reuse list again from the records as they are";

/// The accepted form of a record opened for reuse.
const REUSABLE: &str =
    "a record made on this platform by a build that knows its commit; record the sample again";

/// The accepted form of a frame the runner vouches for.
const VOUCHED: &str = "a record of the frame's lists at the frame's time; record the sample again, or leave the frame out of the reuse list";

impl Reusing {
    /// Opens the record in `reuse`'s folder ([`read_pinned`]) for the
    /// frames `reuse` vouches for.
    ///
    /// # Errors
    /// As [`read_pinned`]; or an invalid diagnostic, with the record's
    /// path, when it is not the record `reuse` pins, so the runner never
    /// judged it; when the build that made it did not know its commit, so
    /// nothing says what code drew it; or when it was drawn on another
    /// platform, whose processor may draw the same lists to other bits.
    pub fn open(reuse: &SampleReuse) -> Result<Reusing> {
        let (record, pin) = read_pinned(&reuse.dir)?;
        let here = Platform::current();
        let refusal = match (
            pin == reuse.record,
            record.build.identified(),
            record.platform == here,
        ) {
            (false, _, _) => Some((
                format!(
                    "the record hashes to {pin}, not to the {} the reuse list was made from",
                    reuse.record
                ),
                PINNED,
            )),
            (true, false, _) => Some((
                "the record was made by a build that does not know its commit".to_owned(),
                REUSABLE,
            )),
            (true, true, false) => Some((
                format!(
                    "the record was drawn on {}, and this machine is {here}",
                    record.platform
                ),
                REUSABLE,
            )),
            (true, true, true) => None,
        };
        match refusal {
            Some((message, expected)) => Err(invalid(&reuse.dir, message, expected)),
            None => Ok(Reusing {
                dir: reuse.dir.clone(),
                vouched: reuse.frames.clone(),
                record,
            }),
        }
    }

    /// The record's folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The record.
    pub fn record(&self) -> &SampleRecord {
        &self.record
    }

    /// The record of frame `index` to read in place of drawing it, which
    /// draws the lists `fresh` at `time` seconds; none when it is to be
    /// drawn.
    ///
    /// The rule, so that no frame is ever read where its draw could differ:
    ///
    /// - a frame the runner does not list is drawn;
    /// - a frame it lists with other identities than `fresh` is drawn: what
    ///   it draws now is not what the runner vouched for;
    /// - a frame it lists with `fresh` is read when its record names
    ///   `fresh` too, at `time`; when the record does not, or holds no
    ///   such frame, it is an error: the runner vouched for a record that
    ///   cannot stand in for the draw, and a draw in its place would hide
    ///   that the runner and the record disagree.
    ///
    /// # Errors
    /// An invalid diagnostic, with the record's path, for a frame the
    /// runner vouched for that the record does not hold, holds with other
    /// lists, or holds at another time.
    pub fn frame(
        &self,
        index: usize,
        fresh: &Identities,
        time: f64,
    ) -> Result<Option<&FrameRecord>> {
        if self.vouched.get(&index) != Some(fresh) {
            return Ok(None);
        }
        let disagreement = match self.record.frames.get(index) {
            None => format!(
                "frame {index} is vouched for, but the record holds {} frames",
                self.record.frames.len()
            ),
            Some(frame) if frame.identities() != *fresh => format!(
                "frame {index} draws {}, and its record names {}",
                lists(fresh),
                lists(&frame.identities())
            ),
            Some(frame) if frame.time.to_bits() != time.to_bits() => format!(
                "frame {index} is at {time}s, and its record at {}s",
                frame.time
            ),
            Some(frame) => return Ok(Some(frame)),
        };
        Err(invalid(&self.dir, disagreement, VOUCHED))
    }
}

/// `identities`, as an error names them.
fn lists(identities: &Identities) -> String {
    let shown = identities
        .shown
        .map_or_else(|| "none".to_owned(), |shown| shown.to_string());
    format!(
        "ours {}, bare {}, shown {shown}",
        identities.ours, identities.bare
    )
}

/// An invalid diagnostic at the record in `dir`.
fn invalid(dir: &Path, message: String, expected: &'static str) -> Diagnostic {
    Diagnostic::new(Kind::Invalid, message)
        .at(Place::Other(dir.join(RECORD).display().to_string()))
        .expected(expected)
}

/// Begins a recording into `dir` ([`begin_sample`]), while frames may be
/// read from the record in the folder `reused`, if any.
///
/// # Errors
/// An invalid diagnostic when `dir` is the folder `reused`, under any
/// spelling of its path: beginning would remove the frames the run is about
/// to read. Otherwise as [`begin_sample`].
pub fn begin_recording(dir: &Path, reused: Option<&Path>) -> Result<()> {
    refuse_same_folder(dir, reused)?;
    begin_sample(dir)
}

/// Checks that a recording into `dir` has begun ([`begin_recording`]),
/// while frames may be read from the record in the folder `reused`, if
/// any: a frame is never written beside a record of another recording.
///
/// # Errors
/// An invalid diagnostic when `dir` is the folder `reused`, under any
/// spelling of its path, since a recording would write over the frames it
/// reads; or when `dir` is not a folder or holds a record, since the
/// recording has not begun.
pub fn check_recording(dir: &Path, reused: Option<&Path>) -> Result<()> {
    refuse_same_folder(dir, reused)?;
    let unbegun = match (dir.is_dir(), dir.join(RECORD).exists()) {
        (false, _) => Some("the folder does not exist"),
        (true, true) => Some("the folder holds a record"),
        (true, false) => None,
    };
    match unbegun {
        Some(why) => Err(invalid(
            dir,
            format!("{why}, and a recording into it has not begun"),
            "a recording begun with `begin_recording`, which makes the folder and removes its record first",
        )),
        None => Ok(()),
    }
}

/// Refuses `dir` when it is the folder `reused`, under any spelling of its
/// path.
fn refuse_same_folder(dir: &Path, reused: Option<&Path>) -> Result<()> {
    match reused {
        Some(reused) if same_folder(dir, reused) => Err(invalid(
            reused,
            format!(
                "the run would record into {}, the folder it reuses",
                dir.display()
            ),
            "a record folder other than the one the run reuses",
        )),
        _ => Ok(()),
    }
}

/// Whether `a` and `b` are the same folder: both exist and resolve to one
/// path.
fn same_folder(a: &Path, b: &Path) -> bool {
    matches!(
        (fs::canonicalize(a), fs::canonicalize(b)),
        (Ok(a), Ok(b)) if a == b
    )
}

#[cfg(test)]
#[path = "../../tests/unit/record/reuse.rs"]
mod tests;
