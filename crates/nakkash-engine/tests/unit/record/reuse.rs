//! Unit tests for `src/record/reuse.rs`: a reuse list read only on its own
//! platform, a record opened only when its build and platform allow it, and
//! each branch of the rule that decides whether a frame is read, drawn or
//! refused.

use std::fs;

use rstest::rstest;

use super::*;
use crate::diff::{Error, FrameDiff};
use crate::record::tests::scratch;
use crate::record::{Build, Schema, Times, write_sample};

/// The path of the scratch folder for `name`, made or not.
fn scratch_of(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nakkash-record-{}-{name}", std::process::id()))
}

/// An identified build, as a binary built in its checkout names itself.
fn build() -> Build {
    Build {
        commit: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
        tree: None,
        profile: "test".to_owned(),
    }
}

/// The identities of frame `index` in [`record`].
fn identities(index: usize) -> Identities {
    let index = index as u128;
    Identities {
        ours: Hex128(0x100 + index),
        bare: Hex128(0x200 + index),
        shown: (index == 1).then_some(Hex128(0x300)),
    }
}

/// The time of frame `index` in [`record`].
fn time(index: usize) -> f64 {
    index as f64 / 10.0
}

/// Frame `index` of [`record`].
fn frame(index: usize) -> FrameRecord {
    let file = format!("f{index:04}.png");
    let Identities { ours, bare, shown } = identities(index);
    FrameRecord {
        index,
        file: file.clone(),
        time: time(index),
        ours,
        bare,
        shown,
        ours_pixels: Hex128(0xa),
        coverage: Hex128(0xb),
        shown_mask: shown.map(|_| Hex128(0xc)),
        diff: FrameDiff {
            file,
            time: time(index),
            content: Error::default(),
            drawn: Error::default(),
            union: Error::default(),
        },
        ms: Times::default(),
    }
}

/// A record of three frames, made on this machine by an identified build.
fn record() -> SampleRecord {
    SampleRecord {
        schema: Schema::V1,
        sample: "m001".to_owned(),
        platform: Platform::current(),
        build: build(),
        entry: Hex128(0xe),
        frames: (0..3).map(frame).collect(),
    }
}

/// [`record`] changed by `change`, written into a scratch folder for
/// `name`, and the runner's list vouching for every frame of [`record`].
fn written(name: &str, change: impl FnOnce(&mut SampleRecord)) -> (PathBuf, SampleReuse) {
    let dir = scratch(&format!("reuse-{name}"));
    let mut changed = record();
    change(&mut changed);
    write_sample(&dir, &changed).unwrap();
    let reuse = SampleReuse::every_frame(dir.clone(), &record());
    (dir, reuse)
}

/// [`record`], written and opened for reuse.
fn opened(name: &str) -> Reusing {
    Reusing::open(&written(name, |_| {}).1).unwrap()
}

/// A reuse list for this machine with one sample, as JSON.
fn reuse_json(platform: &Platform, dir: &str) -> String {
    serde_json::json!({
        "schema": "nakkash.FidelityReuse.1",
        "platform": platform,
        "samples": {"m001": {"dir": dir, "frames": {"1": identities(1)}}},
    })
    .to_string()
}

#[test]
fn the_runner_may_vouch_for_every_frame_of_a_record() {
    let reuse = SampleReuse::every_frame(PathBuf::from("r/m001"), &record());
    assert_eq!(reuse.dir, PathBuf::from("r/m001"));
    assert_eq!(
        reuse.frames,
        BTreeMap::from([(0, identities(0)), (1, identities(1)), (2, identities(2))])
    );
    assert_eq!(frame(1).identities(), identities(1));
}

#[test]
fn a_reuse_list_reads_its_folders_from_its_own_folder() {
    let dir = scratch("reuse-list");
    let path = dir.join("reuse.json");
    fs::write(&path, reuse_json(&Platform::current(), "records/m001")).unwrap();
    let reuse = Reuse::read(&path).unwrap();
    assert_eq!(reuse.schema, ReuseSchema::V1);
    assert_eq!(reuse.platform, Platform::current());
    let sample = &reuse.samples["m001"];
    assert_eq!(sample.dir, dir.join("records/m001"));
    assert_eq!(sample.frames, BTreeMap::from([(1, identities(1))]));
    // An absolute folder stays as it is.
    fs::write(&path, reuse_json(&Platform::current(), "/records/m001")).unwrap();
    let reuse = Reuse::read(&path).unwrap();
    assert_eq!(reuse.samples["m001"].dir, PathBuf::from("/records/m001"));
}

