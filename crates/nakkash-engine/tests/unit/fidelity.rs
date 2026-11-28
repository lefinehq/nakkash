//! Unit tests for `src/fidelity.rs`: a manifest read, a sample measured
//! against its own frames, floors read and applied, a background given in
//! place of the project's, ledgers compared, and verdicts scored under both
//! rules.

use std::sync::Arc;

use pretty_assertions::assert_eq;

use super::*;
use crate::diff::tests::renders;
use rstest::rstest;

use crate::record::{
    Build, Hex128, Platform, RECORD, Reuse, ReuseSchema, SampleReuse, read_sample,
};

/// A fixture project with one star, which the import does not support.
const STAR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../nakkash-import/tests/fixtures/Star.aep"
);

/// A scratch folder for the test `name`, empty.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nakkash-fidelity-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create a scratch folder");
    dir
}

/// Measure options that write error images when `images` is set, and
/// neither record nor reuse.
fn with_images(images: bool) -> MeasureOptions {
    MeasureOptions {
        images,
        ..MeasureOptions::default()
    }
}

/// A measure with every error at `error` and no text.
fn measure_at(error: f64) -> Measure {
    let window = Window {
        from: 0.0,
        frames: 3,
        content_error: Some(error),
        content_failing: Some(error),
        frames_passing: Some(if error <= DiffOptions::PASSING { 3 } else { 1 }),
        frames_passing_floor: Some(if error <= DiffOptions::PASSING { 3 } else { 1 }),
        floor: None,
        error_floor: None,
        frames_empty: Some(0),
        drawn_error: error,
        drawn_failing: error,
        union_error: error,
        union_failing: error,
    };
    Measure {
        unsupported: BTreeMap::from([("stars".to_owned(), 1)]),
        texts: 0,
        texts_within: 0,
        text_worst: Some(0.0),
        all: window.clone(),
        settled: Some(Window {
            from: 1.0,
            ..window
        }),
        bg: None,
    }
}

#[test]
fn a_manifest_reads_paths_from_its_own_folder() {
    let dir = scratch("manifest");
    let path = dir.join("samples.json");
    fs::write(
        &path,
        r#"{"samples":[{"id":"s1","project":"p/a.aep","fonts":["fonts"],"reference":"/frames","fps":25,"ignore":["bg"],"floor":"refs/s1/floor.json"}]}"#,
    )
    .unwrap();
    let manifest = Manifest::read(&path).unwrap();
    let sample = &manifest.samples[0];
    assert_eq!(sample.project, dir.join("p/a.aep"));
    assert_eq!(sample.fonts, [dir.join("fonts")]);
    assert_eq!(sample.reference, PathBuf::from("/frames"));
    assert_eq!(sample.floor, Some(dir.join("refs/s1/floor.json")));
    assert_eq!((sample.start, sample.settled), (0.0, None));
    assert_eq!(sample.bg, None, "the project's background by default");
    fs::write(
        &path,
        r##"{"samples":[{"id":"s1","project":"a.aep","reference":"r","fps":25,"bg":"#000000"}]}"##,
    )
    .unwrap();
    let manifest = Manifest::read(&path).unwrap();
    assert_eq!(manifest.samples[0].bg.as_deref(), Some("#000000"));
    fs::write(&path, r#"{"samples":[{"id":"s1","name":"x"}]}"#).unwrap();
    let error = Manifest::read(&path).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid);
    assert!(
        error.message.contains("unknown field `name`"),
        "{}",
        error.message
    );
}

