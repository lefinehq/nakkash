//! A test's own scratch folder, for scenes written from the scenes it
//! reads: never beside them, since other tests read those folders (the
//! format's corpus test walks `nakkash-format/tests/compat/v1` and would
//! read a half-written file).
//!
//! A scene written here reads as it does in its own folder: the files and
//! folders it names, relative to that folder, are copied to the same place
//! relative to the scratch folder that stands for it ([`Scratch::beside`]).
//! Every real folder has one stand-in, its path under the scratch folder,
//! so a name that climbs out of the scene's folder (`../fonts/x.ttf`)
//! still finds its copy.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::Value as Json;

/// A test's scratch folder, removed when dropped.
pub struct Scratch(PathBuf);

impl Scratch {
    /// The scratch folder of the test `test`, empty: under cargo's folder
    /// for integration tests' files, named by the test and, apart from
    /// another run of it, by this process.
    pub fn new(test: &str) -> Scratch {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(test)
            .join(std::process::id().to_string());
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("make the scratch folder");
        Scratch(dir)
    }

    /// The folder standing for the folder of `scene`, the scene file `json`
    /// was read from, holding a copy of every file and folder `json` names
    /// relative to it, so a scene written there reads as `scene` does.
    /// What a named scene file names is copied too (an included
    /// composition's pictures, an instance's template and its fonts),
    /// relative to the first scene's folder or its own.
    pub fn beside(&self, scene: &Path, json: &Json) -> PathBuf {
        let folder = real(scene.parent().expect("a scene's folder"));
        let mut todo: Vec<PathBuf> = named(json, &[&folder]);
        let mut copied = BTreeSet::new();
        while let Some(path) = todo.pop() {
            if !copied.insert(path.clone()) {
                continue;
            }
            copy(&path, &self.standing_for(&path));
            if let Some(json) = scene_json(&path) {
                let own = path.parent().expect("a file's folder").to_owned();
                todo.extend(named(&json, &[&folder, &own]));
            }
        }
        let beside = self.standing_for(&folder);
        fs::create_dir_all(&beside).expect("make the scene's stand-in folder");
        beside
    }

    /// Where the real path `path` stands under the scratch folder: its
    /// parts below the root, so one real path has one stand-in.
    fn standing_for(&self, path: &Path) -> PathBuf {
        let parts = path.components().filter_map(|part| match part {
            Component::Normal(name) => Some(name),
            _ => None,
        });
        self.0.join(parts.collect::<PathBuf>())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `path` with every link and `..` resolved.
fn real(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every existing file or folder a string of `json` names relative to one
/// of `folders`, resolved; never one of the folders or a folder holding
/// them, which a text such as `.` or `..` would name.
fn named(json: &Json, folders: &[&Path]) -> Vec<PathBuf> {
    let mut texts = Vec::new();
    strings(json, &mut texts);
    let mut found = Vec::new();
    for text in texts.into_iter().filter(|text| !text.is_empty()) {
        for folder in folders {
            let Ok(path) = fs::canonicalize(folder.join(text)) else {
                continue;
            };
            if !folders.iter().any(|f| f.starts_with(&path)) {
                found.push(path);
            }
        }
    }
    found
}

/// Every string value in `json`, keys left out.
fn strings<'a>(json: &'a Json, into: &mut Vec<&'a str>) {
    match json {
        Json::String(text) => into.push(text),
        Json::Array(items) => items.iter().for_each(|item| strings(item, into)),
        Json::Object(map) => map.values().for_each(|value| strings(value, into)),
        Json::Null | Json::Bool(_) | Json::Number(_) => {}
    }
}

/// The JSON of the file at `path` when it is a JSON file: a scene, an
/// included composition or a template, whose strings may name files too.
fn scene_json(path: &Path) -> Option<Json> {
    let json = path.extension().is_some_and(|e| e == "json");
    let text = fs::read_to_string(path).ok().filter(|_| json)?;
    serde_json::from_str(&text).ok()
}

/// Copies the file or folder `from` to `to`, folders with all they hold,
/// making `to`'s folder first; a copy already there is kept.
fn copy(from: &Path, to: &Path) {
    if to.exists() {
        return;
    }
    fs::create_dir_all(to.parent().expect("a folder")).expect("make the copy's folder");
    if from.is_file() {
        fs::copy(from, to).unwrap_or_else(|e| panic!("copy {}: {e}", from.display()));
        return;
    }
    fs::create_dir_all(to).expect("make the folder's copy");
    for entry in fs::read_dir(from).expect("read a named folder") {
        let entry = entry.expect("a folder entry");
        copy(&entry.path(), &to.join(entry.file_name()));
    }
}
