//! Unit tests for `src/diff/frames.rs`: a diff that records what each frame
//! drew, and a diff that reads recorded frames in place of drawing them and
//! measures the same, bit for bit; a frame whose lists changed is drawn
//! again, and a record that cannot stand in for a draw is an error.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use pretty_assertions::assert_eq;
use rstest::rstest;

use crate::diff::tests::{SCENE, options, references, renders, scratch, session};
use crate::record::{
    Build, FrameRecord, Hex128, Identities, Platform, RECORD, Reusing, SampleRecord, SampleReuse,
    Schema, begin_recording, read_frame, read_sample, write_sample,
};
use crate::{Diff, DiffOptions, Session};
use nakkash_format::Kind;

/// SCENE with its box drawn only in its first five frames, so a change to
/// the box changes only those.
fn spanned() -> String {
    SCENE.replace(r#""id":"box","#, r#""id":"box","span":"0f-5f","#)
}

/// An identified build, as a binary built in its checkout names itself.
fn build() -> Build {
    Build {
        commit: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
        tree: None,
        profile: "test".to_owned(),
    }
}

/// `opts` ignoring the layers `ignore`.
fn ignoring(opts: DiffOptions, ignore: &[&str]) -> DiffOptions {
    DiffOptions {
        ignore: ignore.iter().map(|a| a.parse().unwrap()).collect(),
        ..opts
    }
}

/// Begins a recording into `dir`, diffs `scene` with `opts` recording into
/// it, and writes the record as a fidelity run does once the sample has
/// measured; the diff and the frames' records.
fn record(scene: &Session, opts: &DiffOptions, dir: &Path) -> (Diff, Vec<FrameRecord>) {
    begin_recording(dir, opts.reuse.as_deref().map(Reusing::dir)).unwrap();
    let recorded = scene
        .diff_recorded(&DiffOptions {
            record: Some(dir.to_path_buf()),
            ..opts.clone()
        })
        .unwrap();
    let frames = recorded.frames.expect("a recording has its frames");
    let sample = SampleRecord {
        schema: Schema::V1,
        sample: "m001".to_owned(),
        platform: Platform::current(),
        build: build(),
        entry: Hex128(0),
        frames: frames.clone(),
    };
    write_sample(dir, &sample).unwrap();
    (recorded.diff, frames)
}

/// `opts` reusing the record in `dir` for the frames `reuse` vouches for.
fn reusing(opts: &DiffOptions, reuse: &SampleReuse) -> DiffOptions {
    DiffOptions {
        reuse: Some(Arc::new(Reusing::open(reuse).unwrap())),
        ..opts.clone()
    }
}

/// The runner's list vouching for every frame of the record in `dir`.
fn every_frame(dir: &Path) -> SampleReuse {
    SampleReuse::every_frame(dir.to_path_buf()).unwrap()
}

/// Rewrites the record in `dir` with `change`, and pins `reuse` to the
/// record as it is now, vouching for the frames it vouched for: a runner
/// that judged the changed record.
fn rewrite(dir: &Path, reuse: &mut SampleReuse, change: impl FnOnce(&mut SampleRecord)) {
    let mut sample = read_sample(dir).unwrap();
    change(&mut sample);
    write_sample(dir, &sample).unwrap();
    reuse.record = every_frame(dir).record;
}

/// A diff and how many renders it drew.
fn counted(scene: &Session, opts: &DiffOptions) -> (Diff, usize) {
    let mut diff = None;
    let n = renders(|| diff = Some(scene.diff(opts).unwrap()));
    (diff.unwrap(), n)
}

#[rstest]
#[case::plain(&[])]
#[case::with_an_ignored_layer(&["bar"])]
fn a_recorded_run_reused_draws_nothing_and_measures_the_same(#[case] ignore: &[&str]) {
    let scene = session(SCENE);
    let name = format!("reused{}", ignore.join("-"));
    let frames = references(&format!("{name}-ref"), &scene, &[0.0, 3.0, 5.0, 9.0]);
    let opts = ignoring(options(frames, 10.0 / 3.0), ignore);
    let dir = scratch(&name);
    let (recorded, records) = record(&scene, &opts, &dir);
    // What our frame draws: the scene with the ignored layers hidden.
    let measured = match ignore.is_empty() {
        true => session(SCENE),
        false => scene.hiding(&opts.ignore).unwrap(),
    };
    assert_eq!(
        recorded,
        scene.diff(&opts).unwrap(),
        "recording changes nothing"
    );
    let (reused, n) = counted(&scene, &reusing(&opts, &every_frame(&dir)));
    assert_eq!(n, 0);
    assert_eq!(reused, recorded);
    // Each frame's record names what it drew, and its errors unrounded.
    assert_eq!(records.len(), 4);
    for (i, frame) in records.iter().enumerate() {
        assert_eq!((frame.index, &frame.diff), (i, &recorded.frames[i]));
        assert_eq!(frame.file, recorded.frames[i].file);
        assert_eq!(frame.time.to_bits(), recorded.frames[i].time.to_bits());
        assert_eq!(frame.shown.is_some(), !ignore.is_empty());
        assert_eq!(frame.shown_mask.is_some(), !ignore.is_empty());
        assert_ne!(frame.ours, frame.bare, "the bare list has no background");
        assert!(frame.ms.draw > 0.0 && frame.ms.coverage > 0.0);
        assert_eq!(frame.ms.shown > 0.0, !ignore.is_empty());
        let drawn = read_frame(&dir, frame, 40, 20).unwrap();
        assert_eq!(drawn.ours, measured.render(frame.time * 10.0).unwrap());
    }
}

#[test]
fn a_frame_whose_list_changed_is_drawn_again() {
    let scene = session(&spanned());
    let frames = references("changed-ref", &scene, &[0.0, 3.0, 7.0, 9.0]);
    let opts = options(frames, 10.0 / 3.0);
    let dir = scratch("changed");
    record(&scene, &opts, &dir);
    let edited = session(&spanned().replace("#FFFFFF", "#FF0000"));
    let (diff, n) = counted(&edited, &reusing(&opts, &every_frame(&dir)));
    // The box is drawn in frames 0 and 3 only: each drawn again, its
    // coverage and ours.
    assert_eq!(n, 2 * 2);
    assert_eq!(diff, edited.diff(&opts).unwrap());
    assert!(diff.frames[0].drawn.failing > 0.0, "the edit is measured");
}

/// A frame the runner vouched for, under lists other than the ones it
/// draws now, is drawn: nobody vouched for what it draws.
#[test]
fn a_frame_the_runner_vouched_for_under_other_lists_is_drawn() {
    let scene = session(SCENE);
    let frames = references("vouched-ref", &scene, &[0.0, 5.0, 9.0]);
    let opts = options(frames, 10.0 / 4.5);
    let dir = scratch("vouched");
    let (recorded, _) = record(&scene, &opts, &dir);
    let mut reuse = every_frame(&dir);
    let other = Identities {
        ours: Hex128(1),
        ..reuse.frames[&2]
    };
    reuse.frames.insert(2, other);
    let (diff, n) = counted(&scene, &reusing(&opts, &reuse));
    assert_eq!(n, 2);
    assert_eq!(diff, recorded);
}

#[test]
fn a_reuse_run_measures_as_many_frames_as_a_full_run() {
    let scene = session(SCENE);
    let frames = references("count-ref", &scene, &[0.0, 2.0, 4.0, 6.0, 8.0]);
    let opts = ignoring(options(frames, 5.0), &["bar"]);
    let dir = scratch("count");
    let (recorded, _) = record(&scene, &opts, &dir);
    // The runner vouches for two frames: the other three are drawn, each
    // its coverage, its shown mask and ours.
    let mut reuse = every_frame(&dir);
    reuse.frames.retain(|i, _| [1, 3].contains(i));
    let (diff, n) = counted(&scene, &reusing(&opts, &reuse));
    assert_eq!(n, 3 * 3);
    assert_eq!(diff.frames.len(), recorded.frames.len());
    assert_eq!(diff, recorded);
    // With none vouched for, every frame is drawn.
    reuse.frames.clear();
    let (diff, n) = counted(&scene, &reusing(&opts, &reuse));
    assert_eq!((n, diff), (5 * 3, recorded));
}

#[rstest]
#[case::ours("ours.lz4")]
#[case::coverage("cov")]
fn a_reused_frame_with_missing_pixels_is_an_error(#[case] ending: &str) {
    let scene = session(SCENE);
    let frames = references(&format!("missing-{ending}-ref"), &scene, &[0.0, 5.0, 9.0]);
    let opts = options(frames, 10.0 / 4.5);
    let dir = scratch(&format!("missing-{ending}"));
    record(&scene, &opts, &dir);
    let path = dir.join(format!("f0001.{ending}"));
    fs::remove_file(&path).unwrap();
    let reuse = reusing(&opts, &every_frame(&dir));
    let mut error = None;
    let n = renders(|| error = Some(scene.diff(&reuse).unwrap_err()));
    let error = error.unwrap();
    assert_eq!(error.kind, Kind::Io, "{error}");
    assert!(
        error.message.contains(&path.display().to_string()),
        "{error}"
    );
    assert_eq!(n, 0, "never drawn in its place");
}

#[test]
fn a_reused_frame_whose_pixels_are_another_runs_is_an_error() {
    let scene = session(SCENE);
    let frames = references("swapped-ref", &scene, &[0.0, 5.0]);
    let opts = options(frames, 2.0);
    let dir = scratch("swapped");
    record(&scene, &opts, &dir);
    fs::copy(dir.join("f0000.ours.lz4"), dir.join("f0001.ours.lz4")).unwrap();
    let error = scene.diff(&reusing(&opts, &every_frame(&dir))).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    let place = error.path.as_ref().expect("a path").to_string();
    assert!(place.ends_with("f0001.ours.lz4"), "{place}");
}

/// What a manifest entry changes in the scene or the options changes the
/// lists of every frame: each is drawn whole, and measured as afresh.
#[rstest]
#[case::bg(r##""bg":"#202020""##, &[], 2)]
#[case::ignore(r##""bg":"#000000""##, &["bar"], 3)]
fn a_sample_whose_manifest_entry_changed_is_drawn_whole(
    #[case] bg: &str,
    #[case] ignore: &[&str],
    #[case] per_frame: usize,
) {
    let scene = session(SCENE);
    let name = format!("entry-{}", ignore.len());
    let frames = references(&format!("{name}-ref"), &scene, &[0.0, 5.0, 9.0]);
    let opts = options(frames, 10.0 / 4.5);
    let dir = scratch(&name);
    record(&scene, &opts, &dir);
    let changed = session(&SCENE.replace(r##""bg":"#000000""##, bg));
    let changed_opts = ignoring(opts, ignore);
    let (diff, n) = counted(&changed, &reusing(&changed_opts, &every_frame(&dir)));
    assert_eq!(n, 3 * per_frame);
    assert_eq!(diff, changed.diff(&changed_opts).unwrap());
}

#[test]
fn a_reused_frame_keeps_its_time() {
    let scene = session(SCENE);
    let frames = references("time-ref", &scene, &[0.0, 5.0, 9.0]);
    let opts = options(frames, 10.0 / 4.5);
    let dir = scratch("time");
    record(&scene, &opts, &dir);
    let mut reuse = every_frame(&dir);
    rewrite(&dir, &mut reuse, |sample| {
        let frame = &mut sample.frames[1];
        frame.time += 0.05;
        frame.diff.time = frame.time;
    });
    let error = scene.diff(&reusing(&opts, &reuse)).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert!(error.message.starts_with("frame 1 "), "{error}");
}

/// The runner vouched for what the frame draws now, and the record holds
/// other lists: an error, never a silent draw nor a wrong frame.
#[test]
fn a_vouched_frame_whose_record_disagrees_is_an_error() {
    let scene = session(SCENE);
    let frames = references("disagrees-ref", &scene, &[0.0, 5.0, 9.0]);
    let opts = options(frames, 10.0 / 4.5);
    let dir = scratch("disagrees");
    record(&scene, &opts, &dir);
    let mut reuse = every_frame(&dir);
    rewrite(&dir, &mut reuse, |sample| sample.frames[2].bare = Hex128(1));
    let error = scene.diff(&reusing(&opts, &reuse)).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert!(error.message.starts_with("frame 2 "), "{error}");
}

/// A run may record while it reuses: the frames it reads are recorded as
/// the ones it draws, and the new record stands in for draws in turn.
#[test]
fn a_recording_that_reuses_records_what_it_read() {
    let scene = session(SCENE);
    let frames = references("again-ref", &scene, &[0.0, 5.0, 9.0]);
    let opts = ignoring(options(frames, 10.0 / 4.5), &["bar"]);
    let first = scratch("again-first");
    let (recorded, records) = record(&scene, &opts, &first);
    let second = scratch("again-second");
    let mut again = None;
    let n = renders(|| {
        again = Some(record(
            &scene,
            &reusing(&opts, &every_frame(&first)),
            &second,
        ))
    });
    let (diff, copies) = again.unwrap();
    assert_eq!(n, 0);
    assert_eq!((diff, copies), (recorded.clone(), records));
    let (diff, n) = counted(&scene, &reusing(&opts, &every_frame(&second)));
    assert_eq!((diff, n), (recorded, 0));
}

#[test]
fn a_run_never_records_into_the_folder_it_reuses() {
    let scene = session(SCENE);
    let frames = references("same-ref", &scene, &[0.0, 5.0]);
    let opts = options(frames, 2.0);
    let dir = scratch("same");
    record(&scene, &opts, &dir);
    let into_itself = DiffOptions {
        record: Some(dir.clone()),
        ..reusing(&opts, &every_frame(&dir))
    };
    let error = scene.diff_recorded(&into_itself).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert!(dir.join(RECORD).exists(), "the record is kept");
    assert!(read_sample(&dir).is_ok());
}

/// Frames are never written beside the record of a recording before:
/// the caller begins the recording first.
#[test]
fn a_recording_that_has_not_begun_is_refused() {
    let scene = session(SCENE);
    let frames = references("unbegun-ref", &scene, &[0.0, 5.0]);
    let opts = options(frames, 2.0);
    let dir = scratch("unbegun");
    record(&scene, &opts, &dir);
    let again = DiffOptions {
        record: Some(dir.clone()),
        ..opts
    };
    let mut error = None;
    let n = renders(|| error = Some(scene.diff_recorded(&again).unwrap_err()));
    let error = error.unwrap();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert_eq!(n, 0, "refused before any draw");
    assert!(read_sample(&dir).is_ok(), "the record is kept");
}

/// A recording begins by clearing its folder: a frame file of a longer
/// recording before it does not outlive it.
#[test]
fn a_recording_removes_the_frames_of_the_one_before() {
    let scene = session(SCENE);
    let frames = references("shorter-ref", &scene, &[0.0, 3.0, 6.0, 9.0]);
    let dir = scratch("shorter");
    record(&scene, &options(frames.clone(), 10.0 / 3.0), &dir);
    assert!(dir.join("f0003.ours.lz4").exists());
    let (_, records) = record(&scene, &options(frames[..2].to_vec(), 10.0 / 3.0), &dir);
    assert_eq!(records.len(), 2);
    assert!(dir.join("f0001.ours.lz4").exists());
    assert!(!dir.join("f0002.ours.lz4").exists());
    assert!(!dir.join("f0003.cov").exists());
}

#[test]
fn a_diff_that_does_not_record_has_no_frame_records() {
    let scene = session(SCENE);
    let frames = references("unrecorded-ref", &scene, &[0.0]);
    let recorded = scene.diff_recorded(&options(frames, 10.0)).unwrap();
    assert_eq!(recorded.frames, None);
}

/// Each frame's input goes to its own call, the results come back in frame
/// order, and the first frame's error wins, on as many threads as frames
/// are drawn on.
#[test]
fn each_frame_takes_its_own_input_and_answers_in_order() {
    let scene = session(SCENE);
    let inputs: Vec<String> = (0..10).map(|i| format!("in{i}")).collect();
    let each = |i: usize, input: String| Ok(format!("{i}:{input}"));
    let expected: Vec<String> = (0..10).map(|i| format!("{i}:in{i}")).collect();
    assert_eq!(scene.each_frame(inputs.clone(), &each).unwrap(), expected);
    let failing = |i: usize, _: String| match i {
        3 | 7 => Err(nakkash_format::Diagnostic::new(
            Kind::Invalid,
            format!("frame {i}"),
        )),
        _ => Ok(i),
    };
    let error = scene.each_frame(inputs, &failing).unwrap_err();
    assert_eq!(error.message, "frame 3");
    assert_eq!(
        scene.each_frame(Vec::new(), &each).unwrap(),
        Vec::<String>::new()
    );
}
