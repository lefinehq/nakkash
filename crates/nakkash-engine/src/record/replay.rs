//! What a sample's record keeps so that its frames can be drawn again from
//! its folder alone (`nakkash frames`): `scene.json`, the measured scene as
//! the run drew it, and `sample.json`, a [`Replay`]: each frame's time, the
//! ignored layers and the canvas. Both are written whole within the run's
//! quota before the record, and removed with it when another recording
//! begins.

use std::fs;
use std::path::Path;

use nakkash_format::{Address, Diagnostic, Kind, Place, Result};
use serde::{Deserialize, Serialize};

use super::Quota;
use super::files::write_whole;

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
        serde_json::from_slice(&bytes).map_err(|e| {
            Diagnostic::new(Kind::Invalid, e.to_string())
                .at(Place::Other(path.display().to_string()))
                .expected(EXPECTED)
        })
    }

    /// Writes this replay as `sample.json` in `dir`, whole, within `quota`.
    ///
    /// # Errors
    /// An I/O diagnostic when `quota` has no room for it or the file
    /// cannot be written.
    pub fn write(&self, dir: &Path, quota: &Quota) -> Result<()> {
        let path = dir.join(REPLAY);
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|e| {
            Diagnostic::new(Kind::Invalid, format!("cannot write the replay: {e}"))
                .at(Place::Other(path.display().to_string()))
        })?;
        bytes.push(b'\n');
        write_whole(&path, &bytes, quota)
    }
}

/// Writes `scene`, the text of a sample's measured scene, as `scene.json`
/// in `dir`, whole, within `quota`.
///
/// # Errors
/// An I/O diagnostic when `quota` has no room for it or the file cannot be
/// written.
pub fn write_scene(dir: &Path, scene: &str, quota: &Quota) -> Result<()> {
    write_whole(&dir.join(SCENE), scene.as_bytes(), quota)
}

#[cfg(test)]
#[path = "../../tests/unit/record/replay.rs"]
mod tests;
