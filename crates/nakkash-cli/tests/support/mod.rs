//! What the fidelity and `frames` CLI tests share: the real binary, run
//! without a log filter from the environment, scratch folders named by the
//! test, and a manifest of two samples of the star fixture whose
//! references are drawn from their own scene.

#![allow(dead_code, reason = "each test file uses some of it")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use nakkash_engine::{ImportOptions, Session, import_scene, write_document};

/// A fixture project with one star layer, `shape-layer-1`.
pub const STAR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../nakkash-import/tests/fixtures/Star.aep"
);

/// Runs `nakkash` with `args`, without a `NAKKASH_LOG` from the environment.
pub fn nakkash(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nakkash"))
        .env_remove("NAKKASH_LOG")
        .args(args)
        .output()
        .expect("run nakkash")
}

/// What `out` printed on stdout.
pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// What `out` printed on stderr.
pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A fresh scratch folder for the test `name`: tests run in parallel, each
/// in a folder of its own name.
pub fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create a scratch folder");
    dir
}

/// `path` as the string an argument takes.
pub fn arg(path: &Path) -> &str {
    path.to_str().expect("a Unicode path")
}

/// Writes into `dir` a manifest of two samples of the star, compared every
/// second over its first two seconds with its own frames: `star` as it is,
/// and `hidden` with its one layer ignored, so its frames have a shown
/// mask. Returns the manifest's path.
pub fn star_manifest(dir: &Path) -> PathBuf {
    let import = import_scene(STAR, &ImportOptions::default(), None).expect("import the star");
    let session: Session = write_document(&import.json).parse().expect("open the star");
    let fps = import.json["fps"].as_f64().expect("a frame rate");
    let reference = dir.join("reference");
    fs::create_dir_all(&reference).expect("create the reference folder");
    for (i, frame) in [0.0, fps, 2.0 * fps].iter().enumerate() {
        let png = session.render(*frame).expect("draw").to_png().expect("png");
        fs::write(reference.join(format!("f{i}.png")), png).expect("write a reference frame");
    }
    let manifest = serde_json::json!({"samples": [
        {"id": "star", "project": STAR, "reference": "reference", "fps": 1.0},
        {"id": "hidden", "project": STAR, "reference": "reference", "fps": 1.0,
         "ignore": ["shape-layer-1"]},
    ]});
    let path = dir.join("samples.json");
    fs::write(&path, manifest.to_string()).expect("write the manifest");
    path
}
