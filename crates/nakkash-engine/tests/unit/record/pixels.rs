//! Unit tests for `src/record/pixels.rs`: a frame's pixels and masks read
//! back bit for bit against the record that names them, and a file of
//! another run, cut short, damaged, of another frame or another kind
//! refused with its path.

use std::fs;
use std::path::Path;

use proptest::prelude::*;
use rstest::rstest;

use super::*;
use crate::record::tests::{named, scratch};

/// A 3 by 2 frame with every byte different, its coverage and, with
/// `shown`, a shown mask: six pixels, so neither mask fills its last byte.
fn drawn(shown: bool) -> Drawn {
    Drawn {
        ours: Image::new(3, 2, (0..24).collect()).unwrap(),
        coverage: vec![true, false, true, true, false, true],
        shown: shown.then(|| vec![false, true, true, false, false, true]),
    }
}

/// `drawn(true)` written as frame 0 in a scratch folder for `name`.
fn written(name: &str) -> PathBuf {
    let dir = scratch(name);
    write_frame(&dir, 0, &drawn(true)).unwrap();
    dir
}

/// The error of reading frame 0 of `dir` against the record of
/// `drawn(true)`.
fn read_error(dir: &Path) -> Diagnostic {
    read_frame(dir, &named(0, &drawn(true)), 3, 2).unwrap_err()
}

/// Asserts that `error` is invalid at a path ending in `file`.
fn assert_invalid_at(error: &Diagnostic, file: &str) {
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    let path = error
        .path
        .as_ref()
        .expect("the error has a path")
        .to_string();
    assert!(path.ends_with(file), "{path}");
    assert!(error.expected.is_some(), "{error}");
}

#[rstest]
#[case::with_shown(true)]
#[case::without_shown(false)]
fn a_frame_round_trips_bit_for_bit(#[case] shown: bool) {
    let dir = scratch(&format!("round-trip-{shown}"));
    let frame = drawn(shown);
    write_frame(&dir, 7, &frame).unwrap();
    let read = read_frame(&dir, &named(7, &frame), 3, 2).unwrap();
    assert_eq!(read.ours.pixels(), frame.ours.pixels());
    assert_eq!(read.coverage, frame.coverage);
    assert_eq!(read.shown, frame.shown);
    assert_eq!(dir.join("f0007.shown").exists(), shown);
    // Each file was written whole: nothing is left beside it.
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let expected = match shown {
        true => vec!["f0007.cov", "f0007.ours.lz4", "f0007.shown"],
        false => vec!["f0007.cov", "f0007.ours.lz4"],
    };
    assert_eq!(names, expected);
}

/// A file overwritten, whole, by the same frame of another run holds what
/// its own header names, but not what the record names, and is refused.
#[rstest]
#[case::ours("f0000.ours.lz4")]
#[case::coverage("f0000.cov")]
#[case::shown("f0000.shown")]
fn a_file_of_another_run_is_refused_against_its_record(#[case] file: &str) {
    let dir = written(&format!("other-run-{file}"));
    let other = scratch(&format!("other-run-{file}-b"));
    let frame = Drawn {
        ours: Image::new(3, 2, vec![9; 24]).unwrap(),
        coverage: vec![false; 6],
        shown: Some(vec![true; 6]),
    };
    write_frame(&other, 0, &frame).unwrap();
    fs::copy(other.join(file), dir.join(file)).unwrap();
    assert_invalid_at(&read_error(&dir), file);
    assert!(read_frame(&dir, &named(0, &frame), 3, 2).is_err());
}

/// Writing a frame again replaces its files whole.
#[test]
fn a_frame_written_again_is_read_as_written_last() {
    let dir = written("again");
    let frame = Drawn {
        ours: Image::new(3, 2, vec![9; 24]).unwrap(),
        coverage: vec![false; 6],
        shown: None,
    };
    write_frame(&dir, 0, &frame).unwrap();
    assert_eq!(read_frame(&dir, &named(0, &frame), 3, 2).unwrap(), frame);
}

/// A frame of the size of a real one, with long runs and noise, so the
/// compressed stream has more than one block.
#[test]
fn a_large_frame_round_trips_bit_for_bit() {
    let dir = scratch("large");
    let (width, height) = (640, 360);
    let pixels = (0..width * height * 4)
        .map(|i: u32| match i % 7919 < 4000 {
            true => 255,
            false => (i.wrapping_mul(2_654_435_761) >> 24) as u8,
        })
        .collect();
    let coverage: Vec<bool> = (0..width * height).map(|i| i % 3 == 0).collect();
    let frame = Drawn {
        ours: Image::new(width, height, pixels).unwrap(),
        coverage,
        shown: None,
    };
    write_frame(&dir, 1, &frame).unwrap();
    assert_eq!(
        read_frame(&dir, &named(1, &frame), width, height).unwrap(),
        frame
    );
}

