//! Unit tests for `src/record/sample.rs`: a sample's record written whole
//! or not at all, read back as written, unrounded numbers included, and
//! refused with its path when it holds a key it does not know.

use std::fs;

use proptest::prelude::*;
use rstest::rstest;

use super::*;
use crate::diff::{Error, FrameDiff};
use crate::record::tests::scratch;
use crate::record::{Build, FrameRecord, Hex128, Platform, Schema, Times};
use nakkash_format::Kind;

/// The error over `pixels` pixels, `mean` from 0 to 1.
fn error(pixels: u64, mean: f64) -> Error {
    Error {
        pixels,
        mean,
        psnr: (mean > 0.0).then(|| 20.0 * (1.0 / mean).log10()),
        failing: mean / 2.0,
    }
}

/// A frame of a record with every number given, its errors `mean`.
fn frame(index: usize, mean: f64, shown: bool) -> FrameRecord {
    let file = format!("f{:04}.png", index + 1);
    FrameRecord {
        index,
        file: file.clone(),
        time: index as f64 / 25.0,
        ours: Hex128(0x1111 + index as u128),
        bare: Hex128(0x2222 + index as u128),
        shown: shown.then_some(Hex128(0x3333)),
        ours_pixels: Hex128(u128::MAX - index as u128),
        coverage: Hex128(0x4444),
        shown_mask: shown.then_some(Hex128(0x5555)),
        diff: FrameDiff {
            file,
            time: index as f64 / 25.0,
            content: error(6, mean),
            drawn: error(4, mean * 2.0),
            union: error(5, 0.0),
        },
        ms: Times {
            draw: 12.5,
            coverage: 3.25,
            shown: if shown { 2.0 } else { 0.0 },
        },
    }
}

/// A record of two frames, the first with ignored layers shown, on a
/// platform and build given here, not this machine's.
fn record() -> SampleRecord {
    SampleRecord {
        schema: Schema::V1,
        sample: "m001".to_owned(),
        platform: Platform {
            target: "aarch64-macos".to_owned(),
            simd: "neon".to_owned(),
        },
        build: Build {
            commit: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
            tree: Some(Hex128(0xd1f7)),
            profile: "release".to_owned(),
        },
        entry: Hex128(0xfeed),
        frames: vec![
            frame(0, 0.0123456789012345, true),
            frame(1, 0.1 + 0.2, false),
        ],
    }
}

#[rstest]
#[case::top("\"schema\"", "\"extra\":1,\"schema\"")]
#[case::frame("\"index\"", "\"extra\":1,\"index\"")]
#[case::build("\"profile\"", "\"extra\":1,\"profile\"")]
#[case::error("\"pixels\"", "\"extra\":1,\"pixels\"")]
#[case::times("\"draw\"", "\"extra\":1,\"draw\"")]
fn a_sample_record_round_trips_and_refuses_unknown_fields(
    #[case] key: &str,
    #[case] with_extra: &str,
) {
    let dir = scratch(&format!("record-{}", key.trim_matches('"')));
    write_sample(&dir, &record()).unwrap();
    let path = dir.join(RECORD);
    let json = fs::read_to_string(&path).unwrap();
    insta::assert_snapshot!("record", json);
    assert_eq!(read_sample(&dir).unwrap(), record());

    let compact: serde_json::Value = serde_json::from_str(&json).unwrap();
    let compact = serde_json::to_string(&compact).unwrap();
    assert!(compact.contains(key), "{compact}");
    fs::write(&path, compact.replacen(key, with_extra, 1)).unwrap();
    let error = read_sample(&dir).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid);
    let place = error
        .path
        .as_ref()
        .expect("the error has a path")
        .to_string();
    assert!(place.ends_with(RECORD), "{place}");
    assert!(error.message.contains("unknown field `extra`"), "{error}");
    assert!(error.expected.is_some());
}

#[rstest]
#[case::schema("\"nakkash.FidelityRecord.1\"", "\"nakkash.FidelityRecord.2\"")]
#[case::short_hash("\"0000000000000000000000000000feed\"", "\"feed\"")]
#[case::upper_hash(
    "\"0000000000000000000000000000feed\"",
    "\"0000000000000000000000000000FEED\""
)]
#[case::missing_key("\"sample\": \"m001\",", "")]
#[case::not_json("\"frames\"", "\"frames")]
fn a_record_of_another_form_is_invalid(#[case] from: &str, #[case] to: &str) {
    let dir = scratch(&format!("form-{}", to.len()));
    write_sample(&dir, &record()).unwrap();
    let path = dir.join(RECORD);
    let json = fs::read_to_string(&path).unwrap();
    assert!(json.contains(from), "{json}");
    fs::write(&path, json.replacen(from, to, 1)).unwrap();
    assert_eq!(read_sample(&dir).unwrap_err().kind, Kind::Invalid);
}

/// A frame's index is its place, and its file and time are its errors'.
#[rstest]
#[case::index("index", |f: &mut FrameRecord| f.index = 0)]
#[case::file("file", |f: &mut FrameRecord| f.file = "f0009.png".to_owned())]
#[case::time("time", |f: &mut FrameRecord| f.time = f64::from_bits(f.diff.time.to_bits() + 1))]
fn a_record_whose_frames_disagree_is_invalid(
    #[case] name: &str,
    #[case] change: fn(&mut FrameRecord),
) {
    // Named by the case, never by an address, so a run names it alike.
    let dir = scratch(&format!("disagree-{name}"));
    let mut sample = record();
    change(&mut sample.frames[1]);
    write_sample(&dir, &sample).unwrap();
    let error = read_sample(&dir).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid);
    let place = error
        .path
        .as_ref()
        .expect("the error has a path")
        .to_string();
    assert!(place.ends_with(RECORD), "{place}");
    assert!(error.message.starts_with("frame 1 "), "{error}");
    assert!(error.expected.is_some());
}