#[test]
fn a_reuse_list_from_another_platform_is_refused() {
    let dir = scratch("reuse-platform");
    let path = dir.join("reuse.json");
    let other = Platform {
        simd: "other".to_owned(),
        ..Platform::current()
    };
    fs::write(&path, reuse_json(&other, "m001")).unwrap();
    let error = Reuse::read(&path).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert_eq!(
        error.expected.as_deref(),
        Some("a reuse list made on this platform")
    );
}

#[rstest]
#[case::unknown_key("\"samples\"", "\"extra\":1,\"samples\"")]
#[case::other_schema("FidelityReuse.1", "FidelityReuse.2")]
#[case::missing_shown(",\"shown\":null", "")]
#[case::not_an_index("{\"0\":", "{\"zero\":")]
#[case::not_json("\"samples\"", "\"samples")]
fn a_reuse_list_of_another_form_is_invalid(#[case] from: &str, #[case] to: &str) {
    let dir = scratch(&format!("reuse-form-{}", from.len() + to.len()));
    let path = dir.join("reuse.json");
    let json = serde_json::json!({
        "schema": "nakkash.FidelityReuse.1",
        "platform": Platform::current(),
        "samples": {"m001": {"dir": "m001", "frames": {"0": identities(0)}}},
    })
    .to_string();
    assert!(json.contains(from), "{json}");
    fs::write(&path, json.replacen(from, to, 1)).unwrap();
    let error = Reuse::read(&path).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert!(error.expected.is_some());
}

#[test]
fn a_missing_reuse_list_is_an_io_error() {
    let dir = scratch("reuse-missing");
    let error = Reuse::read(&dir.join("reuse.json")).unwrap_err();
    assert_eq!(error.kind, Kind::Io);
}

#[test]
fn a_record_opened_for_reuse_holds_what_its_folder_holds() {
    let reusing = opened("open");
    assert_eq!(reusing.record(), &record());
    assert_eq!(reusing.dir(), scratch_of("reuse-open"));
}

/// A record whose build does not know its code, or that was drawn on
/// another machine, is never opened for reuse.
#[rstest]
#[case::unidentified("unidentified", |r: &mut SampleRecord| r.build.commit = None)]
#[case::other_simd("other-simd", |r: &mut SampleRecord| r.platform.simd = "other".to_owned())]
#[case::other_target("other-target", |r: &mut SampleRecord| r.platform.target = "other".to_owned())]
fn a_record_that_cannot_vouch_for_its_pixels_is_refused(
    #[case] name: &str,
    #[case] change: fn(&mut SampleRecord),
) {
    let (dir, reuse) = written(name, change);
    let error = Reusing::open(&reuse).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    let place = error.path.as_ref().expect("a path").to_string();
    assert_eq!(place, dir.join(RECORD).display().to_string());
}

#[test]
fn a_folder_without_a_record_is_an_io_error() {
    let dir = scratch("reuse-no-record");
    let reuse = SampleReuse::every_frame(dir, &record());
    assert_eq!(Reusing::open(&reuse).unwrap_err().kind, Kind::Io);
}

#[test]
fn a_frame_vouched_for_with_its_own_lists_is_read() {
    let reusing = opened("read");
    for index in 0..3 {
        let found = reusing.frame(index, &identities(index), time(index));
        assert_eq!(found.unwrap(), Some(&frame(index)));
    }
}

#[test]
fn a_frame_the_runner_does_not_list_is_drawn() {
    let (_, mut reuse) = written("unlisted", |_| {});
    reuse.frames.remove(&1);
    let reusing = Reusing::open(&reuse).unwrap();
    assert_eq!(reusing.frame(1, &identities(1), time(1)).unwrap(), None);
    // Past the record too: nobody vouched for it.
    assert_eq!(reusing.frame(7, &identities(1), time(7)).unwrap(), None);
}

/// The runner vouched for other lists than the frame draws now: its code
/// or data changed, so the frame is drawn, whatever the record holds.
#[rstest]
#[case::ours(|i: &mut Identities| i.ours = Hex128(1))]
#[case::bare(|i: &mut Identities| i.bare = Hex128(1))]
#[case::shown(|i: &mut Identities| i.shown = Some(Hex128(1)))]
fn a_frame_whose_lists_changed_since_the_runner_vouched_is_drawn(
    #[case] change: fn(&mut Identities),
) {
    let reusing = opened("changed");
    for index in [0, 1] {
        let mut fresh = identities(index);
        change(&mut fresh);
        assert_eq!(reusing.frame(index, &fresh, time(index)).unwrap(), None);
    }
}

#[test]
fn a_frame_whose_shown_list_went_away_is_drawn() {
    let reusing = opened("shown-gone");
    let fresh = Identities {
        shown: None,
        ..identities(1)
    };
    assert_eq!(reusing.frame(1, &fresh, time(1)).unwrap(), None);
}

