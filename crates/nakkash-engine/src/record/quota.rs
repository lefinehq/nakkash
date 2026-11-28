//! The room a run's records may take ([`Quota`]): checked against the free
//! space of their file system before anything is written, and taken file by
//! file before each is written, so a recording stops at its cap rather
//! than fill the disk.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use nakkash_format::{Diagnostic, Kind, Result};

/// The accepted form of a cap.
const FITS: &str = "a record cap that fits in the free space of the records' file system";

/// The accepted form of a recording that reached its cap.
const LARGER: &str = "a larger record cap, or fewer samples recorded at once";

/// How many bytes a run's records may take, and how many they took. Every
/// file a recording writes takes its bytes first ([`Quota::take`]); once a
/// write is refused, the quota is spent and refuses every one after it
/// ([`Quota::check`]), so a run stops recording rather than record some
/// samples past the point it should have stopped. Threads share one.
#[derive(Debug)]
pub struct Quota {
    /// The most the records may take, in bytes.
    cap: u64,
    /// What they took so far, in bytes.
    taken: AtomicU64,
    /// Whether a write was refused.
    spent: AtomicBool,
}

impl PartialEq for Quota {
    fn eq(&self, other: &Quota) -> bool {
        self.cap == other.cap
            && self.taken() == other.taken()
            && self.spent.load(Ordering::Acquire) == other.spent.load(Ordering::Acquire)
    }
}

impl Quota {
    /// A quota of `cap` bytes for records written under `dir`, which need
    /// not exist yet: the file system that holds it, or the folder that
    /// would hold it, must have `cap` bytes free.
    ///
    /// # Errors
    /// An I/O diagnostic when the file system has fewer than `cap` bytes
    /// free, or does not say how many it has.
    pub fn within(dir: &Path, cap: u64) -> Result<Quota> {
        let absolute = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
        let holder = absolute.ancestors().find(|folder| folder.is_dir());
        let free = holder.and_then(nakkash_machine::free_space);
        let shown = dir.display();
        let short = match free {
            Some(free) if free >= cap => None,
            Some(free) => Some(format!(
                "the file system of `{shown}` holds {free} bytes free, fewer than the records' cap of {cap} bytes"
            )),
            None => Some(format!(
                "cannot tell how many bytes are free on the file system of `{shown}`"
            )),
        };
        match short {
            Some(message) => Err(Diagnostic::new(Kind::Io, message).expected(FITS)),
            None => Ok(Quota {
                cap,
                taken: AtomicU64::new(0),
                spent: AtomicBool::new(false),
            }),
        }
    }

    /// The most the records may take, in bytes.
    pub fn cap(&self) -> u64 {
        self.cap
    }

    /// What the records took so far, in bytes.
    pub fn taken(&self) -> u64 {
        self.taken.load(Ordering::Acquire)
    }

    /// Checks that the quota is not spent: what a recording asks before it
    /// begins, so a sample after the cap was reached keeps its record.
    ///
    /// # Errors
    /// An I/O diagnostic once a write was refused.
    pub fn check(&self) -> Result<()> {
        match self.spent.load(Ordering::Acquire) {
            true => Err(Diagnostic::new(
                Kind::Io,
                format!(
                    "the records reached their cap of {} bytes, and nothing more is recorded",
                    self.cap
                ),
            )
            .expected(LARGER)),
            false => Ok(()),
        }
    }

    /// Takes `bytes` for the file at `path`, before it is written.
    ///
    /// # Errors
    /// An I/O diagnostic, which spends the quota, when the records would
    /// take more than the cap with it; or as [`Quota::check`].
    pub fn take(&self, path: &Path, bytes: u64) -> Result<()> {
        self.check()?;
        let cap = self.cap;
        let fits = |taken: u64| taken.checked_add(bytes).filter(|&total| total <= cap);
        match (self.taken).fetch_update(Ordering::AcqRel, Ordering::Acquire, fits) {
            Ok(_) => Ok(()),
            Err(taken) => {
                self.spent.store(true, Ordering::Release);
                Err(Diagnostic::new(
                    Kind::Io,
                    format!(
                        "writing `{}` would take the records past their cap of {cap} bytes, with {taken} taken",
                        path.display()
                    ),
                )
                .expected(LARGER))
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/record/quota.rs"]
mod tests;