#[test]
fn a_sample_measured_against_its_own_frames_has_no_error() {
    let dir = scratch("own");
    let work = dir.join("work");
    let first = Sample {
        id: "star".into(),
        project: STAR.into(),
        comp: None,
        fonts: Vec::new(),
        reference: dir.join("reference"),
        fps: 30.0,
        start: 0.0,
        ignore: Vec::new(),
        settled: Some(1.0),
        excluded: None,
        floor: None,
        bg: None,
        folder: PathBuf::new(),
    };
    // The reference: the imported scene itself, drawn every two seconds,
    // as a sample at half a frame a second compares it.
    let import = import_scene(STAR, &Options::default(), None).unwrap();
    let session: Session = write_document(&import.json).parse().unwrap();
    let fps = import.json["fps"].as_f64().unwrap();
    fs::create_dir_all(&first.reference).unwrap();
    for (i, frame) in [0.0, 2.0 * fps, 4.0 * fps].iter().enumerate() {
        let png = session.render(*frame).unwrap().to_png().unwrap();
        fs::write(first.reference.join(format!("f{i}.png")), png).unwrap();
    }
    let sample = Sample { fps: 0.5, ..first };
    let measure = measure(&sample, &work, &with_images(true)).unwrap();
    assert!(measure.unsupported.is_empty(), "{measure:?}");
    assert_eq!((measure.texts, measure.text_worst), (0, Some(0.0)));
    assert_eq!(measure.all.frames, 3);
    assert_eq!(measure.all.union_failing, 0.0);
    assert_eq!(measure.all.content_failing, Some(0.0));
    assert_eq!(measure.all.frames_passing, Some(3));
    // The star is drawn, in the scene and so in its own frames: every frame
    // passes and shows it, so the sample passes.
    assert_eq!(measure.all.frames_empty, Some(0));
    assert_eq!(measure.verdict(None, Rule::Floor), Verdict::Passes);
    assert_eq!(measure.settled.as_ref().map(|w| w.frames), Some(2));
    assert!(work.join("star/scene.json").exists());
    assert!(work.join("star/errors/f0.png").exists());
    assert_eq!(measure.all.floor, None, "no floor given");
    // With floors, the window records their mean and counts over them.
    let floor = dir.join("floor.json");
    fs::write(
        &floor,
        r#"{"frames":{"f0.png":0.01,"f1.png":0.02,"f2.png":0.03}}"#,
    )
    .unwrap();
    let floored = Sample {
        floor: Some(floor.clone()),
        ..sample.clone()
    };
    let measure = super::measure(&floored, &work, &with_images(false)).unwrap();
    assert_eq!(measure.all.floor.map(|f| (f * 1e6).round()), Some(20_000.0));
    assert_eq!(measure.all.frames_passing_floor, Some(3));
    let settled = measure.settled.and_then(|w| w.floor);
    assert_eq!(settled.map(|f| (f * 1e6).round()), Some(25_000.0));
    // Floors made for other frames are refused, not applied.
    fs::write(&floor, r#"{"frames":{"f0.png":0.01,"f1.png":0.02}}"#).unwrap();
    let stale = super::measure(&floored, &work, &with_images(false)).unwrap_err();
    assert!(
        stale
            .message
            .contains("sample `star`: the floors give no floor for `f2.png`"),
        "{}",
        stale.message
    );
}

#[test]
fn a_sample_is_measured_over_the_background_its_reference_shows() {
    let dir = scratch("bg");
    let work = dir.join("work");
    // The reference: the scene drawn over another background than the
    // project's, as some references were rendered.
    // Imported plain, so the scene writes its own background, not a slot.
    let plain = Options {
        plain: true,
        ..Options::default()
    };
    let mut import = import_scene(STAR, &plain, None).unwrap();
    let own = import.json["bg"].as_str().unwrap().to_owned();
    let other = if own == "#000000" {
        "#FFFFFF"
    } else {
        "#000000"
    };
    import.json["bg"] = serde_json::json!(other);
    let session: Session = write_document(&import.json).parse().unwrap();
    let reference = dir.join("reference");
    fs::create_dir_all(&reference).unwrap();
    let fps = import.json["fps"].as_f64().unwrap();
    for (i, frame) in [0.0, fps].iter().enumerate() {
        let png = session.render(*frame).unwrap().to_png().unwrap();
        fs::write(reference.join(format!("f{i}.png")), png).unwrap();
    }
    let project = Sample {
        id: "star".into(),
        project: STAR.into(),
        comp: None,
        fonts: Vec::new(),
        reference,
        fps: 1.0,
        start: 0.0,
        ignore: Vec::new(),
        settled: None,
        excluded: None,
        floor: None,
        bg: None,
        folder: PathBuf::new(),
    };
    // Over the project's background, everything the star leaves uncovered
    // fails.
    let measure = super::measure(&project, &work, &with_images(false)).unwrap();
    assert_eq!(measure.all.frames_passing, Some(0));
    assert_eq!(measure.bg, None);
    // Over the reference's, nothing does, and the measure says what it was
    // measured over; the import is not changed.
    let given = Sample {
        bg: Some(other.into()),
        ..project.clone()
    };
    let measure = super::measure(&given, &work, &with_images(false)).unwrap();
    assert_eq!(measure.all.content_failing, Some(0.0));
    assert_eq!(measure.all.frames_passing, Some(2));
    assert_eq!(measure.bg.as_deref(), Some(other));
    let again = import_scene(STAR, &plain, None).unwrap();
    assert_eq!(again.json["bg"].as_str(), Some(own.as_str()));
    // The ledger records it, and one written before it reads as measured
    // over the project's.
    let ledger = Ledger {
        samples: BTreeMap::from([("star".to_owned(), measure)]),
    };
    let text = ledger.to_json();
    assert!(text.contains(&format!(r#""bg": "{other}""#)), "{text}");
    let path = dir.join("ledger.json");
    fs::write(&path, &text).unwrap();
    assert_eq!(
        Ledger::read(&path).unwrap().samples["star"].bg.as_deref(),
        Some(other)
    );
    assert!(!Ledger::default().to_json().contains("bg"));
    // A background that is not a colour is refused, not measured.
    let wrong = Sample {
        bg: Some("dark".into()),
        ..project
    };
    let error = super::measure(&wrong, &work, &with_images(false)).unwrap_err();
    assert!(
        error.message.starts_with("sample `star`: "),
        "{}",
        error.message
    );
}

#[test]
fn comparing_ledgers_names_what_got_better_or_worse() {
    let before = Ledger {
        samples: BTreeMap::from([
            ("a".to_owned(), measure_at(0.1)),
            ("gone".to_owned(), measure_at(0.1)),
        ]),
    };
    let mut better = measure_at(0.05);
    better.all.drawn_error = 0.1 + NOISE / 2.0;
    better.texts_within = 1;
    better.all.frames_passing = Some(3);
    better.unsupported.clear();
    let after = Ledger {
        samples: BTreeMap::from([("a".to_owned(), better)]),
    };
    let changes = compare(&before, &after);
    let of = |metric: &str| {
        changes
            .iter()
            .find(|c| c.sample == "a" && c.metric == metric)
            .map(|c| (c.before, c.after, c.better))
    };
    assert_eq!(of("all drawn_error"), None, "within the noise");
    assert_eq!(
        of("settled union_failing"),
        Some((Some(0.1), Some(0.05), Some(true)))
    );
    assert_eq!(
        of("settled content_failing"),
        Some((Some(0.1), Some(0.05), Some(true)))
    );
    assert_eq!(of("texts_within"), Some((Some(0.0), Some(1.0), Some(true))));
    assert_eq!(
        of("all frames_passing"),
        Some((Some(1.0), Some(3.0), Some(true))),
        "more frames passing is better"
    );
    assert_eq!(of("settled frames_passing"), None, "the same count");
    assert_eq!(of("unsupported: stars"), Some((Some(1.0), None, None)));
    assert!(
        changes
            .iter()
            .any(|c| c.sample == "gone" && c.after.is_none())
    );
    let worse = compare(&after, &before);
    assert!(
        worse
            .iter()
            .any(|c| c.metric == "settled content_failing" && c.better == Some(false))
    );
    // A ledger from before the content window: its numbers are new, not worse.
    let mut old = measure_at(0.1);
    old.settled = None;
    old.all.content_error = None;
    old.all.content_failing = None;
    old.all.frames_passing = None;
    old.all.frames_empty = None;
    let older = Ledger {
        samples: BTreeMap::from([("a".to_owned(), old)]),
    };
    let first = compare(&older, &before);
    let content = first
        .iter()
        .find(|c| c.metric == "all content_failing")
        .unwrap();
    assert_eq!((content.before, content.better), (None, None));
}

#[test]
fn a_ledger_round_trips_rounded() {
    let dir = scratch("ledger");
    let path = dir.join("ledger.json");
    assert_eq!(Ledger::read(&path).unwrap(), Ledger::default());
    let mut measure = measure_at(0.123_456_789);
    measure.text_worst = None;
    let ledger = Ledger {
        samples: BTreeMap::from([("a".to_owned(), measure)]),
    };
    fs::write(&path, ledger.to_json()).unwrap();
    let read = Ledger::read(&path).unwrap();
    assert_eq!(read.samples["a"].all.drawn_error, 0.123_457);
    assert_eq!(read.samples["a"].text_worst, None);
    assert!(ledger.to_json().ends_with("}\n"));
    // A ledger written before the content window still reads.
    let old: String = ledger
        .to_json()
        .lines()
        .filter(|line| !line.contains("\"content_") && !line.contains("\"frames_"))
        .collect();
    fs::write(&path, old).unwrap();
    assert_eq!(
        Ledger::read(&path).unwrap().samples["a"]
            .all
            .content_failing,
        None
    );
    assert_eq!(
        Ledger::read(&path).unwrap().samples["a"].verdict(None, Rule::Floor),
        Verdict::Unscored
    );
}

/// A sample with id `id`, for scoring; its files are never read.
fn sample(id: &str, excluded: Option<&str>) -> Sample {
    Sample {
        id: id.into(),
        project: PathBuf::new(),
        comp: None,
        fonts: Vec::new(),
        reference: PathBuf::new(),
        fps: 25.0,
        start: 0.0,
        ignore: Vec::new(),
        settled: Some(1.0),
        excluded: excluded.map(str::to_owned),
        floor: None,
        bg: None,
        folder: PathBuf::new(),
    }
}

#[test]
fn a_sample_passes_only_when_every_frame_does() {
    let mut measure = measure_at(DiffOptions::PASSING);
    assert_eq!(
        measure.verdict(None, Rule::Strict),
        Verdict::Passes,
        "the limit passes"
    );
    measure.all.frames_passing = Some(2);
    assert_eq!(
        measure.verdict(None, Rule::Strict),
        Verdict::Settles,
        "its entrance fails"
    );
    // Frames with nothing in them pass without showing anything.
    let mut hollow = measure.clone();
    hollow.settled.as_mut().unwrap().frames_empty = Some(3);
    assert!(
        matches!(hollow.verdict(None, Rule::Strict), Verdict::Blocked(_)),
        "all empty"
    );
    hollow.settled.as_mut().unwrap().frames_empty = Some(2);
    assert_eq!(
        hollow.verdict(None, Rule::Strict),
        Verdict::Settles,
        "one has content"
    );
    let none = Window {
        frames: 0,
        frames_passing: Some(0),
        frames_empty: Some(0),
        ..measure.all.clone()
    };
    hollow.settled = Some(none);
    assert!(
        matches!(hollow.verdict(None, Rule::Strict), Verdict::Blocked(_)),
        "no frame"
    );
    let failing = measure_at(0.5);
    assert_eq!(
        failing.verdict(None, Rule::Strict),
        Verdict::Blocked(vec!["stars".into()])
    );
    let mut silent = failing.clone();
    silent.unsupported.clear();
    assert_eq!(silent.verdict(None, Rule::Strict), Verdict::Unexplained);
    assert_eq!(
        silent.verdict(Some("other footage"), Rule::Strict),
        Verdict::Excluded("other footage".into())
    );
    let frame = |failing: f64| crate::diff::Error {
        failing,
        ..crate::diff::Error::default()
    };
    assert!(frame(0.01).passes() && !frame(0.010_001).passes());
}

#[test]
fn the_scoreboard_ranks_single_gaps_and_leaves_out_the_excluded() {
    let mut two = measure_at(0.5);
    two.unsupported.insert("masks".into(), 3);
    let mut silent = measure_at(0.5);
    silent.unsupported.clear();
    let ledger = Ledger {
        samples: BTreeMap::from([
            ("pass".to_owned(), measure_at(0.0)),
            ("star1".to_owned(), measure_at(0.5)),
            ("star2".to_owned(), measure_at(0.5)),
            ("two".to_owned(), two),
            ("silent".to_owned(), silent.clone()),
            ("out".to_owned(), silent),
        ]),
    };
    let samples = [
        sample("pass", None),
        sample("star1", None),
        sample("two", None),
        sample("star2", None),
        sample("silent", None),
        sample("out", Some("an empty composition")),
        sample("unmeasured", None),
    ];
    let board = Scoreboard::new(&samples, &ledger, Rule::Floor);
    assert_eq!(board.samples.len(), 6, "the unmeasured one is left out");
    assert_eq!(board.scored(), 5);
    assert_eq!(board.ids(|v| *v == Verdict::Passes), ["pass"]);
    assert_eq!(board.ids(|v| *v == Verdict::Unexplained), ["silent"]);
    assert_eq!(board.single_gaps(), [("stars", vec!["star1", "star2"])]);
    // Frames are counted over the scored samples only, not the excluded.
    assert_eq!(
        (board.frames_passing, board.frames),
        (3 + 1 + 1 + 1 + 1, 15)
    );
}

#[test]
fn floors_read_by_frame_and_must_cover_the_reference() {
    let dir = scratch("floors");
    let path = dir.join("floor.json");
    // What the floors were measured from is the tooling's, and not read.
    fs::write(
        &path,
        r#"{"frames":{"f1.png":0.0128,"f2.png":0},"made":{"x264":"crf=23.0"}}"#,
    )
    .unwrap();
    let floors = Floors::read(&path).unwrap();
    let frames = |names: &[&str]| -> Vec<PathBuf> { names.iter().map(|n| dir.join(n)).collect() };
    // A file without error floors holds the failing share alone.
    let only = |failing: f64| Floor {
        failing,
        error: None,
    };
    assert_eq!(
        floors.of(&frames(&["f1.png", "f2.png"])).unwrap(),
        [only(0.0128), only(0.0)]
    );
    let missing = floors
        .of(&frames(&["f1.png", "f2.png", "f3.png"]))
        .unwrap_err();
    assert!(
        missing.message.contains("no floor for `f3.png`"),
        "{}",
        missing.message
    );
    let extra = floors.of(&frames(&["f1.png"])).unwrap_err();
    assert!(
        extra
            .message
            .contains("`f2.png`, which the reference lacks"),
        "{}",
        extra.message
    );
    fs::write(
        &path,
        r#"{"frames":{"f1.png":0.0128,"f2.png":0},"errors":{"f1.png":0.002,"f2.png":0}}"#,
    )
    .unwrap();
    let both = Floors::read(&path).unwrap();
    let floors = both.of(&frames(&["f1.png", "f2.png"])).unwrap();
    assert_eq!(floors[0].error, Some(0.002));
    fs::write(
        &path,
        r#"{"frames":{"f1.png":0.0128,"f2.png":0},"errors":{"f1.png":0.002}}"#,
    )
    .unwrap();
    let short = Floors::read(&path).unwrap();
    let missing = short.of(&frames(&["f1.png", "f2.png"])).unwrap_err();
    assert!(
        missing.message.contains("no error floor for `f2.png`"),
        "{}",
        missing.message
    );
    for bad in [
        r#"{"frames":{"f1.png":0.1},"errors":{"f1.png":2}}"#,
        r#"{"frames":{"f1.png":1.5}}"#,
        r#"{"frames":{"f1.png":-0.1}}"#,
        r#"{"floors":{}}"#,
    ] {
        fs::write(&path, bad).unwrap();
        assert_eq!(
            Floors::read(&path).unwrap_err().kind,
            Kind::Invalid,
            "{bad}"
        );
    }
}

#[test]
fn a_frame_passes_over_its_floor_plus_the_limit() {
    let frame = |failing: f64| crate::diff::Error {
        failing,
        ..crate::diff::Error::default()
    };
    let floor = |failing: f64| Floor {
        failing,
        error: None,
    };
    assert!(frame(0.0228).passes_over(floor(0.0128)) && !frame(0.0229).passes_over(floor(0.0128)));
    assert!(!frame(0.0228).passes(), "strictly, the same frame fails");
    assert_eq!(frame(0.01).passes_over(Floor::NONE), frame(0.01).passes());
    // With an error floor, the mean error must stay within its margin too.
    let both = Floor {
        failing: 0.5,
        error: Some(0.02),
    };
    let at = |mean: f64| crate::diff::Error {
        failing: 0.4,
        mean,
        ..crate::diff::Error::default()
    };
    let limit = 0.02 + DiffOptions::ERROR_MARGIN;
    assert!(at(limit).passes_over(both), "the margin passes");
    assert!(
        !at(limit + 1e-6).passes_over(both),
        "a failing share within its floor is not enough"
    );
    assert!(
        at(0.2).passes_over(floor(0.5)),
        "without an error floor, as before"
    );
}

/// A frame of the content window failing on `failing`, at `time`.
fn frame_at(time: f64, failing: f64) -> FrameDiff {
    let error = crate::diff::Error {
        pixels: 100,
        failing,
        ..crate::diff::Error::default()
    };
    FrameDiff {
        file: format!("f{time}.png"),
        time,
        content: error,
        drawn: error,
        union: error,
    }
}

#[test]
fn a_window_counts_its_frames_under_both_rules() {
    let frames = [
        frame_at(0.0, 0.0),
        frame_at(1.0, 0.015),
        frame_at(2.0, 0.03),
    ];
    let floors = [0.0, 0.008, 0.01].map(|failing| Floor {
        failing,
        error: Some(0.0),
    });
    let floored = window(&frames, Some(&floors), 0.0);
    assert_eq!(floored.frames_passing, Some(1));
    assert_eq!(
        floored.frames_passing_floor,
        Some(2),
        "0.015 <= 0.008 + 0.01"
    );
    assert_eq!(floored.floor.map(|f| (f * 1e6).round() / 1e6), Some(0.006));
    assert_eq!(floored.error_floor, Some(0.0));
    let plain = window(&frames, None, 0.0);
    assert_eq!(
        (plain.frames_passing_floor, plain.floor, plain.error_floor),
        (Some(1), None, None)
    );
}

#[test]
fn a_sample_may_pass_over_its_floor_and_fail_strictly() {
    let mut measure = measure_at(0.015);
    for w in [&mut measure.all, measure.settled.as_mut().unwrap()] {
        w.frames_passing = Some(1);
        w.frames_passing_floor = Some(3);
        w.floor = Some(0.008);
    }
    assert_eq!(measure.verdict(None, Rule::Floor), Verdict::Passes);
    assert_eq!(
        measure.verdict(None, Rule::Strict),
        Verdict::Blocked(vec!["stars".into()])
    );
    measure.all.frames_passing_floor = Some(2);
    assert_eq!(measure.verdict(None, Rule::Floor), Verdict::Settles);
    let samples = [sample("a", None)];
    let ledger = Ledger {
        samples: BTreeMap::from([("a".to_owned(), measure)]),
    };
    let floor = Scoreboard::new(&samples, &ledger, Rule::Floor);
    let strict = Scoreboard::new(&samples, &ledger, Rule::Strict);
    assert_eq!(
        (floor.rule, floor.frames_passing, floor.frames),
        (Rule::Floor, 2, 3)
    );
    assert_eq!(
        (
            strict.frames_passing,
            strict.ids(|v| *v == Verdict::Settles).len()
        ),
        (1, 0)
    );
}

#[test]
fn a_ledger_from_before_floors_reads_and_compares_as_strict() {
    let dir = scratch("before-floors");
    let path = dir.join("ledger.json");
    let ledger = Ledger {
        samples: BTreeMap::from([("a".to_owned(), measure_at(0.015))]),
    };
    let old: String = ledger
        .to_json()
        .lines()
        .filter(|line| !line.contains("\"frames_passing_floor\"") && !line.contains("\"floor\""))
        .collect();
    fs::write(&path, old).unwrap();
    let older = Ledger::read(&path).unwrap();
    let window = &older.samples["a"].all;
    assert_eq!((window.frames_passing_floor, window.floor), (None, None));
    // Measured with no floor, its floor-rule count is its strict one.
    assert_eq!(window.passing(Rule::Floor), Some(1));
    assert_eq!(
        older.samples["a"].verdict(None, Rule::Floor),
        older.samples["a"].verdict(None, Rule::Strict)
    );
    // Floors coming in show as frames passing over them, and the floor
    // itself, what the reference allows, is not compared.
    let mut floored = measure_at(0.015);
    floored.all.frames_passing_floor = Some(3);
    floored.all.floor = Some(0.008);
    let after = Ledger {
        samples: BTreeMap::from([("a".to_owned(), floored)]),
    };
    let changes = compare(&older, &after);
    let moved: Vec<(&str, Option<bool>)> = changes
        .iter()
        .map(|c| (c.metric.as_str(), c.better))
        .collect();
    assert_eq!(moved, [("all frames_passing_floor", Some(true))]);
    // A text line wholly off before and after has not moved.
    let mut off = after.clone();
    off.samples.get_mut("a").unwrap().text_worst = None;
    assert!(compare(&off, &off).is_empty());
    assert!(
        compare(&after, &older)
            .iter()
            .all(|c| c.better == Some(false))
    );
}

/// The star sample, compared every second over its first two seconds, with
/// its reference drawn from its own scene into `dir`.
fn star_sample(dir: &Path) -> Sample {
    let import = import_scene(STAR, &Options::default(), None).unwrap();
    let session: Session = write_document(&import.json).parse().unwrap();
    let fps = import.json["fps"].as_f64().unwrap();
    let reference = dir.join("reference");
    fs::create_dir_all(&reference).unwrap();
    for (i, frame) in [0.0, fps, 2.0 * fps].iter().enumerate() {
        let png = session.render(*frame).unwrap().to_png().unwrap();
        fs::write(reference.join(format!("f{i}.png")), png).unwrap();
    }
    Sample {
        project: STAR.into(),
        reference,
        fps: 1.0,
        settled: Some(1.0),
        ..sample("star", None)
    }
}

/// Options recording into `dir` with an identified build.
fn recording(dir: &Path) -> MeasureOptions {
    MeasureOptions {
        record: Some(Recording {
            dir: dir.to_path_buf(),
            build: Build {
                commit: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
                tree: None,
                profile: "test".to_owned(),
            },
        }),
        ..MeasureOptions::default()
    }
}

/// Options reusing every frame of the samples recorded under `dir`, as a
/// runner vouching for an unchanged build writes them.
fn reusing_all(dir: &Path, ids: &[&str]) -> MeasureOptions {
    let samples = ids
        .iter()
        .map(|id| {
            let sample = dir.join(id);
            ((*id).to_owned(), SampleReuse::every_frame(sample).unwrap())
        })
        .collect();
    MeasureOptions {
        reuse: Some(Arc::new(Reuse {
            schema: ReuseSchema::V1,
            platform: Platform::current(),
            samples,
        })),
        ..MeasureOptions::default()
    }
}

/// A measure and how many renders it drew.
fn measured(sample: &Sample, work: &Path, options: &MeasureOptions) -> (Measure, usize) {
    let mut measure = None;
    let n = renders(|| measure = Some(super::measure(sample, work, options).unwrap()));
    (measure.unwrap(), n)
}

#[test]
fn a_recorded_sample_reused_draws_nothing_and_writes_the_same_ledger() {
    let dir = scratch("record-reuse");
    let work = dir.join("work");
    let records = dir.join("records");
    let sample = star_sample(&dir);
    let (full, drawn) = measured(&sample, &work, &recording(&records));
    assert_eq!(drawn, 3 * 2, "each frame's coverage and ours");
    let record = read_sample(&records.join("star")).unwrap();
    assert_eq!(record.sample, "star");
    assert_eq!(record.entry, sample.entry().unwrap());
    assert_eq!(record.platform, Platform::current());
    assert_eq!(record.build.commit.as_deref().map(str::len), Some(40));
    assert_eq!(record.frames.len(), 3);
    let (reused, n) = measured(&sample, &work, &reusing_all(&records, &["star"]));
    assert_eq!(n, 0);
    assert_eq!(reused, full);
    let ledger = |measure: Measure| {
        Ledger {
            samples: BTreeMap::from([("star".to_owned(), measure)]),
        }
        .to_json()
    };
    assert_eq!(ledger(reused), ledger(full.clone()));
    // A run without a record or a list measures the same.
    assert_eq!(measured(&sample, &work, &with_images(false)).0, full);
}

/// A changed manifest entry is a changed sample: its record is not reused
/// even where every frame draws the same lists, and it is measured afresh.
#[test]
fn a_sample_whose_manifest_entry_changed_is_drawn_whole() {
    let dir = scratch("entry");
    let work = dir.join("work");
    let records = dir.join("records");
    let sample = star_sample(&dir);
    measured(&sample, &work, &recording(&records));
    let changed = Sample {
        excluded: Some("a reference of other footage".to_owned()),
        ..sample.clone()
    };
    assert_ne!(changed.entry().unwrap(), sample.entry().unwrap());
    let (reused, n) = measured(&changed, &work, &reusing_all(&records, &["star"]));
    assert_eq!(n, 3 * 2);
    assert_eq!(reused, measured(&changed, &work, &with_images(false)).0);
    // Still never recorded over: the folder it would reuse is refused
    // whether or not its record is reused.
    let into_itself = MeasureOptions {
        record: recording(&records).record,
        ..reusing_all(&records, &["star"])
    };
    let error = super::measure(&changed, &work, &into_itself).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert!(records.join("star").join(RECORD).exists());
}

#[test]
fn a_sample_the_reuse_list_does_not_name_is_drawn() {
    let dir = scratch("unnamed");
    let work = dir.join("work");
    let records = dir.join("records");
    let sample = star_sample(&dir);
    let (full, _) = measured(&sample, &work, &recording(&records));
    let mut options = reusing_all(&records, &["star"]);
    let reuse = options.reuse.as_mut().map(Arc::make_mut).unwrap();
    let only = reuse.samples.remove("star").unwrap();
    reuse.samples.insert("other".to_owned(), only);
    let (measure, n) = measured(&sample, &work, &options);
    assert_eq!((measure, n), (full, 3 * 2));
}

#[test]
fn a_sample_whose_record_is_gone_is_an_error() {
    let dir = scratch("gone");
    let work = dir.join("work");
    let records = dir.join("records");
    let sample = star_sample(&dir);
    measured(&sample, &work, &recording(&records));
    let options = reusing_all(&records, &["star"]);
    fs::remove_file(records.join("star").join(RECORD)).unwrap();
    let error = super::measure(&sample, &work, &options).unwrap_err();
    assert_eq!(error.kind, Kind::Io, "{error}");
    assert!(error.message.starts_with("sample `star`: "), "{error}");
}

#[test]
fn a_failing_sample_leaves_no_record() {
    let dir = scratch("failing");
    let work = dir.join("work");
    let records = dir.join("records");
    let record = records.join("star").join(RECORD);
    let sample = star_sample(&dir);
    super::measure(&sample, &work, &recording(&records)).unwrap();
    assert!(record.exists());
    // A reference frame of the wrong size fails the sample as it draws.
    let small = crate::Image::new(2, 2, vec![0; 16]).unwrap();
    fs::write(sample.reference.join("f1.png"), small.to_png().unwrap()).unwrap();
    let error = super::measure(&sample, &work, &recording(&records)).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert!(!record.exists());
    // A sample that fails before it draws leaves none either, not even the
    // record of the run before it.
    let sample = star_sample(&dir);
    super::measure(&sample, &work, &recording(&records)).unwrap();
    assert!(record.exists());
    let missing = Sample {
        project: dir.join("missing.aep"),
        ..sample
    };
    assert!(super::measure(&missing, &work, &recording(&records)).is_err());
    assert!(!record.exists());
}

#[test]
fn a_sample_is_known_by_its_whole_entry() {
    let base = sample("s1", None);
    let entries = [
        base.clone(),
        Sample {
            bg: Some("#000000".to_owned()),
            ..base.clone()
        },
        Sample {
            ignore: vec!["plate".parse().unwrap()],
            ..base.clone()
        },
        Sample {
            settled: None,
            ..base.clone()
        },
        Sample {
            floor: Some(PathBuf::from("floor.json")),
            ..base.clone()
        },
        Sample {
            id: "s2".to_owned(),
            ..base.clone()
        },
    ];
    let hashes: std::collections::BTreeSet<u128> =
        entries.iter().map(|s| s.entry().unwrap().0).collect();
    assert_eq!(hashes.len(), entries.len());
    assert_eq!(base.entry().unwrap(), sample("s1", None).entry().unwrap());
}

/// The entry is hashed only by a run that records or reuses: a plain run
/// of a sample whose paths are not Unicode is measured as it always was,
/// and fails, if at all, as it always did.
#[cfg(unix)]
#[test]
fn an_entry_whose_path_is_not_unicode_is_hashed_only_when_recording() {
    use std::os::unix::ffi::OsStrExt;
    let dir = scratch("not-unicode");
    let work = dir.join("work");
    let sample = Sample {
        project: dir.join(std::ffi::OsStr::from_bytes(b"\xff.aep")),
        ..star_sample(&dir)
    };
    let error = sample.entry().unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert!(error.expected.is_some());
    // A plain run gets past the entry, to the import of a missing project.
    let plain = super::measure(&sample, &work, &MeasureOptions::default()).unwrap_err();
    assert!(!plain.message.contains("manifest entry"), "{plain}");
    // A run that records names its entry, and cannot.
    let records = dir.join("records");
    let recorded = super::measure(&sample, &work, &recording(&records)).unwrap_err();
    assert!(recorded.message.contains("manifest entry"), "{recorded}");
}

/// A sample's id names its work and record folders, so it is one folder
/// name, and a manifest naming another way is refused.
#[rstest]
#[case::empty("")]
#[case::here(".")]
#[case::up("..")]
#[case::nested("a/b")]
#[case::absolute("/m001")]
#[case::trailing("m001/")]
fn a_manifest_refuses_an_id_that_is_not_one_folder_name(#[case] id: &str) {
    let dir = scratch(&format!("id-{}", id.len()));
    let path = dir.join("samples.json");
    let manifest = serde_json::json!({
        "samples": [{"id": id, "project": "a.aep", "reference": "r", "fps": 25}],
    });
    fs::write(&path, manifest.to_string()).unwrap();
    let error = Manifest::read(&path).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert!(error.message.contains("is not one folder name"), "{error}");
    assert!(
        error.message.starts_with(&path.display().to_string()),
        "{error}"
    );
    let sample = Sample {
        id: id.to_owned(),
        ..sample("s1", None)
    };
    assert_eq!(sample.check_id().unwrap_err().kind, Kind::Invalid);
}

/// A sample made in code with such an id is refused before it records, so
/// a recording never begins in another folder than its own.
#[test]
fn a_sample_whose_id_is_not_a_folder_name_does_not_record() {
    let dir = scratch("id-record");
    let records = dir.join("records");
    fs::create_dir_all(&records).unwrap();
    fs::write(records.join("f0000.cov"), b"not ours").unwrap();
    let here = Sample {
        id: ".".to_owned(),
        ..sample("s1", None)
    };
    let error = super::measure(&here, &dir.join("work"), &recording(&records)).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert!(records.join("f0000.cov").exists(), "nothing removed");
    assert!(sample("m001", None).check_id().is_ok());
}

/// A manifest named another way, from here, with `./` or from the root,
/// gives each sample the same entry, so its records stay reusable.
#[test]
fn the_entry_does_not_depend_on_how_the_manifest_is_named() {
    let dir = scratch("spelling");
    let path = dir.join("m.json");
    let manifest = serde_json::json!({
        "samples": [{"id": "s1", "project": "p/a.aep", "fonts": ["fonts"],
            "reference": "refs/s1", "fps": 25, "floor": "floor.json"}],
    });
    fs::write(&path, manifest.to_string()).unwrap();
    // The manifest's path from the current folder, without changing it:
    // up to the root, then down to the manifest.
    let here = std::env::current_dir().unwrap();
    let ups = here.components().count() - 1;
    let relative = std::iter::repeat_n("..", ups)
        .collect::<PathBuf>()
        .join(path.strip_prefix("/").unwrap());
    let spellings = [
        relative.clone(),
        Path::new(".").join(&relative),
        path.clone(),
        dir.join(".").join("m.json"),
    ];
    let entries: Vec<Hex128> = spellings
        .iter()
        .map(|spelling| {
            Manifest::read(spelling).unwrap().samples[0]
                .entry()
                .unwrap()
        })
        .collect();
    assert!(entries.iter().all(|e| *e == entries[0]), "{entries:?}");
    let read = Manifest::read(&path).unwrap();
    assert_eq!(
        read.samples[0].project,
        dir.join("p/a.aep"),
        "paths resolve"
    );
    // A changed path changes it.
    let moved = Sample {
        project: dir.join("p/b.aep"),
        ..read.samples[0].clone()
    };
    assert_ne!(moved.entry().unwrap(), entries[0]);
}