/// A key whose value may be null is still required: a record that lacks
/// it was not written by this build, and is refused rather than read as
/// null.
#[rstest]
#[case::tree("\"tree\"")]
#[case::commit("\"commit\"")]
#[case::shown("\"shown\"")]
#[case::shown_mask("\"shown_mask\"")]
#[case::psnr("\"psnr\"")]
fn a_record_that_lacks_a_key_is_invalid(#[case] key: &str) {
    let dir = scratch(&format!("lacks-{}", key.trim_matches('"')));
    write_sample(&dir, &record()).unwrap();
    let path = dir.join(RECORD);
    let mut json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let name = key.trim_matches('"');
    let removed = ["/build", "/frames/0", "/frames/0/diff/content"]
        .iter()
        .filter_map(|pointer| json.pointer_mut(pointer)?.as_object_mut()?.remove(name))
        .count();
    assert_eq!(removed, 1, "{name} is in one of them");
    fs::write(&path, json.to_string()).unwrap();
    let error = read_sample(&dir).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert!(
        error.message.contains(&format!("missing field `{name}`")),
        "{error}"
    );
}

/// A recording begins by removing the folder's record, then the frame
/// files and the half-written files of recordings before it, and nothing
/// else.
#[test]
fn beginning_a_recording_removes_the_record_and_stale_frames() {
    let dir = scratch("begin");
    write_sample(&dir, &record()).unwrap();
    let stale = [
        "f0000.cov",
        "f0001.ours.lz4",
        "f0012.shown",
        "f12345.cov",
        "f0000.cov.tmp",
        "record.json.tmp",
        "scene.json.tmp",
    ];
    let kept = [
        "scene.json",
        "f0000.png",
        "f.cov",
        "fx001.cov",
        "f0000.ours",
        "g0000.cov",
        "notes.txt",
    ];
    for name in stale.iter().chain(&kept) {
        fs::write(dir.join(name), b"frame").unwrap();
    }
    fs::create_dir(dir.join("f0002.cov")).unwrap();
    begin_sample(&dir).unwrap();
    assert!(!dir.join(RECORD).exists());
    for name in stale {
        assert!(!dir.join(name).exists(), "{name} is removed");
    }
    for name in kept {
        assert!(dir.join(name).exists(), "{name} is kept");
    }
    assert!(dir.join("f0002.cov").is_dir(), "a folder is not ours");
    assert_eq!(read_sample(&dir).unwrap_err().kind, Kind::Io);
    // Again, with no record, and in a folder not made yet.
    begin_sample(&dir).unwrap();
    let new = dir.join("new");
    begin_sample(&new).unwrap();
    assert!(new.is_dir());
}

#[test]
fn a_record_that_cannot_be_removed_is_an_io_error() {
    let dir = scratch("begin-blocked");
    fs::create_dir(dir.join(RECORD)).unwrap();
    assert_eq!(begin_sample(&dir).unwrap_err().kind, Kind::Io);
}

#[test]
fn a_record_is_written_only_whole() {
    let dir = scratch("whole");
    write_sample(&dir, &record()).unwrap();
    let names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, [RECORD]);

    // A write that fails leaves the record before it, whole.
    let mut other = record();
    other.sample = "m002".to_owned();
    fs::create_dir(dir.join(format!("{RECORD}.tmp"))).unwrap();
    assert_eq!(write_sample(&dir, &other).unwrap_err().kind, Kind::Io);
    assert_eq!(read_sample(&dir).unwrap(), record());
}

#[test]
fn a_failed_write_leaves_no_record() {
    let dir = scratch("failed");
    fs::create_dir(dir.join(format!("{RECORD}.tmp"))).unwrap();
    assert_eq!(write_sample(&dir, &record()).unwrap_err().kind, Kind::Io);
    assert!(!dir.join(RECORD).exists());
    assert_eq!(read_sample(&dir).unwrap_err().kind, Kind::Io);
}

proptest! {
    /// Errors are kept unrounded, so a record compared with a fresh measure
    /// compares the numbers bit for bit.
    #[test]
    fn unrounded_numbers_read_back_bit_for_bit(
        mean in any::<f64>().prop_filter("finite", |x| x.is_finite()),
        time in any::<f64>().prop_filter("finite", |x| x.is_finite()),
    ) {
        let mut sample = record();
        sample.frames[0].diff.content.mean = mean;
        sample.frames[0].diff.drawn.psnr = Some(mean);
        sample.frames[0].time = time;
        sample.frames[0].ms.draw = time;
        let json = serde_json::to_string(&sample).unwrap();
        let read: SampleRecord = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(read.frames[0].diff.content.mean.to_bits(), mean.to_bits());
        prop_assert_eq!(read.frames[0].diff.drawn.psnr.map(f64::to_bits), Some(mean.to_bits()));
        prop_assert_eq!(read.frames[0].time.to_bits(), time.to_bits());
        prop_assert_eq!(read.frames[0].ms.draw.to_bits(), time.to_bits());
    }
}
