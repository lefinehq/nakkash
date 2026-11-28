//! Unit tests for `src/record/replay.rs`: a sample's `sample.json` and
//! `scene.json` written whole within the quota, read back as written,
//! times bit for bit, and an index past the sample refused.

use pretty_assertions::assert_eq;

use super::*;
use crate::record::tests::{as_json, room, scratch};
use crate::record::{
    Build, Hex128, Platform, RECORD, SampleRecord, Schema, read_sample, write_sample,
};
use nakkash_format::Kind;

/// A replay of three frames at awkward times, one layer ignored.
pub(crate) fn replay() -> Replay {
    Replay {
        times: vec![0.0, 1.0 / 3.0, 0.1 + 0.2],
        ignore: vec!["card/photo".parse().unwrap()],
        width: 40,
        height: 20,
    }
}

#[test]
fn a_replay_round_trips_bit_for_bit() {
    let dir = scratch("replay-round");
    let quota = room();
    replay().write(&dir, &quota).unwrap();
    let read = Replay::read(&dir).unwrap();
    assert_eq!(read, replay());
    let bits = |r: &Replay| r.times.iter().map(|t| t.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&read), bits(&replay()));
    let written = std::fs::metadata(dir.join(REPLAY)).unwrap().len();
    assert_eq!(quota.taken(), written, "the file is counted");
    write_scene(&dir, "{\"nakkash\":1}\n", &quota).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join(SCENE)).unwrap(),
        "{\"nakkash\":1}\n"
    );
    assert_eq!(quota.taken(), written + 14);
}

#[test]
fn a_frame_past_the_sample_is_out_of_range() {
    assert_eq!(
        replay().time(1).unwrap().to_bits(),
        (1.0f64 / 3.0).to_bits()
    );
    let error = replay().time(3).unwrap_err();
    assert_eq!(error.kind, Kind::OutOfRange, "{error}");
    assert_eq!(
        error.to_string(),
        "frame 3 is past the sample's 3 frames; expected a frame index from 0 to 2"
    );
}

#[test]
fn a_replay_of_another_form_is_refused_with_its_path() {
    let dir = scratch("replay-form");
    let path = dir.join(REPLAY);
    std::fs::write(
        &path,
        r#"{"times":[0],"ignore":[],"width":4,"height":2,"fps":25}"#,
    )
    .unwrap();
    let error = Replay::read(&dir).unwrap_err();
    assert_eq!(error.kind, Kind::Invalid, "{error}");
    assert!(
        error.to_string().starts_with(&path.display().to_string()),
        "{error}"
    );
    assert!(error.message.contains("unknown field `fps`"), "{error}");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(Replay::read(&dir).unwrap_err().kind, Kind::Io);
}

/// A write past the quota leaves no file.
#[test]
fn a_replay_past_the_quota_is_not_written() {
    let dir = scratch("replay-quota");
    let quota = Quota::within(&dir, 10).unwrap();
    assert_eq!(replay().write(&dir, &quota).unwrap_err().kind, Kind::Io);
    assert!(!dir.join(REPLAY).exists());
}

/// The files a sample's record keeps for `nakkash frames` belong to the
/// recording: beginning another removes them with the record.
#[test]
fn a_recording_begins_without_the_replay_of_the_one_before() {
    let dir = scratch("replay-begin");
    let quota = room();
    replay().write(&dir, &quota).unwrap();
    write_scene(&dir, "{}", &quota).unwrap();
    std::fs::write(dir.join("notes.txt"), "kept").unwrap();
    std::fs::write(dir.join("sample.json.tmp"), "half").unwrap();
    crate::record::begin_sample(&dir).unwrap();
    for gone in [REPLAY, SCENE, "sample.json.tmp"] {
        assert!(!dir.join(gone).exists(), "{gone}");
    }
    assert!(dir.join("notes.txt").exists());
}

/// Writes into `dir` a replay, a scene and a record naming both, as a
/// recording does.
fn recorded(dir: &std::path::Path) {
    let quota = room();
    let scene = write_scene(dir, "{\"nakkash\":1}\n", &quota).unwrap();
    let replay = replay().write(dir, &quota).unwrap();
    let record = SampleRecord {
        schema: Schema::V1,
        sample: "m001".to_owned(),
        platform: Platform::current(),
        build: Build {
            commit: None,
            tree: None,
            profile: "test".to_owned(),
        },
        entry: Hex128(0),
        scene,
        replay,
        frames: Vec::new(),
    };
    write_sample(dir, &record, &quota).unwrap();
}

#[test]
fn a_recorded_sample_opens_with_the_files_its_record_names() {
    let dir = scratch("replay-open");
    recorded(&dir);
    let opened = open_replay(&dir).unwrap();
    assert_eq!(opened.replay, replay());
    assert_eq!(opened.scene, "{\"nakkash\":1}\n");
    assert_eq!(
        as_json(&opened.record),
        as_json(&read_sample(&dir).unwrap())
    );
}

/// A scene or replay another run wrote over the recorded one, or one
/// beside no record, is refused, never drawn.
#[test]
fn a_scene_or_replay_its_record_does_not_name_is_refused() {
    let dir = scratch("replay-other");
    let error = |dir: &std::path::Path| open_replay(dir).unwrap_err();
    recorded(&dir);
    std::fs::write(dir.join(SCENE), "{\"nakkash\":1,\"bg\":\"#000\"}\n").unwrap();
    let scene = error(&dir);
    recorded(&dir);
    let mut other = replay();
    other.times[1] = 0.5;
    other.write(&dir, &room()).unwrap();
    let replayed = error(&dir);
    for refused in [&scene, &replayed] {
        assert_eq!(refused.kind, Kind::Invalid, "{refused}");
        assert!(refused.message.contains("its record names"), "{refused}");
    }
    assert!(
        scene
            .to_string()
            .starts_with(&dir.join(SCENE).display().to_string())
    );
    assert!(
        replayed
            .to_string()
            .starts_with(&dir.join(REPLAY).display().to_string())
    );
    // Without its record, a folder is not a recorded sample.
    recorded(&dir);
    std::fs::remove_file(dir.join(RECORD)).unwrap();
    assert_eq!(error(&dir).kind, Kind::Io);
}