/// Each header names the hash the record names for what the file holds.
#[test]
fn a_file_names_the_hash_its_record_names() {
    let dir = written("hashes");
    let record = named(0, &drawn(true));
    let header = |file: &str| {
        let bytes = fs::read(dir.join(file)).unwrap();
        let (header, payload) = split(&bytes).unwrap();
        assert_eq!(header.payload, content_hash(PAYLOAD, payload));
        Hex128(header.hash)
    };
    assert_eq!(header("f0000.ours.lz4"), record.ours_pixels);
    assert_eq!(header("f0000.cov"), record.coverage);
    assert_eq!(Some(header("f0000.shown")), record.shown_mask);
}

/// The hashes cover the frame's size: the same bytes in another shape are
/// other content.
#[test]
fn a_hash_covers_the_frame_size() {
    let bytes: Vec<u8> = (0..24).collect();
    let wide = Image::new(3, 2, bytes.clone()).unwrap();
    let tall = Image::new(2, 3, bytes).unwrap();
    assert_ne!(pixels_hash(&wide), pixels_hash(&tall));
    let mask = [true, false, true, true, false, true];
    assert_ne!(mask_hash(&mask, 3, 2), mask_hash(&mask, 2, 3));
    assert_ne!(
        mask_hash(&mask, 3, 2),
        mask_hash(&[mask, [false; 6]].concat(), 4, 3)
    );
}

#[rstest]
#[case::empty(0)]
#[case::in_the_magic(5)]
#[case::in_the_header(30)]
#[case::at_the_payload(HEADER)]
#[case::in_the_payload(HEADER + 10)]
#[case::by_one(1)]
#[case::by_four(4)]
fn a_truncated_pixel_file_is_invalid_with_its_path(#[case] cut: usize) {
    let dir = written(&format!("truncated-{cut}"));
    let path = dir.join("f0000.ours.lz4");
    let bytes = fs::read(&path).unwrap();
    // Small cuts count from the end, the rest from the start.
    let keep = match cut {
        1 | 4 => bytes.len() - cut,
        _ => cut,
    };
    fs::write(&path, &bytes[..keep]).unwrap();
    assert_invalid_at(&read_error(&dir), "f0000.ours.lz4");
}

#[rstest]
#[case::ours("f0000.ours.lz4")]
#[case::coverage("f0000.cov")]
#[case::shown("f0000.shown")]
fn a_damaged_pixel_file_is_invalid_with_its_path(#[case] file: &str) {
    let dir = written(&format!("damaged-{file}"));
    let path = dir.join(file);
    let mut bytes = fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 0x01;
    fs::write(&path, bytes).unwrap();
    assert_invalid_at(&read_error(&dir), file);
}

/// A header whose hash of the content (the last byte) or of the payload
/// (16 bytes before it) is not the file's.
#[rstest]
#[case::ours_content("f0000.ours.lz4", 1)]
#[case::ours_payload("f0000.ours.lz4", 17)]
#[case::coverage_content("f0000.cov", 1)]
#[case::coverage_payload("f0000.cov", 17)]
fn a_file_whose_hash_differs_is_invalid(#[case] file: &str, #[case] from_end: usize) {
    let dir = written(&format!("hash-{file}-{from_end}"));
    let path = dir.join(file);
    let mut bytes = fs::read(&path).unwrap();
    bytes[HEADER - from_end] ^= 0x80;
    fs::write(&path, bytes).unwrap();
    assert_invalid_at(&read_error(&dir), file);
}

/// The bytes of a pixel file of `content` for a 3 by 2 frame: a whole
/// header naming `hash` with the right payload hash, then `payload`.
pub(in crate::record) fn with_header(content: Content, hash: Hex128, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let header = Header {
        content,
        width: 3,
        height: 2,
        payload: content_hash(PAYLOAD, payload),
        hash: hash.0,
    };
    header.write(&mut std::io::Cursor::new(&mut bytes)).unwrap();
    bytes.extend_from_slice(payload);
    bytes
}

