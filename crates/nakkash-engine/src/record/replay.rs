//! What a sample's record keeps so that its frames can be drawn again from
//! its folder alone (`nakkash frames`): `scene.json`, the measured scene as
//! the run drew it, and `sample.json`, a [`Replay`]: each frame's time, the
//! ignored layers and the canvas. Both are written whole within the run's
//! quota before the record, which names each by the content hash of its
//! bytes, and removed with it when another recording begins. They are read
//! only against that record ([`open_replay`]): a file another run wrote
//! over them, or one beside no record, is refused, never drawn.

use std::fs;
use std::path::Path;

use nakkash_format::{Address, Diagnostic, Kind, Place, Result};
use nakkash_hash::content_hash;
use serde::{Deserialize, Serialize};

use super::files::write_whole;
use super::{Hex128, Quota, SampleRecord, read_sample};

/// The tag of the hash of a `scene.json`'s bytes.
const SCENE_TAG: &str = "nakkash.record.scene";

/// The tag of the hash of a `sample.json`'s bytes.
const REPLAY_TAG: &str = "nakkash.record.replay";

/// The accepted form of a scene or replay file read against its record.
const RECORDED: &str =
    "the file the sample was recorded with, as its record names it; record the sample again";

/// The name of a sample's replay file in its record's folder.
pub const REPLAY: &str = "sample.json";

/// The name of the copy of a sample's measured scene in its record's
/// folder.
pub const SCENE: &str = "scene.json";

/// The accepted form of a replay file.
const EXPECTED: &str =
    r#"a replay written by a recording: {"times":[0.0],"ignore":[],"width":1920,"height":1080}"#;

/// How a sample's frames are drawn, besides its scene: what draws them
/// again as its fidelity run drew them
/// ([`Session::replayer`](crate::Session::replayer)).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replay {
    /// The scene time of each compared frame, in seconds, in order; a
    /// frame's index is its place here.
    pub times: Vec<f64>,
    /// The layers left out of the drawing and the measure, by address.
    pub ignore: Vec<Address>,
    /// The canvas's width, in pixels.
    pub width: u32,
    /// The canvas's height, in pixels.
    pub height: u32,
}

impl Replay {
    /// The scene time of frame `index`.
    ///
    /// # Errors
    /// An out-of-range diagnostic for an index past the sample's frames.
    pub fn time(&self, index: usize) -> Result<f64> {
        self.times.get(index).copied().ok_or_else(|| {
            let count = self.times.len();
            let expected = match count {
                0 => "a sample with frames".to_owned(),
                n => format!("a frame index from 0 to {}", n - 1),
            };
            Diagnostic::new(
                Kind::OutOfRange,
                format!("frame {index} is past the sample's {count} frames"),
            )
            .expected(expected)
        })
    }

    /// Reads the replay in `dir`.
    ///
    /// # Errors
    /// An I/O diagnostic when `sample.json` cannot be read, or an invalid
    /// one, with its path, when it is not a replay: a key it does not know
    /// or lacks, or a value of another form.
    pub fn read(dir: &Path) -> Result<Replay> {
        let path = dir.join(REPLAY);
        let bytes = fs::read(&path).map_err(|e| Diagnostic::io("read", &path, &e))?;
        Replay::parse(&path, &bytes)
    }

    /// The replay in `bytes`, read from the file at `path`.
    fn parse(path: &Path, bytes: &[u8]) -> Result<Replay> {
        serde_json::from_slice(bytes).map_err(|e| {
            Diagnostic::new(Kind::Invalid, e.to_string())
                .at(Place::Other(path.display().to_string()))
                .expected(EXPECTED)
        })
    }

    /// Writes this replay as `sample.json` in `dir`, whole, within `quota`;
    /// the content hash of its bytes, which the record names it by.
    ///
    /// # Errors
    /// An I/O diagnostic when `quota` has no room for it or the file
    /// cannot be written.
    pub fn write(&self, dir: &Path, quota: &Quota) -> Result<Hex128> {
        let path = dir.join(REPLAY);
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|e| {
            Diagnostic::new(Kind::Invalid, format!("cannot write the replay: {e}"))
                .at(Place::Other(path.display().to_string()))
        })?;
        bytes.push(b'\n');
        write_whole(&path, &bytes, quota)?;
        Ok(Hex128(content_hash(REPLAY_TAG, &bytes)))
    }
}

/// Writes `scene`, the text of a sample's measured scene, as `scene.json`
/// in `dir`, whole, within `quota`; the content hash of its bytes, which the
/// record names it by.
///
/// # Errors
/// An I/O diagnostic when `quota` has no room for it or the file cannot be
/// written.
pub fn write_scene(dir: &Path, scene: &str, quota: &Quota) -> Result<Hex128> {
    write_whole(&dir.join(SCENE), scene.as_bytes(), quota)?;
    Ok(Hex128(content_hash(SCENE_TAG, scene.as_bytes())))
}

/// A recorded sample opened to draw its frames again: its record, its
/// replay and the text of its scene, each file checked against the record.
#[derive(Debug, Clone, PartialEq)]
pub struct Replayed {
    /// The sample's record.
    pub record: SampleRecord,
    /// How its frames are drawn.
    pub replay: Replay,
    /// The text of its measured scene, as `scene.json` holds it.
    pub scene: String,
}

/// Opens the sample recorded in `dir` to draw its frames again: its record
/// first, then `sample.json` and `scene.json`, each read once and refused
/// unless its bytes hash to what the record names.
///
/// # Errors
/// As [`read_sample`] for a folder without a record; an I/O diagnostic
/// when a file cannot be read; an invalid one, with its path, when a file
/// is not the one the record names, or `sample.json` is not a replay, or
/// `scene.json` is not text.
pub fn open_replay(dir: &Path) -> Result<Replayed> {
    let record = read_sample(dir)?;
    let replay_path = dir.join(REPLAY);
    let bytes = recorded(&replay_path, REPLAY_TAG, record.replay)?;
    let replay = Replay::parse(&replay_path, &bytes)?;
    let scene_path = dir.join(SCENE);
    let scene = String::from_utf8(recorded(&scene_path, SCENE_TAG, record.scene)?)
        .map_err(|e| invalid(&scene_path, e.to_string()))?;
    Ok(Replayed {
        record,
        replay,
        scene,
    })
}

/// The bytes of the file at `path`, whose content hash, tagged `tag`, its
/// record names as `named`.
fn recorded(path: &Path, tag: &str, named: Hex128) -> Result<Vec<u8>> {
    let bytes = fs::read(path).map_err(|e| Diagnostic::io("read", path, &e))?;
    let found = Hex128(content_hash(tag, &bytes));
    match found == named {
        true => Ok(bytes),
        false => Err(invalid(
            path,
            format!("the file hashes to {found}, not to the {named} its record names"),
        )),
    }
}

/// An invalid diagnostic at the file at `path`, not the one its record
/// names.
fn invalid(path: &Path, message: String) -> Diagnostic {
    Diagnostic::new(Kind::Invalid, message)
        .at(Place::Other(path.display().to_string()))
        .expected(RECORDED)
}

#[cfg(test)]
#[path = "../../tests/unit/record/replay.rs"]
mod tests;