/// The runner vouched for what the frame draws now, and the record holds
/// something else: the record cannot stand in for the draw, and saying so
/// is the only safe answer.
#[rstest]
#[case::ours("ours", |f: &mut FrameRecord| f.ours = Hex128(1))]
#[case::bare("bare", |f: &mut FrameRecord| f.bare = Hex128(1))]
#[case::shown("shown", |f: &mut FrameRecord| f.shown = None)]
#[case::time("time", |f: &mut FrameRecord| { f.time += 0.05; f.diff.time = f.time; })]
fn a_vouched_frame_the_record_disagrees_with_is_invalid(
    #[case] name: &str,
    #[case] change: fn(&mut FrameRecord),
) {
    let (dir, reuse) = written(&format!("disagrees-{name}"), |r| change(&mut r.frames[1]));
    let reusing = Reusing::open(&reuse).unwrap();
    let error = reusing.frame(1, &identities(1), time(1)).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert!(error.message.starts_with("frame 1 "), "{error}");
    let place = error.path.as_ref().expect("a path").to_string();
    assert_eq!(place, dir.join(RECORD).display().to_string());
    // The frames it does not disagree on are still read.
    assert!(reusing.frame(0, &identities(0), time(0)).unwrap().is_some());
}

#[test]
fn a_vouched_frame_the_record_does_not_hold_is_invalid() {
    let (_, mut reuse) = written("short", |r| {
        r.frames.pop();
    });
    reuse.frames.insert(5, identities(5));
    let reusing = Reusing::open(&reuse).unwrap();
    for index in [2, 5] {
        let error = reusing
            .frame(index, &identities(index), time(index))
            .unwrap_err();
        assert_eq!(error.kind, Kind::Invalid, "{error}");
    }
}

#[test]
fn a_recording_begins_anywhere_but_in_the_folder_it_reuses() {
    let reusing = opened("begin");
    let other = scratch("reuse-begin-other");
    fs::write(other.join("f0000.cov"), b"stale").unwrap();
    begin_recording(&other, Some(&reusing)).unwrap();
    assert!(!other.join("f0000.cov").exists());
    begin_recording(&other.join("new"), Some(&reusing)).unwrap();
    begin_recording(&other, None).unwrap();
    // The same folder, by another spelling of its path.
    let same = reusing
        .dir()
        .join("..")
        .join(reusing.dir().file_name().unwrap());
    let error = begin_recording(&same, Some(&reusing)).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert!(reusing.dir().join(RECORD).exists(), "its record is kept");
}

/// The wording of every reuse error, pinned.
#[test]
fn every_reuse_error_message() {
    let dir = scratch("reuse-errors");
    let list = dir.join("reuse.json");
    let other = Platform {
        target: "other-os".to_owned(),
        simd: "other".to_owned(),
    };
    fs::write(&list, reuse_json(&other, "m001")).unwrap();
    let opened_error = |name: &str, change: fn(&mut SampleRecord)| {
        Reusing::open(&written(name, change).1)
            .unwrap_err()
            .to_string()
    };
    let frame_error = |name: &str, index: usize, change: fn(&mut SampleRecord)| {
        let (_, mut reuse) = written(name, change);
        reuse.frames.insert(index, identities(index));
        let reusing = Reusing::open(&reuse).unwrap();
        reusing
            .frame(index, &identities(index), time(index))
            .unwrap_err()
            .to_string()
    };
    let reusing = opened("errors-same");
    let errors = [
        (
            "other platform",
            Reuse::read(&list).unwrap_err().to_string(),
        ),
        (
            "unidentified build",
            opened_error("errors-build", |r| r.build.commit = None),
        ),
        (
            "record of another platform",
            opened_error("errors-platform", |r| {
                r.platform = Platform {
                    target: "other-os".to_owned(),
                    simd: "other".to_owned(),
                };
            }),
        ),
        ("not in the record", frame_error("errors-short", 4, |_| {})),
        (
            "other lists",
            frame_error("errors-lists", 1, |r| {
                r.frames[1].ours = Hex128(1);
                r.frames[1].shown = None;
            }),
        ),
        (
            "other time",
            frame_error("errors-time", 1, |r| {
                r.frames[1].time = 0.25;
                r.frames[1].diff.time = 0.25;
            }),
        ),
        (
            "same folder",
            begin_recording(reusing.dir(), Some(&reusing))
                .unwrap_err()
                .to_string(),
        ),
    ];
    let platform = Platform::current();
    let text = errors
        .iter()
        .map(|(what, error)| format!("{what}: {error}"))
        .collect::<Vec<_>>()
        .join("\n")
        .replace(&scratch_of("").display().to_string(), "<scratch>/")
        .replace(&platform.to_string(), "<this platform>");
    insta::assert_snapshot!(text);
}