/// The lz4 frame of `bytes`.
pub(in crate::record) fn compressed(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = FrameEncoder::new(Vec::new());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// Payloads whose hash is right, in files whose header names what the
/// record names, that still do not hold the frame: bits set past the last
/// pixel (written as zeros and read only as zeros, so each mask has one
/// file), a mask or pixels too short or too long, a payload that is not
/// lz4, and a whole frame other than the one the header names.
#[rstest]
#[case::past_the_pixels("f0000.cov", Content::Mask, vec![0b0110_1101])]
#[case::short_mask("f0000.cov", Content::Mask, vec![])]
#[case::long_mask("f0000.cov", Content::Mask, vec![0b0010_1101, 0])]
#[case::other_mask("f0000.cov", Content::Mask, vec![0b0010_1100])]
#[case::short_pixels("f0000.ours.lz4", Content::Pixels, compressed(&[0; 23]))]
#[case::long_pixels("f0000.ours.lz4", Content::Pixels, compressed(&[0; 25]))]
#[case::not_lz4("f0000.ours.lz4", Content::Pixels, vec![1, 2, 3])]
#[case::other_pixels("f0000.ours.lz4", Content::Pixels, compressed(&[0; 24]))]
fn a_payload_that_is_not_the_frame_is_invalid(
    #[case] file: &str,
    #[case] content: Content,
    #[case] payload: Vec<u8>,
) {
    let dir = written(&format!(
        "payload-{file}-{}-{:?}",
        payload.len(),
        payload.first()
    ));
    let record = named(0, &drawn(true));
    let hash = match content {
        Content::Pixels => record.ours_pixels,
        Content::Mask => record.coverage,
    };
    fs::write(dir.join(file), with_header(content, hash, &payload)).unwrap();
    assert_invalid_at(&read_error(&dir), file);
}

#[rstest]
#[case::mask_for_ours("f0000.cov", "f0000.ours.lz4")]
#[case::ours_for_mask("f0000.ours.lz4", "f0000.cov")]
fn a_file_of_the_other_kind_is_invalid(#[case] from: &str, #[case] to: &str) {
    let dir = written(&format!("kind-{to}"));
    fs::copy(dir.join(from), dir.join(to)).unwrap();
    assert_invalid_at(&read_error(&dir), to);
}

#[rstest]
#[case::narrower(2, 2)]
#[case::wider(4, 2)]
#[case::shorter(3, 1)]
fn a_file_of_another_size_is_invalid(#[case] width: u32, #[case] height: u32) {
    let dir = written(&format!("size-{width}x{height}"));
    let error = read_frame(&dir, &named(0, &drawn(true)), width, height).unwrap_err();
    assert_invalid_at(&error, "f0000.ours.lz4");
}

#[test]
fn a_missing_file_is_an_io_error() {
    let dir = scratch("missing");
    write_frame(&dir, 0, &drawn(false)).unwrap();
    assert_eq!(read_error(&dir).kind, Kind::Io);
    let other = named(1, &drawn(false));
    assert_eq!(read_frame(&dir, &other, 3, 2).unwrap_err().kind, Kind::Io);
}

#[rstest]
#[case::coverage(vec![true; 5], None)]
#[case::shown(vec![true; 6], Some(vec![false; 7]))]
fn a_mask_of_another_size_is_not_written(
    #[case] coverage: Vec<bool>,
    #[case] shown: Option<Vec<bool>>,
) {
    let dir = scratch(&format!("unwritten-{}", coverage.len()));
    let frame = Drawn {
        ours: drawn(false).ours,
        coverage,
        shown,
    };
    let error = write_frame(&dir, 0, &frame).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid);
    assert!(error.expected.is_some());
    assert!(!dir.join("f0000.ours.lz4").exists());
}

proptest! {
    #[test]
    fn reading_any_bytes_as_a_pixel_file_never_panics(
        bytes in any::<Vec<u8>>(),
        hash in any::<u128>(),
    ) {
        let _ = pixels_of(&bytes, 3, 2, hash);
        let _ = mask_of(&bytes, 3, 2, hash);
    }

    /// Past a whole header with the right hashes, so the decoder and the
    /// mask's checks see them.
    #[test]
    fn reading_any_payload_never_panics(payload in any::<Vec<u8>>(), hash in any::<u128>()) {
        let _ = pixels_of(&with_header(Content::Pixels, Hex128(hash), &payload), 3, 2, hash);
        let _ = mask_of(&with_header(Content::Mask, Hex128(hash), &payload), 3, 2, hash);
    }
}
