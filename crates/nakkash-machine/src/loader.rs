//! The maths library the process calls, as the dynamic loader holds it
//! (Linux with the GNU C library): the loaded object that defines `sin`,
//! named by the content of its read-only segments ([`library`]).
//!
//! glibc's maths library (`libm.so.6`) defines every C maths function the
//! rasteriser calls (`exp`, `sin`, `cos`, `powf`, `atan2`, `hypot`, …), so
//! the object that defines one defines them all. A distribution's patch
//! update of glibc keeps its upstream version, which is all
//! `gnu_get_libc_version` says, but not the library's bytes.
//!
//! [`library`]: crate::library

use std::ffi::{CStr, c_int, c_void};
use std::sync::OnceLock;

use crate::library::{self, Segment};

/// The function whose defining object is the maths library.
const MATHS_SYMBOL: &CStr = c"sin";

/// A loaded library: its name as the loader gives it, and its content
/// hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Loaded {
    /// The path the loader opened it by, such as
    /// `/lib/x86_64-linux-gnu/libm.so.6`; only said, never compared.
    #[cfg_attr(not(test), allow(dead_code, reason = "the tests read it"))]
    pub(crate) name: String,
    /// The hash of its read-only segments ([`library::content_hash`]).
    pub(crate) hash: u128,
}

/// The maths library the process calls, read once: what is loaded does
/// not change while it runs. `None` when no loaded object defines `sin`;
/// when the one that does is the program itself (a static build, or one
/// that keeps its own entry for the function), so it cannot be told apart;
/// or when none of its segments can be hashed.
pub(crate) fn maths_library() -> Option<Loaded> {
    static MATHS: OnceLock<Option<Loaded>> = OnceLock::new();
    MATHS.get_or_init(read).clone()
}

/// Asks the loader where `sin` is, then walks the loaded objects for the
/// one that holds it.
fn read() -> Option<Loaded> {
    // SAFETY: `MATHS_SYMBOL` is NUL-terminated, and with `RTLD_DEFAULT`
    // `dlsym` only looks the name up in the process's global scope.
    let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, MATHS_SYMBOL.as_ptr()) };
    if address.is_null() {
        return None;
    }
    let mut search = Search {
        address: address as usize,
        found: None,
    };
    // SAFETY: `visit` has the signature `dl_iterate_phdr` calls, and
    // `data` points at `search`, which outlives the call; nothing else
    // reads or writes it meanwhile.
    unsafe { libc::dl_iterate_phdr(Some(visit), (&raw mut search).cast()) };
    search.found.flatten()
}

/// A walk of the loaded objects for the one holding `address`.
struct Search {
    /// The address looked for.
    address: usize,
    /// None until an object holds it; then that object, or none when it
    /// cannot be named.
    found: Option<Option<Loaded>>,
}

/// One loaded object of the walk: when it holds the address, records it
/// in the `Search` that `data` points at and stops the walk (1); else
/// goes on (0).
///
/// # Safety
///
/// `info` must point at a valid `dl_phdr_info` and `data` at a `Search`
/// that nothing else uses, both for the call, as `dl_iterate_phdr` gives
/// them to the callback [`read`] passes.
unsafe extern "C" fn visit(
    info: *mut libc::dl_phdr_info,
    _size: usize,
    data: *mut c_void,
) -> c_int {
    // SAFETY: as the caller promises, `info` points at a valid
    // `dl_phdr_info` for the call.
    let info = unsafe { &*info };
    // SAFETY: as the caller promises, `data` points at the walk's
    // `Search`, which nothing else uses meanwhile.
    let search = unsafe { &mut *data.cast::<Search>() };
    if info.dlpi_phdr.is_null() {
        return 0;
    }
    // SAFETY: `dlpi_phdr` is non-null and points at the object's
    // `dlpi_phnum` program headers, which the loader keeps while the
    // object is loaded; it cannot be unloaded during the walk.
    let headers =
        unsafe { std::slice::from_raw_parts(info.dlpi_phdr, usize::from(info.dlpi_phnum)) };
    let segments: Vec<Segment> = headers.iter().map(segment).collect();
    let base = info.dlpi_addr as usize;
    if !library::holds(&segments, base, search.address) {
        return 0;
    }
    search.found = Some(name(info).and_then(|name| {
        let hashed = segments.iter().filter(|s| s.hashed());
        let contents: Option<Vec<(usize, &[u8])>> =
            hashed.map(|s| Some((s.start, bytes(info, s)?))).collect();
        // An object with nothing to hash would be named by the hash of
        // nothing, the same for every such object.
        let contents = contents.filter(|contents| !contents.is_empty());
        Some(Loaded {
            name,
            hash: library::content_hash(contents?),
        })
    }));
    1
}

/// The object's name, the path the loader opened it by; `None` for the
/// program itself, whose name glibc leaves empty, or one that is not
/// UTF-8.
fn name(info: &libc::dl_phdr_info) -> Option<String> {
    if info.dlpi_name.is_null() {
        return None;
    }
    // SAFETY: `dlpi_name` is non-null and points at a NUL-terminated
    // string the loader keeps while the object is loaded.
    let name = unsafe { CStr::from_ptr(info.dlpi_name) };
    name.to_str()
        .ok()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// A program header of this process's word size.
#[cfg(target_pointer_width = "64")]
type Header = libc::Elf64_Phdr;
/// A program header of this process's word size.
#[cfg(target_pointer_width = "32")]
type Header = libc::Elf32_Phdr;

/// What a program header says of its segment; a size past the last
/// address is the last address, which no segment then fits under.
fn segment(header: &Header) -> Segment {
    Segment {
        loaded: header.p_type == libc::PT_LOAD,
        start: usize::try_from(header.p_vaddr).unwrap_or(usize::MAX),
        memory: usize::try_from(header.p_memsz).unwrap_or(usize::MAX),
        file: usize::try_from(header.p_filesz).unwrap_or(usize::MAX),
        readable: header.p_flags & libc::PF_R != 0,
        writable: header.p_flags & libc::PF_W != 0,
    }
}

/// The bytes of the segment that came from the file, where the object
/// `info` describes holds them, for as long as the walk lends `info`;
/// `None` when the header says more file bytes than the segment holds in
/// memory, or they would end past the last address.
fn bytes<'a>(info: &'a libc::dl_phdr_info, segment: &Segment) -> Option<&'a [u8]> {
    let start = (info.dlpi_addr as usize).checked_add(segment.start)?;
    start.checked_add(segment.file)?;
    (segment.file <= segment.memory).then_some(())?;
    // SAFETY: the loader maps a loaded segment's `memory` bytes at
    // `start`, readable as the caller hashes only readable ones,, its `file` bytes first, and keeps them mapped while the
    // object is loaded, which it stays during the walk, as `info` lives.
    // The segment is never written, so the bytes do not change.
    Some(unsafe { std::slice::from_raw_parts(start as *const u8, segment.file) })
}

#[cfg(test)]
#[path = "../tests/unit/loader.rs"]
mod tests;
