//! End-to-end tests of `nakkash frames`: a recorded sample's frames drawn
//! again from its folder alone, each printed as one JSON line with the
//! content hashes its record names; an index past the sample, a folder
//! that is not a recorded sample, and drawing other than the reference
//! way are refused.

mod support;

use std::path::{Path, PathBuf};

use nakkash_engine::record::read_sample;
use pretty_assertions::assert_eq;
use rstest::rstest;
use serde_json::Value;

use support::{arg, nakkash, scratch, star_manifest, stderr, stdout};

/// Records the star manifest under `dir`; the records' folder.
fn recorded(dir: &Path) -> PathBuf {
    let manifest = star_manifest(dir);
    let records = dir.join("records");
    let out = nakkash(&[
        "fidelity",
        arg(&manifest),
        "--work",
        arg(dir),
        "--record",
        arg(&records),
        "--record-cap",
        "1",
    ]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    records
}

#[test]
fn frames_prints_the_recorded_hashes() {
    let records = recorded(&scratch("frames-hashes"));
    for id in ["star", "hidden"] {
        let folder = records.join(id);
        let out = nakkash(&["frames", arg(&folder), "--frames", "2,0,1"]);
        assert!(out.status.success(), "{}", stderr(&out));
        let lines: Vec<Value> = stdout(&out)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let record = read_sample(&folder).unwrap();
        assert_eq!(lines.len(), record.frames.len(), "{id}");
        for (line, frame) in lines.iter().zip(&record.frames) {
            let shown = frame.shown_mask.map(|hash| hash.to_string());
            assert_eq!(line["index"], frame.index, "{id}: in index order");
            assert_eq!(line["ours"], frame.ours_pixels.to_string(), "{id}");
            assert_eq!(line["coverage"], frame.coverage.to_string(), "{id}");
            assert_eq!(line["shown"], serde_json::json!(shown), "{id}");
            let ms = line["ms"].as_object().unwrap();
            let keys: Vec<&str> = ms.keys().map(String::as_str).collect();
            assert_eq!(keys, ["draw", "coverage", "shown"], "{id}");
            assert!(
                ms.values()
                    .all(|ms| ms.as_f64().is_some_and(|ms| ms >= 0.0))
            );
        }
    }
}

/// Every index is checked before any frame is drawn, so nothing is
/// printed for a list that names one past the sample.
#[test]
fn frames_refuses_an_index_past_the_sample() {
    let records = recorded(&scratch("frames-past"));
    let out = nakkash(&["frames", arg(&records.join("star")), "--frames", "0,3"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout(&out), "");
    insta::assert_snapshot!(stderr(&out));
    // An index named twice is drawn once.
    let out = nakkash(&["frames", arg(&records.join("star")), "--frames", "1,1"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out).lines().count(), 1);
}

/// A list that is not indices is a usage error, before anything is read.
#[rstest]
#[case::an_empty_item("0,,3")]
#[case::a_word("x")]
#[case::nothing("")]
#[case::negative("-1")]
fn frames_refuses_a_list_that_is_not_indices(#[case] list: &str) {
    let out = nakkash(&["frames", "no-such-sample", "--frames", list]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout(&out), "");
    let err = stderr(&out);
    assert!(
        err.starts_with("error: ") && err.contains("--frames"),
        "{err}"
    );
}

/// `text` with the first hash after each `marker` written as `name`.
fn redacted(text: &str, marker: &str, name: &str) -> String {
    match text.find(marker) {
        Some(at) => {
            let start = at + marker.len();
            format!("{}{name}{}", &text[..start], &text[start + 32..])
        }
        None => text.to_owned(),
    }
}

/// The folder's scene and replay are read only against its record: a
/// scene a later run wrote over the recorded one is refused, naming the
/// file, and so is a folder whose record is gone; nothing is drawn.
#[test]
fn frames_refuses_files_its_record_does_not_name() {
    let records = recorded(&scratch("frames-unnamed"));
    let folder = records.join("star");
    let scene = folder.join("scene.json");
    let text = std::fs::read_to_string(&scene).unwrap();
    std::fs::write(&scene, text.replacen('{', "{\"bg\":\"#000000\",", 1)).unwrap();
    let out = nakkash(&["frames", arg(&folder), "--frames", "0"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout(&out), "");
    let err = stderr(&out).replace(&records.display().to_string(), "<records>");
    let err = redacted(
        &redacted(&err, "hashes to ", "<found>"),
        "not to the ",
        "<recorded>",
    );
    insta::assert_snapshot!(err);
    std::fs::write(&scene, text).unwrap();
    assert!(
        nakkash(&["frames", arg(&folder), "--frames", "0"])
            .status
            .success()
    );
    std::fs::remove_file(folder.join("record.json")).unwrap();
    let out = nakkash(&["frames", arg(&folder), "--frames", "0"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout(&out), "");
    let err = stderr(&out);
    assert!(
        err.starts_with("error: cannot read `") && err.contains("record.json"),
        "{err}"
    );
}

/// A frame is drawn as its fidelity run drew it: on the CPU, at precise
/// quality; asked otherwise, it refuses before reading the sample.
#[rstest]
#[case::on_the_gpu("gpu", &["--gpu"])]
#[case::at_fast_quality("fast", &["--quality", "fast"])]
fn frames_draws_only_as_the_reference_does(#[case] name: &str, #[case] extra: &[&str]) {
    let mut args = vec!["frames", "sample", "--frames", "0"];
    args.extend_from_slice(extra);
    let out = nakkash(&args);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout(&out), "");
    insta::assert_snapshot!(format!("frames_refuses_{name}"), stderr(&out));
}

/// A folder without a recorded sample's files is an I/O error naming the
/// record it lacks.
#[test]
fn frames_needs_a_recorded_sample() {
    let dir = scratch("frames-empty");
    let out = nakkash(&["frames", arg(&dir), "--frames", "0", "--quality", "precise"]);
    assert_eq!(out.status.code(), Some(2));
    let err = stderr(&out);
    assert!(err.starts_with("error: cannot read `"), "{err}");
    assert!(err.contains("record.json"), "{err}");
}
