//! The process's resource use, from `getrusage`, the same call on every
//! Unix, and the free space of a file system.

use std::ffi::CString;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::Duration;

use crate::Usage;

/// What `getrusage` says this process used; zero if it fails.
pub(crate) fn rusage() -> Usage {
    let mut usage = MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `getrusage` writes one `rusage` into the pointer, which
    // points at one.
    let done = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if done != 0 {
        return Usage::default();
    }
    // SAFETY: a zeroed `rusage` is valid, as it holds only integers, and
    // the call above filled it.
    let usage = unsafe { usage.assume_init() };
    let time = |t: libc::timeval| {
        Duration::new(
            u64::try_from(t.tv_sec).unwrap_or(0),
            u32::try_from(t.tv_usec).unwrap_or(0) * 1000,
        )
    };
    let peak = u64::try_from(usage.ru_maxrss).unwrap_or(0);
    Usage {
        user: time(usage.ru_utime),
        system: time(usage.ru_stime),
        // macOS gives bytes, Linux KiB.
        peak_resident: match cfg!(target_os = "macos") {
            true => peak,
            false => peak * 1024,
        },
        major_faults: u64::try_from(usage.ru_majflt).unwrap_or(0),
        written: None,
    }
}

/// The size of a memory page, in bytes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code, reason = "read on Linux"))]
pub(crate) fn page_size() -> u64 {
    // SAFETY: `sysconf` only reads a configuration value.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    u64::try_from(size).ok().filter(|&n| n > 0).unwrap_or(4096)
}

/// The bytes free to an unprivileged process on the file system holding
/// `path`: the blocks available to it times their size. macOS's `statvfs`
/// counts blocks in 32 bits, so there `statfs`, whose counts are 64-bit,
/// gives them.
pub(crate) fn free_space(path: &Path) -> Option<u64> {
    let name = CString::new(path.as_os_str().as_bytes()).ok()?;
    #[cfg(target_os = "macos")]
    {
        let mut stat = MaybeUninit::<libc::statfs>::zeroed();
        // SAFETY: `name` is a NUL-terminated string, and `statfs` writes
        // one `statfs` into the pointer, which points at one.
        let done = unsafe { libc::statfs(name.as_ptr(), stat.as_mut_ptr()) };
        if done != 0 {
            return None;
        }
        // SAFETY: a zeroed `statfs` is valid, as it holds only integers
        // and byte arrays, and the call above filled it.
        let stat = unsafe { stat.assume_init() };
        stat.f_bavail.checked_mul(u64::from(stat.f_bsize))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut stat = MaybeUninit::<libc::statvfs>::zeroed();
        // SAFETY: `name` is a NUL-terminated string, and `statvfs` writes
        // one `statvfs` into the pointer, which points at one.
        let done = unsafe { libc::statvfs(name.as_ptr(), stat.as_mut_ptr()) };
        if done != 0 {
            return None;
        }
        // SAFETY: a zeroed `statvfs` is valid, as it holds only integers,
        // and the call above filled it.
        let stat = unsafe { stat.assume_init() };
        #[allow(clippy::useless_conversion, reason = "the widths differ by platform")]
        let (available, size) = (u64::from(stat.f_bavail), u64::from(stat.f_frsize));
        available.checked_mul(size)
    }
}
