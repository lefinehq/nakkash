//! Unit tests for `src/record/pixels.rs`: a frame's pixels and masks read
//! back bit for bit, and a file that is cut short, damaged, of another
//! frame or another kind is refused with its path.

use std::fs;
use std::path::Path;

use proptest::prelude::*;
use rstest::rstest;

use super::*;
use crate::record::tests::scratch;

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

/// The error of reading frame 0 of `dir` as `drawn(true)`.
fn read_error(dir: &Path) -> Diagnostic {
    read_frame(dir, 0, 3, 2, true).unwrap_err()
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
    let read = read_frame(&dir, 7, 3, 2, shown).unwrap();
    assert_eq!(read.ours.pixels(), frame.ours.pixels());
    assert_eq!(read.coverage, frame.coverage);
    assert_eq!(read.shown, frame.shown);
    assert_eq!(dir.join("f0007.shown").exists(), shown);
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
    assert_eq!(read_frame(&dir, 1, width, height, false).unwrap(), frame);
}

/// A mask file's header gives the hash the record names for the mask; a
/// file of our pixels, the hash of them compressed.
#[test]
fn a_file_names_the_hash_of_its_payload() {
    let dir = written("hashes");
    let frame = drawn(true);
    let header = |file: &str| {
        let bytes = fs::read(dir.join(file)).unwrap();
        let (header, payload) = split(&bytes).unwrap();
        (header.hash, content_hash(header.content.tag(), payload))
    };
    let (ours, compressed) = header("f0000.ours.lz4");
    assert_eq!(ours, compressed);
    assert_ne!(Hex128(ours), pixels_hash(&frame.ours));
    assert_eq!(Hex128(header("f0000.cov").0), mask_hash(&frame.coverage));
    assert_eq!(
        Hex128(header("f0000.shown").0),
        mask_hash(frame.shown.as_ref().unwrap())
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

/// A hash in the header that is not the hash of the payload.
#[rstest]
#[case::ours("f0000.ours.lz4")]
#[case::coverage("f0000.cov")]
fn a_file_whose_hash_differs_is_invalid(#[case] file: &str) {
    let dir = written(&format!("hash-{file}"));
    let path = dir.join(file);
    let mut bytes = fs::read(&path).unwrap();
    bytes[HEADER - 1] ^= 0x80;
    fs::write(&path, bytes).unwrap();
    assert_invalid_at(&read_error(&dir), file);
}

/// The bytes of a pixel file of `content` for a 3 by 2 frame: a whole
/// header with the right hash, then `payload`.
pub(in crate::record) fn with_header(content: Content, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let header = Header {
        content,
        width: 3,
        height: 2,
        hash: content_hash(content.tag(), payload),
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

/// Payloads whose hash is right that still do not hold the frame: bits set
/// past the last pixel (written as zeros and read only as zeros, so each
/// mask has one file), a mask or pixels too short or too long, and a
/// payload that is not lz4.
#[rstest]
#[case::past_the_pixels("f0000.cov", Content::Mask, vec![0b0100_0000])]
#[case::short_mask("f0000.cov", Content::Mask, vec![])]
#[case::long_mask("f0000.cov", Content::Mask, vec![0, 0])]
#[case::short_pixels("f0000.ours.lz4", Content::Pixels, compressed(&[0; 23]))]
#[case::long_pixels("f0000.ours.lz4", Content::Pixels, compressed(&[0; 25]))]
#[case::not_lz4("f0000.ours.lz4", Content::Pixels, vec![1, 2, 3])]
fn a_payload_that_is_not_the_frame_is_invalid(
    #[case] file: &str,
    #[case] content: Content,
    #[case] payload: Vec<u8>,
) {
    let dir = written(&format!("payload-{file}-{}", payload.len()));
    fs::write(dir.join(file), with_header(content, &payload)).unwrap();
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
    let error = read_frame(&dir, 0, width, height, true).unwrap_err();
    assert_invalid_at(&error, "f0000.ours.lz4");
}

#[test]
fn a_missing_file_is_an_io_error() {
    let dir = scratch("missing");
    write_frame(&dir, 0, &drawn(false)).unwrap();
    assert_eq!(read_error(&dir).kind, Kind::Io);
    assert_eq!(read_frame(&dir, 1, 3, 2, false).unwrap_err().kind, Kind::Io);
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
    fn reading_any_bytes_as_a_pixel_file_never_panics(bytes in any::<Vec<u8>>()) {
        let _ = pixels_of(&bytes, 3, 2);
        let _ = mask_of(&bytes, 3, 2);
    }

    /// Past a whole header with the right hash, so the decoder and the
    /// mask's checks see them.
    #[test]
    fn reading_any_payload_never_panics(payload in any::<Vec<u8>>()) {
        let _ = pixels_of(&with_header(Content::Pixels, &payload), 3, 2);
        let _ = mask_of(&with_header(Content::Mask, &payload), 3, 2);
    }
}
