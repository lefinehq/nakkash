//! Unit tests for `src/library.rs`: which loaded object holds an address,
//! which of its segments are its content, and a content hash that changes
//! with every byte, start and boundary of those segments, never with
//! anything else.

use rstest::rstest;

use super::*;

/// A loaded, readable segment at `start`, `memory` bytes long, all from
/// the file.
fn code(start: usize, memory: usize) -> Segment {
    Segment {
        loaded: true,
        start,
        memory,
        file: memory,
        readable: true,
        writable: false,
    }
}

/// The segments of a small library: its header and constants, its code,
/// then its data, which holds more in memory than in the file, and a note
/// that is not mapped apart, lying inside the first segment.
fn library() -> [Segment; 4] {
    [
        code(0, 0x1000),
        code(0x1000, 0x3000),
        Segment {
            writable: true,
            file: 0x100,
            ..code(0x5000, 0x800)
        },
        Segment {
            loaded: false,
            ..code(0x200, 0x24)
        },
    ]
}

/// An object holds an address that one of its mapped segments spans,
/// after its load address; a gap between segments, an end and a header
/// not mapped apart hold nothing.
#[rstest]
#[case::first_byte(0x7000_0000, true)]
#[case::in_the_header(0x7000_0210, true)]
#[case::in_the_code(0x7000_2345, true)]
#[case::last_byte_of_the_code(0x7000_3fff, true)]
#[case::in_the_gap(0x7000_4000, false)]
#[case::in_the_data(0x7000_57ff, true)]
#[case::past_the_data(0x7000_5800, false)]
#[case::before_the_object(0x6fff_ffff, false)]
#[case::far_away(0x1234, false)]
fn an_object_holds_what_its_mapped_segments_span(#[case] address: usize, #[case] held: bool) {
    assert_eq!(holds(&library(), 0x7000_0000, address), held);
}

/// A note that is not mapped apart holds nothing by itself, even where no
/// mapped segment lies around it.
#[test]
fn a_segment_not_mapped_holds_nothing() {
    let note = Segment {
        loaded: false,
        ..code(0x9000, 0x100)
    };
    assert!(!holds(&[note], 0, 0x9010));
    assert!(!holds(&[], 0, 0));
}

/// A segment whose end would pass the last address holds nothing, rather
/// than wrapping round to the first addresses.
#[rstest]
#[case::start_past_the_end(usize::MAX, code(1, 0x10), 0)]
#[case::end_past_the_end(usize::MAX - 0x10, code(0, 0x100), usize::MAX - 1)]
#[case::wrapped_round(usize::MAX - 0x10, code(0, 0x100), 0x20)]
fn a_segment_past_the_last_address_holds_nothing(
    #[case] base: usize,
    #[case] segment: Segment,
    #[case] address: usize,
) {
    assert!(!holds(&[segment], base, address));
}

/// Only mapped segments the process reads and never writes are the
/// library's content: its code and constants, not its data, a header not
/// mapped apart, or a segment mapped only to run.
#[rstest]
#[case::code(code(0, 0x10), true)]
#[case::data(Segment { writable: true, ..code(0, 0x10) }, false)]
#[case::not_mapped(Segment { loaded: false, ..code(0, 0x10) }, false)]
#[case::execute_only(Segment { readable: false, ..code(0, 0x10) }, false)]
fn a_segment_is_content_when_mapped_readable_and_never_written(
    #[case] segment: Segment,
    #[case] hashed: bool,
) {
    assert_eq!(segment.hashed(), hashed);
}

#[test]
fn the_small_library_hashes_its_first_two_segments() {
    let hashed: Vec<usize> = (library().iter())
        .filter(|s| s.hashed())
        .map(|s| s.start)
        .collect();
    assert_eq!(hashed, [0, 0x1000]);
}

/// Where the function called starts in the library.
const ENTRY: usize = 0x1004;

/// The two read-only segments of a library, as a loader would lend them.
const CONSTANTS: &[u8] = b"\x7fELF constants";
const CODE: &[u8] = b"sin cos exp";

#[test]
fn the_same_library_hashes_the_same() {
    let parts = || [(0, CONSTANTS), (0x1000, CODE)];
    assert_eq!(content_hash(ENTRY, parts()), content_hash(ENTRY, parts()));
}

/// The hash is pinned: a change to its tag or form names every library
/// anew, so every fidelity record made before it is refused once.
#[test]
fn the_content_hash_is_pinned() {
    let hash = content_hash(ENTRY, [(0, CONSTANTS), (0x1000, CODE)]);
    insta::assert_snapshot!(format!("{hash:032x}"));
}

/// Every byte, start and boundary of the hashed segments changes the hash:
/// a patched library is another library, wherever the patch lies.
#[rstest]
#[case::a_byte_of_the_code([(0, CONSTANTS), (0x1000, &b"sin cos exq"[..])])]
#[case::a_byte_of_the_constants([(0, &b"\x7fELF constantz"[..]), (0x1000, CODE)])]
#[case::a_moved_segment([(0, CONSTANTS), (0x2000, CODE)])]
#[case::a_byte_moved_between_segments([
    (0, &b"\x7fELF constantss"[..]),
    (0x1000, &b"in cos exp"[..]),
])]
#[case::segments_swapped([(0x1000, CODE), (0, CONSTANTS)])]
fn every_byte_start_and_boundary_changes_the_hash(#[case] parts: [(usize, &[u8]); 2]) {
    assert_ne!(
        content_hash(ENTRY, parts),
        content_hash(ENTRY, [(0, CONSTANTS), (0x1000, CODE)])
    );
}

/// Where the function called starts changes the hash: the same library
/// calling another version of the function for the processor (with fused
/// multiply-adds or without) may round to other bits.
#[test]
fn where_the_function_called_starts_changes_the_hash() {
    let parts = || [(0, CONSTANTS), (0x1000, CODE)];
    assert_ne!(content_hash(0x1008, parts()), content_hash(ENTRY, parts()));
}

/// A segment more or less is another library, even one with no bytes.
#[test]
fn the_number_of_segments_changes_the_hash() {
    let two = content_hash(ENTRY, [(0, CONSTANTS), (0x1000, CODE)]);
    assert_ne!(content_hash(ENTRY, [(0, CONSTANTS)]), two);
    assert_ne!(
        content_hash(ENTRY, [(0, CONSTANTS), (0x1000, CODE), (0x5000, &[][..])]),
        two
    );
}

/// The hash is tagged as a library's: the same bytes hashed as another
/// kind of content hash apart.
#[test]
fn a_library_hashes_apart_from_other_content() {
    assert_ne!(
        content_hash(ENTRY, [(0, CODE)]),
        nakkash_hash::content_hash(TAG, CODE)
    );
    assert_ne!(
        content_hash(ENTRY, [(0, CODE)]),
        nakkash_hash::ContentHasher::new("nakkash.build.codegen")
            .u64(ENTRY as u64)
            .u64(0)
            .bytes(CODE)
            .finish()
    );
}
