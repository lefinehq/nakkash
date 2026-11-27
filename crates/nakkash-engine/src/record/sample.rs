//! A sample's `record.json`: written to a temporary file, flushed to disk
//! and renamed into place, so a reader sees a whole record or none.

use std::fs::{self, File};
use std::io::{BufWriter, IntoInnerError, Write};
use std::path::Path;

use nakkash_format::{Diagnostic, Kind, Place, Result};

use super::SampleRecord;

/// The name of a sample's record file in its folder.
pub const RECORD: &str = "record.json";

/// Writes `record` as `record.json` in `dir`, which it makes if needed:
/// to `record.json.tmp` first, then renamed, so a write that fails leaves
/// any record before it whole. A failed write removes what it wrote.
///
/// # Errors
/// An I/O diagnostic when the folder or the file cannot be written.
pub fn write_sample(dir: &Path, record: &SampleRecord) -> Result<()> {
    fs::create_dir_all(dir).map_err(|e| Diagnostic::io("create", dir, &e))?;
    let path = dir.join(RECORD);
    let tmp = dir.join(format!("{RECORD}.tmp"));
    let write = || {
        let mut file = BufWriter::new(File::create(&tmp)?);
        serde_json::to_writer_pretty(&mut file, record)?;
        file.write_all(b"\n")?;
        file.into_inner()
            .map_err(IntoInnerError::into_error)?
            .sync_all()
    };
    write()
        .map_err(|e| Diagnostic::io("write", &tmp, &e))
        .and_then(|()| fs::rename(&tmp, &path).map_err(|e| Diagnostic::io("rename", &tmp, &e)))
        .inspect_err(|_| {
            // Only a file this write made; a folder in its way is not ours.
            if tmp.is_file() {
                let _ = fs::remove_file(&tmp);
            }
        })
}

/// Reads the record in `dir`.
///
/// # Errors
/// An I/O diagnostic when `record.json` cannot be read, or an invalid one,
/// with its path, when it is not a record of this form: another schema, a
/// key it does not know or lacks, or a value of another form.
pub fn read_sample(dir: &Path) -> Result<SampleRecord> {
    let path = dir.join(RECORD);
    let text = fs::read_to_string(&path).map_err(|e| Diagnostic::io("read", &path, &e))?;
    serde_json::from_str(&text).map_err(|e| {
        Diagnostic::new(Kind::Invalid, e.to_string())
            .at(Place::Other(path.display().to_string()))
            .expected(r#"a record written by this build: {"schema":"nakkash.FidelityRecord.1",…}"#)
    })
}

#[cfg(test)]
#[path = "../../tests/unit/record/sample.rs"]
mod tests;
