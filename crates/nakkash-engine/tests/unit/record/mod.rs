//! Unit tests for `src/record/mod.rs`: the wording of every record error,
//! pinned once, and the scratch folders and records the record's tests use.

use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::Image;
use crate::diff::{Error, FrameDiff};

/// A record of frame `index` that names the hashes of `drawn`, its other
/// values zero.
pub(crate) fn named(index: usize, drawn: &Drawn) -> FrameRecord {
    let (width, height) = (drawn.ours.width(), drawn.ours.height());
    let file = format!("f{:04}.png", index + 1);
    FrameRecord {
        index,
        file: file.clone(),
        time: 0.0,
        ours: Hex128(0),
        bare: Hex128(0),
        shown: drawn.shown.as_ref().map(|_| Hex128(0)),
        ours_pixels: pixels_hash(&drawn.ours),
        coverage: mask_hash(&drawn.coverage, width, height),
        shown_mask: (drawn.shown.as_ref()).map(|mask| mask_hash(mask, width, height)),
        diff: FrameDiff {
            file,
            time: 0.0,
            content: Error::default(),
            drawn: Error::default(),
            union: Error::default(),
        },
        ms: Times::default(),
    }
}

/// A scratch folder for the record test `name`, empty.
pub(crate) fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nakkash-record-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create a scratch folder");
    dir
}

/// A 3 by 2 frame with a shown mask.
fn frame() -> Drawn {
    Drawn {
        ours: Image::new(3, 2, vec![7; 24]).unwrap(),
        coverage: vec![true; 6],
        shown: Some(vec![false; 6]),
    }
}

/// Writes [`frame`] as frame 0 of `dir`.
fn fresh(dir: &Path) {
    write_frame(dir, 0, &frame()).unwrap();
}

/// The error of reading frame 0 of `dir`, freshly written, then rewritten
/// by `damage`.
fn damaged(dir: &Path, file: &str, damage: impl FnOnce(&mut Vec<u8>)) -> String {
    fresh(dir);
    let path = dir.join(file);
    let mut bytes = fs::read(&path).unwrap();
    damage(&mut bytes);
    fs::write(&path, bytes).unwrap();
    read_frame(dir, &named(0, &frame()), 3, 2)
        .unwrap_err()
        .to_string()
}

/// The error of reading a record in `dir` whose one frame `change`
/// changes.
fn disagreeing(dir: &Path, change: impl FnOnce(&mut FrameRecord)) -> String {
    let mut frame = named(0, &frame());
    change(&mut frame);
    let record = SampleRecord {
        schema: Schema::V1,
        sample: "m001".to_owned(),
        platform: Platform::current(),
        build: Build {
            commit: None,
            tree: None,
            profile: "debug".to_owned(),
        },
        entry: Hex128(0),
        frames: vec![frame],
    };
    write_sample(dir, &record).unwrap();
    read_sample(dir).unwrap_err().to_string()
}

#[test]
fn every_error_message() {
    let dir = scratch("errors");
    let narrow = Drawn {
        ours: Image::new(3, 2, vec![0; 24]).unwrap(),
        coverage: vec![true; 5],
        shown: None,
    };
    let errors = [
        ("hash", "ABC".parse::<Hex128>().unwrap_err().to_string()),
        (
            "not a pixel file",
            damaged(&dir, "f0000.cov", |b| b[0] = b'N'),
        ),
        (
            "other kind",
            damaged(&dir, "f0000.ours.lz4", |b| {
                *b = fs::read(dir.join("f0000.cov")).unwrap()
            }),
        ),
        ("other size", {
            fresh(&dir);
            read_frame(&dir, &named(0, &frame()), 2, 3)
                .unwrap_err()
                .to_string()
        }),
        (
            "other run",
            damaged(&dir, "f0000.cov", |b| {
                let other = scratch("errors-other");
                let mut drawn = frame();
                drawn.coverage = vec![false; 6];
                write_frame(&other, 0, &drawn).unwrap();
                *b = fs::read(other.join("f0000.cov")).unwrap();
            }),
        ),
        (
            "payload hash",
            damaged(&dir, "f0000.shown", |b| *b.last_mut().unwrap() ^= 1),
        ),
        (
            "damaged payload",
            damaged(&dir, "f0000.ours.lz4", |b| {
                let hash = named(0, &frame()).ours_pixels;
                *b = pixels::tests::with_header(pixels::Content::Pixels, hash, &[1, 2, 3])
            }),
        ),
        (
            "decoded",
            damaged(&dir, "f0000.ours.lz4", |b| {
                let hash = named(0, &frame()).ours_pixels;
                let other = pixels::tests::compressed(&[0; 24]);
                *b = pixels::tests::with_header(pixels::Content::Pixels, hash, &other)
            }),
        ),
        (
            "mask of another size",
            write_frame(&dir, 1, &narrow).unwrap_err().to_string(),
        ),
        ("record", {
            fs::write(dir.join(RECORD), "{}").unwrap();
            read_sample(&dir).unwrap_err().to_string()
        }),
        ("index", disagreeing(&dir, |f| f.index = 1)),
        ("file", disagreeing(&dir, |f| f.file = "x.png".to_owned())),
        ("time", disagreeing(&dir, |f| f.time = 0.04)),
    ];
    let text = errors
        .iter()
        .map(|(what, error)| format!("{what}: {error}"))
        .collect::<Vec<_>>()
        .join("\n")
        .replace(&dir.display().to_string(), "<dir>");
    insta::assert_snapshot!(text);
}
