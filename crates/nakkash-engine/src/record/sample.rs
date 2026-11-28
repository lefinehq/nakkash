//! A sample's `record.json`: removed when a recording begins, written whole
//! when it ends, so a folder holds a record only for the frame files of the
//! one recording that wrote them both.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nakkash_format::{Diagnostic, Kind, Place, Result};
use nakkash_hash::content_hash;

use super::files::{sync_dir, temporary, write_whole};
use super::pixels::is_frame_file;
use super::replay::{REPLAY, SCENE};
use super::{Hex128, Quota, SampleRecord};

/// The name of a sample's record file in its folder.
pub const RECORD: &str = "record.json";

/// The accepted form of a record file.
const EXPECTED: &str = r#"a record written by this build: {"schema":"nakkash.FidelityRecord.1",…}"#;

/// Begins a recording into `dir`, which it makes if needed: removes the
/// folder's record, for good, before any frame of the new recording is
/// written, so a recording that stops part way leaves no record beside
/// frames it does not name; then the frame files and the replay
/// (`sample.json`, `scene.json`) of the recordings before it, so none
/// outlives them, and the record's own files written part way
/// (`record.json.tmp`, `f0000.cov.tmp`). Folders, and files of other names,
/// are left as they are: the folder may hold what other tools wrote.
///
/// # Errors
/// An I/O diagnostic when the folder cannot be made or read, or a file in
/// it cannot be removed.
pub fn begin_sample(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).map_err(|e| Diagnostic::io("create", dir, &e))?;
    remove(&dir.join(RECORD))?;
    sync_dir(dir)?;
    let named: Vec<PathBuf> = [RECORD, REPLAY, SCENE]
        .into_iter()
        .flat_map(|name| [PathBuf::from(name), temporary(Path::new(name))])
        .filter(|name| name != Path::new(RECORD))
        .collect();
    let entries = fs::read_dir(dir).map_err(|e| Diagnostic::io("read", dir, &e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Diagnostic::io("read", dir, &e))?;
        let path = entry.path();
        let name = entry.file_name();
        let ours = named.iter().any(|ours| ours.as_os_str() == name)
            || name.to_str().is_some_and(is_frame_file);
        if ours && path.is_file() {
            remove(&path)?;
        }
    }
    sync_dir(dir)
}

/// Removes the file at `path`, if there is one.
fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(Diagnostic::io("remove", path, &e)),
        _ => Ok(()),
    }
}

/// Writes `record` as `record.json` in `dir`, whole, once `quota` has room
/// for it: to `record.json.tmp`, flushed to disk, then renamed, and the
/// folder flushed after it. A write that fails leaves any record before it
/// as it was.
///
/// # Errors
/// An I/O diagnostic when `quota` has no room for it or the file cannot be
/// written.
pub fn write_sample(dir: &Path, record: &SampleRecord, quota: &Quota) -> Result<()> {
    let path = dir.join(RECORD);
    let mut bytes = serde_json::to_vec_pretty(record).map_err(|e| {
        Diagnostic::new(Kind::Invalid, format!("cannot write the record: {e}"))
            .at(Place::Other(path.display().to_string()))
    })?;
    bytes.push(b'\n');
    write_whole(&path, &bytes, quota)?;
    sync_dir(dir)
}

/// The tag of the hash of a record file's bytes ([`read_pinned`]).
const PIN: &str = "nakkash.record.file";

/// Reads the record in `dir`.
///
/// # Errors
/// As [`read_pinned`].
pub fn read_sample(dir: &Path) -> Result<SampleRecord> {
    read_pinned(dir).map(|(record, _)| record)
}

/// Reads the record in `dir`, with the content hash of the bytes it was
/// read from: what pins a reuse list to the one record its runner judged.
///
/// # Errors
/// An I/O diagnostic when `record.json` cannot be read, or an invalid one,
/// with its path, when it is not a record of this form: another schema, a
/// key it does not know or lacks, a value of another form, frames out of
/// order, a frame whose file or time differs from its errors', or one that
/// names a shown list without a shown mask, or a mask without a list.
pub fn read_pinned(dir: &Path) -> Result<(SampleRecord, Hex128)> {
    let path = dir.join(RECORD);
    let invalid = |message: String| {
        Diagnostic::new(Kind::Invalid, message)
            .at(Place::Other(path.display().to_string()))
            .expected(EXPECTED)
    };
    let bytes = fs::read(&path).map_err(|e| Diagnostic::io("read", &path, &e))?;
    let record: SampleRecord =
        serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
    for (i, frame) in record.frames.iter().enumerate() {
        let checks = [
            (
                frame.index != i,
                format!("frame {i} has the index {}", frame.index),
            ),
            (
                frame.file != frame.diff.file,
                format!(
                    "frame {i} names the file `{}`, its errors `{}`",
                    frame.file, frame.diff.file
                ),
            ),
            (
                frame.time.to_bits() != frame.diff.time.to_bits(),
                format!(
                    "frame {i} is at {}s, its errors at {}s",
                    frame.time, frame.diff.time
                ),
            ),
            (
                frame.shown.is_some() != frame.shown_mask.is_some(),
                format!(
                    "frame {i} names a shown list without its mask, or a mask without its list"
                ),
            ),
        ];
        if let Some((_, message)) = checks.into_iter().find(|(wrong, _)| *wrong) {
            return Err(invalid(message));
        }
    }
    Ok((record, Hex128(content_hash(PIN, &bytes))))
}

#[cfg(test)]
#[path = "../../tests/unit/record/sample.rs"]
mod tests;
