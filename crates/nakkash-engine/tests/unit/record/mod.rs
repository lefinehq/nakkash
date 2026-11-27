//! Unit tests for `src/record/mod.rs`: the wording of every record error,
//! pinned once, and the scratch folders the record's tests write in.

use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::Image;

/// A scratch folder for the record test `name`, empty.
pub(crate) fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nakkash-record-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create a scratch folder");
    dir
}

/// Writes a 3 by 2 frame with a shown mask as frame 0 of `dir`.
fn fresh(dir: &Path) {
    let frame = Drawn {
        ours: Image::new(3, 2, vec![7; 24]).unwrap(),
        coverage: vec![true; 6],
        shown: Some(vec![false; 6]),
    };
    write_frame(dir, 0, &frame).unwrap();
}

/// The error of reading frame 0 of `dir`, freshly written, then rewritten
/// by `damage`.
fn damaged(dir: &Path, file: &str, damage: impl FnOnce(&mut Vec<u8>)) -> String {
    fresh(dir);
    let path = dir.join(file);
    let mut bytes = fs::read(&path).unwrap();
    damage(&mut bytes);
    fs::write(&path, bytes).unwrap();
    read_frame(dir, 0, 3, 2, true).unwrap_err().to_string()
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
            read_frame(&dir, 0, 2, 3, false).unwrap_err().to_string()
        }),
        (
            "damaged payload",
            damaged(&dir, "f0000.ours.lz4", |b| {
                *b = pixels::tests::with_header(pixels::Content::Pixels, &[1, 2, 3])
            }),
        ),
        (
            "content hash",
            damaged(&dir, "f0000.shown", |b| *b.last_mut().unwrap() ^= 1),
        ),
        (
            "mask of another size",
            write_frame(&dir, 1, &narrow).unwrap_err().to_string(),
        ),
        ("record", {
            fs::write(dir.join(RECORD), "{}").unwrap();
            read_sample(&dir).unwrap_err().to_string()
        }),
    ];
    let text = errors
        .iter()
        .map(|(what, error)| format!("{what}: {error}"))
        .collect::<Vec<_>>()
        .join("\n")
        .replace(&dir.display().to_string(), "<dir>");
    insta::assert_snapshot!(text);
}
