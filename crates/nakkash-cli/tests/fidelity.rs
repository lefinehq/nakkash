//! End-to-end tests of `nakkash fidelity --record` and `--reuse`: a run
//! that records, then a run that reuses every frame of that record, write
//! the same ledger byte for byte, and the second draws no frame; a record
//! is capped, and the cap is checked against free space before anything
//! is written.

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use nakkash_engine::record::{Platform, RECORD, Reuse, ReuseSchema, SampleReuse, read_sample};
use pretty_assertions::assert_eq;
use rstest::rstest;

use support::{arg, nakkash, scratch, star_manifest, stderr, stdout};

/// The ids of the star manifest's samples.
const SAMPLES: [&str; 2] = ["star", "hidden"];

/// The line `--stats` prints of how many frames a fidelity run drew and
/// how many it reused from a record.
fn frames_line(err: &str) -> &str {
    err.lines()
        .find(|line| line.starts_with("stats: fidelity:"))
        .unwrap_or_else(|| panic!("no frames line in {err}"))
}

/// Writes into `dir` a reuse list vouching for every frame of each sample
/// recorded under `records`, as a runner writes it for an unchanged build.
/// A binary built without git (a checkout synced without `.git`) cannot
/// name its commit, so its records are never reused; such a record is
/// marked with a commit here, so the reuse path is still run.
fn reuse_list(dir: &Path, records: &Path) -> std::path::PathBuf {
    let samples: BTreeMap<String, SampleReuse> = SAMPLES
        .iter()
        .map(|id| {
            let folder = records.join(id);
            if !read_sample(&folder).unwrap().build.identified() {
                let path = folder.join(RECORD);
                let mut record: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                record["build"]["commit"] = serde_json::json!("0".repeat(40));
                fs::write(&path, serde_json::to_string_pretty(&record).unwrap()).unwrap();
            }
            ((*id).to_owned(), SampleReuse::every_frame(folder).unwrap())
        })
        .collect();
    let reuse = Reuse {
        schema: ReuseSchema::V1,
        platform: Platform::current(),
        samples,
    };
    let path = dir.join("reuse.json");
    fs::write(&path, serde_json::to_string_pretty(&reuse).unwrap()).unwrap();
    path
}

#[test]
fn fidelity_records_then_reuses_with_the_same_ledger() {
    let dir = scratch("fidelity-record-reuse");
    let manifest = star_manifest(&dir);
    let records = dir.join("records");
    let ledger = |name: &str| dir.join(name);
    let run = |name: &str, extra: &[&str]| {
        let mut args = vec![
            "--stats",
            "fidelity",
            arg(&manifest),
            "--work",
            arg(&dir),
            "--ledger",
        ];
        let path = ledger(name);
        args.push(arg(&path));
        args.push("--write");
        args.extend_from_slice(extra);
        let out = nakkash(&args);
        assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
        (fs::read(path).unwrap(), stderr(&out))
    };
    let (full, err) = run(
        "full.json",
        &["--record", arg(&records), "--record-cap", "1"],
    );
    assert_eq!(
        frames_line(&err),
        "stats: fidelity: frames drawn 6, reused from a record 0"
    );
    for id in SAMPLES {
        let folder = records.join(id);
        for file in [
            RECORD,
            "scene.json",
            "sample.json",
            "f0002.ours.lz4",
            "f0002.cov",
        ] {
            assert!(folder.join(file).is_file(), "{id}: {file}");
        }
        assert_eq!(read_sample(&folder).unwrap().frames.len(), 3);
    }
    assert!(records.join("hidden/f0000.shown").is_file());
    assert!(!records.join("star/f0000.shown").exists());
    // A record a build that cannot name its commit wrote is never reused,
    // and the run says so once.
    let identified = read_sample(&records.join("star"))
        .unwrap()
        .build
        .identified();
    let warned = err
        .lines()
        .filter(|l| l.contains("cannot be reused"))
        .count();
    assert_eq!(warned, usize::from(!identified), "{err}");
    let list = reuse_list(&dir, &records);
    let (reused, err) = run("reused.json", &["--reuse", arg(&list)]);
    assert_eq!(
        frames_line(&err),
        "stats: fidelity: frames drawn 0, reused from a record 6"
    );
    assert_eq!(
        String::from_utf8(reused).unwrap(),
        String::from_utf8(full.clone()).unwrap()
    );
    // A run without either flag writes the same ledger, drawing every frame.
    let (plain, err) = run("plain.json", &[]);
    assert_eq!(
        frames_line(&err),
        "stats: fidelity: frames drawn 6, reused from a record 0"
    );
    assert_eq!(plain, full);
}

/// The cap is checked against the free space of the folder's file system
/// before anything is written.
#[test]
fn fidelity_refuses_a_cap_that_does_not_fit() {
    let dir = scratch("fidelity-cap-too-big");
    let manifest = star_manifest(&dir);
    let records = dir.join("records");
    let out = nakkash(&[
        "fidelity",
        arg(&manifest),
        "--work",
        arg(&dir),
        "--record",
        arg(&records),
        "--record-cap",
        "1000000000",
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.starts_with("error: ") && err.contains("free"), "{err}");
    assert_eq!(stdout(&out), "");
    assert!(!records.exists(), "nothing written");
}

/// A run whose records reach the cap stops writing them: every sample left
/// is refused, and none leaves a record.
#[test]
fn fidelity_stops_recording_at_its_cap() {
    let dir = scratch("fidelity-cap-reached");
    let manifest = star_manifest(&dir);
    let records = dir.join("records");
    let out = nakkash(&[
        "fidelity",
        arg(&manifest),
        "--work",
        arg(&dir),
        "--record",
        arg(&records),
        "--record-cap",
        "0.000001",
    ]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let text = stdout(&out);
    for id in SAMPLES {
        assert!(
            text.contains(&format!("{id}: not measured: sample `{id}`: ")),
            "{text}"
        );
        assert!(!records.join(id).join(RECORD).exists(), "{id}");
    }
    assert!(text.contains("cap"), "{text}");
}

/// Recording is opt-in and always capped, and draws on the CPU at precise
/// quality alone: what else is asked is a usage error.
#[rstest]
#[case::record_without_a_cap(&["--record", "r"])]
#[case::cap_without_a_record(&["--record-cap", "1"])]
#[case::a_cap_of_nothing(&["--record", "r", "--record-cap", "0"])]
#[case::a_cap_that_is_not_a_size(&["--record", "r", "--record-cap", "big"])]
#[case::on_the_gpu(&["--record", "r", "--record-cap", "1", "--gpu"])]
#[case::reuse_at_fast_quality(&["--reuse", "reuse.json", "--quality", "fast"])]
fn fidelity_refuses_what_it_cannot_record(#[case] extra: &[&str]) {
    let mut args = vec!["fidelity", "samples.json"];
    args.extend_from_slice(extra);
    let out = nakkash(&args);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).starts_with("error: "), "{}", stderr(&out));
}
