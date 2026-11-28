//! Writing a record's files whole: each to a temporary file beside it,
//! flushed to disk and renamed into place, so a reader finds the file as it
//! was before or as it is now, never in between.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use nakkash_format::{Diagnostic, Result};

/// Writes the file at `path` with what `write` writes: to `path` with
/// `.tmp` added, flushed to disk, then renamed. A write that fails removes
/// what it wrote and leaves any file before it as it was.
///
/// # Errors
/// An I/O diagnostic when the file cannot be written or renamed.
pub(super) fn write_whole(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<File>) -> io::Result<()>,
) -> Result<()> {
    let tmp = temporary(path);
    let attempt = || {
        let mut out = BufWriter::new(File::create(&tmp)?);
        write(&mut out)?;
        out.flush()?;
        out.into_inner()
            .map_err(io::IntoInnerError::into_error)?
            .sync_all()
    };
    attempt()
        .map_err(|e| Diagnostic::io("write", &tmp, &e))
        .and_then(|()| fs::rename(&tmp, path).map_err(|e| Diagnostic::io("rename", &tmp, &e)))
        .inspect_err(|_| {
            // Only a file this write made; a folder in its way is not ours.
            if tmp.is_file() {
                let _ = fs::remove_file(&tmp);
            }
        })
}

/// What a temporary file's name adds to the name of the file it becomes.
pub(super) const TMP: &str = ".tmp";

/// `path` with `.tmp` added, such as `record.json.tmp`.
pub(super) fn temporary(path: &Path) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(TMP);
    PathBuf::from(name)
}

/// Flushes the entries of the folder `dir` to disk, so the renames and
/// removals in it outlast a power loss.
///
/// # Errors
/// An I/O diagnostic when the folder cannot be opened or flushed.
#[cfg(unix)]
pub(super) fn sync_dir(dir: &Path) -> Result<()> {
    File::open(dir)
        .and_then(|folder| folder.sync_all())
        .map_err(|e| Diagnostic::io("flush", dir, &e))
}

/// Windows cannot open a folder as a file; its file system journals the
/// renames in it.
#[cfg(not(unix))]
pub(super) fn sync_dir(_dir: &Path) -> Result<()> {
    Ok(())
}
