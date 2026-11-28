//! A sample's `record.json`: removed when a recording begins, written whole
//! when it ends, so a folder holds a record only for the frame files of the
//! one recording that wrote them both.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use nakkash_format::{Diagnostic, Kind, Place, Result};

use super::SampleRecord;
use super::files::{sync_dir, write_whole};

/// The name of a sample's record file in its folder.
pub const RECORD: &str = "record.json";

/// The accepted form of a record file.
const EXPECTED: &str = r#"a record written by this build: {"schema":"nakkash.FidelityRecord.1",…}"#;

/// Begins a recording into `dir`, which it makes if needed: removes the
/// folder's record, for good, before any frame of the new recording is
/// written over the old ones, so a recording that stops part way leaves no
/// record beside frames it does not name.
///
/// # Errors
/// An I/O diagnostic when the folder cannot be made, or its record cannot
/// be removed.
pub fn begin_sample(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).map_err(|e| Diagnostic::io("create", dir, &e))?;
    let path = dir.join(RECORD);
    match fs::remove_file(&path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            return Err(Diagnostic::io("remove", &path, &e));
        }
        _ => {}
    }
    sync_dir(dir)
}

/// Writes `record` as `record.json` in `dir`, whole: to `record.json.tmp`,
/// flushed to disk, then renamed, and the folder flushed after it. A write
/// that fails leaves any record before it as it was.
///
/// # Errors
/// An I/O diagnostic when the file cannot be written.
pub fn write_sample(dir: &Path, record: &SampleRecord) -> Result<()> {
    write_whole(&dir.join(RECORD), |out| {
        serde_json::to_writer_pretty(&mut *out, record)?;
        out.write_all(b"\n")
    })?;
    sync_dir(dir)
}

/// Reads the record in `dir`.
///
/// # Errors
/// An I/O diagnostic when `record.json` cannot be read, or an invalid one,
/// with its path, when it is not a record of this form: another schema, a
/// key it does not know or lacks, a value of another form, frames out of
/// order, or a frame whose file or time differs from its errors'.
pub fn read_sample(dir: &Path) -> Result<SampleRecord> {
    let path = dir.join(RECORD);
    let invalid = |message: String| {
        Diagnostic::new(Kind::Invalid, message)
            .at(Place::Other(path.display().to_string()))
            .expected(EXPECTED)
    };
    let text = fs::read_to_string(&path).map_err(|e| Diagnostic::io("read", &path, &e))?;
    let record: SampleRecord = serde_json::from_str(&text).map_err(|e| invalid(e.to_string()))?;
    for (i, frame) in record.frames.iter().enumerate() {
        let wrong = if frame.index != i {
            Some(format!("frame {i} has the index {}", frame.index))
        } else if frame.file != frame.diff.file {
            Some(format!(
                "frame {i} names the file `{}`, its errors `{}`",
                frame.file, frame.diff.file
            ))
        } else if frame.time.to_bits() != frame.diff.time.to_bits() {
            Some(format!(
                "frame {i} is at {}s, its errors at {}s",
                frame.time, frame.diff.time
            ))
        } else {
            None
        };
        if let Some(message) = wrong {
            return Err(invalid(message));
        }
    }
    Ok(record)
}

#[cfg(test)]
#[path = "../../tests/unit/record/sample.rs"]
mod tests;
