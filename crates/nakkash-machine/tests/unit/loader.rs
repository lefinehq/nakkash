//! Unit tests for `src/loader.rs`, on Linux with glibc: the maths library
//! is found where the loader keeps it, named by its content the same at
//! every call, and a program header reads as the segment it describes.

use rstest::rstest;

use super::*;

/// The process calls its maths from glibc's maths library, a shared object
/// apart from the program, which the loader names.
#[test]
fn the_maths_library_is_the_shared_object_that_defines_sin() {
    // Call the library, so the test program surely loads it.
    assert!(std::hint::black_box(0.5_f64).sin() > 0.0);
    let maths = maths_library().expect("glibc's maths library is loaded");
    assert!(maths.name.contains("libm.so"), "{maths:?}");
    assert_eq!(maths_library(), Some(maths.clone()));
    assert_eq!(read(), Some(maths));
}

/// A program header of each kind, with each permission.
fn header(kind: u32, flags: u32) -> Header {
    Header {
        p_type: kind,
        p_flags: flags,
        p_offset: 0x1000,
        p_vaddr: 0x1000,
        p_paddr: 0x1000,
        p_filesz: 0x80,
        p_memsz: 0x100,
        p_align: 0x1000,
    }
}

/// A program header reads as the segment it describes: mapped only when
/// it is `PT_LOAD`, readable and writable by its flags.
#[rstest]
#[case::code(header(libc::PT_LOAD, libc::PF_R | libc::PF_X), true, true, false)]
#[case::constants(header(libc::PT_LOAD, libc::PF_R), true, true, false)]
#[case::data(header(libc::PT_LOAD, libc::PF_R | libc::PF_W), true, true, true)]
#[case::execute_only(header(libc::PT_LOAD, libc::PF_X), true, false, false)]
#[case::note(header(libc::PT_NOTE, libc::PF_R), false, true, false)]
#[case::dynamic(header(libc::PT_DYNAMIC, libc::PF_R | libc::PF_W), false, true, true)]
fn a_program_header_reads_as_its_segment(
    #[case] header: Header,
    #[case] loaded: bool,
    #[case] readable: bool,
    #[case] writable: bool,
) {
    assert_eq!(
        segment(&header),
        Segment {
            loaded,
            start: 0x1000,
            memory: 0x100,
            file: 0x80,
            readable,
            writable,
        }
    );
}

/// The description of an object loaded at `base`, with no name or
/// headers: all `bytes` reads of it.
fn loaded_at(base: usize) -> libc::dl_phdr_info {
    // SAFETY: `dl_phdr_info` holds only integers and pointers, for which
    // all zeroes are valid (null pointers).
    let mut info: libc::dl_phdr_info = unsafe { std::mem::zeroed() };
    info.dlpi_addr = base as libc::Elf64_Addr;
    info
}

/// A segment's bytes are the file's part of it, where the object holds
/// them: its load address and the segment's start.
#[test]
fn a_segment_lends_its_file_bytes() {
    let memory = *b"constantscode\0\0\0";
    let info = loaded_at(memory.as_ptr() as usize);
    let segment = Segment {
        loaded: true,
        start: 9,
        memory: 7,
        file: 4,
        readable: true,
        writable: false,
    };
    assert_eq!(bytes(&info, &segment), Some(&b"code"[..]));
}

/// A header that says more file bytes than its segment holds, or a
/// segment that would end past the last address, lends nothing.
#[rstest]
#[case::more_file_than_memory(0x1000, 0, 0x20, 0x10)]
#[case::start_past_the_end(usize::MAX, 1, 0, 0)]
#[case::end_past_the_end(usize::MAX - 4, 0, 8, 8)]
fn a_segment_that_does_not_fit_lends_nothing(
    #[case] base: usize,
    #[case] start: usize,
    #[case] file: usize,
    #[case] memory: usize,
) {
    let segment = Segment {
        loaded: true,
        start,
        memory,
        file,
        readable: true,
        writable: false,
    };
    assert_eq!(bytes(&loaded_at(base), &segment), None);
}
