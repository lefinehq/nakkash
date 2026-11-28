//! Unit tests for `build/checkout.rs`: a build identified by its commit and
//! the hash of its changes, and unidentified without git or outside its own
//! checkout; then git asked about a real checkout.

use std::path::{Path, PathBuf};
use std::process::Command;

use rstest::rstest;

use super::*;

/// A checkout of the workspace at `/w` with `diff` and `untracked`.
fn checkout(diff: &[u8], untracked: &[(&str, &[u8])]) -> Checkout {
    Checkout {
        top: PathBuf::from("/w"),
        commit: "c0ffee".to_owned(),
        diff: diff.to_vec(),
        untracked: (untracked.iter())
            .map(|(path, bytes)| ((*path).to_owned(), bytes.to_vec()))
            .collect(),
    }
}

/// The identity of `checkout` as a build of `/w`.
fn of(checkout: &Checkout) -> Identity {
    identity(Some(checkout), Path::new("/w"))
}

#[test]
fn a_clean_checkout_is_its_commit() {
    let identity = of(&checkout(b"", &[]));
    assert_eq!(identity.commit.as_deref(), Some("c0ffee"));
    assert_eq!(identity.tree, None);
}

/// Each change is a tree of its own: two dirty trees are never one.
#[rstest]
#[case::modified(checkout(b"+a", &[]), checkout(b"+b", &[]))]
#[case::untracked_bytes(checkout(b"", &[("x", b"1")]), checkout(b"", &[("x", b"2")]))]
#[case::untracked_name(checkout(b"", &[("x", b"1")]), checkout(b"", &[("y", b"1")]))]
#[case::untracked_or_modified(checkout(b"", &[("x", b"")]), checkout(b"x", &[]))]
#[case::more_untracked(checkout(b"", &[("x", b"")]), checkout(b"", &[("x", b""), ("y", b"")]))]
fn a_dirty_checkout_is_its_commit_and_its_changes(#[case] one: Checkout, #[case] other: Checkout) {
    let (one, other) = (of(&one), of(&other));
    assert_eq!(one.commit.as_deref(), Some("c0ffee"));
    assert!(one.tree.is_some() && other.tree.is_some());
    assert_ne!(one.tree, other.tree);
}

#[rstest]
#[case::no_git(None)]
#[case::outer_checkout(Some(Checkout { top: PathBuf::from("/"), ..checkout(b"", &[]) }))]
fn a_build_without_its_own_checkout_is_unidentified(#[case] found: Option<Checkout>) {
    let identity = identity(found.as_ref(), Path::new("/w"));
    assert_eq!(
        identity,
        Identity {
            commit: None,
            tree: None
        }
    );
}

/// `git args` in `dir`, with a fixed author; false when git is missing.
fn run(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// A scratch folder for the test `name`, empty, with links resolved.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nakkash-checkout-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A real checkout: clean, then modified, then with an untracked file,
/// and an ignored file that changes nothing.
#[test]
fn git_is_asked_about_a_real_checkout() {
    let dir = scratch("real");
    fs::write(dir.join("a.txt"), "one").unwrap();
    fs::write(dir.join(".gitignore"), "ignored\n").unwrap();
    if !run(&dir, &["init", "-q"]) {
        return; // No git here: nothing to ask.
    }
    assert!(run(&dir, &["add", "."]) && run(&dir, &["commit", "-q", "-m", "first"]));
    let ask = || identity(probe(&dir).as_ref(), &dir);
    let clean = ask();
    assert_eq!(clean.commit.as_ref().map(String::len), Some(40));
    assert_eq!(clean.tree, None);

    fs::write(dir.join("ignored"), "anything").unwrap();
    assert_eq!(ask(), clean);

    fs::write(dir.join("a.txt"), "two").unwrap();
    let modified = ask();
    assert_eq!(modified.commit, clean.commit);
    assert!(modified.tree.is_some());

    fs::write(dir.join("b.txt"), "new").unwrap();
    let untracked = ask();
    assert!(untracked.tree.is_some());
    assert_ne!(untracked.tree, modified.tree);

    // Below the top folder, the checkout is not the workspace's own.
    let inner = dir.join("inner");
    fs::create_dir(&inner).unwrap();
    assert_eq!(identity(probe(&inner).as_ref(), &inner).commit, None);
}

#[test]
fn a_folder_outside_any_checkout_has_none() {
    let dir = std::env::temp_dir().canonicalize().unwrap();
    let found = probe(&dir);
    assert!(found.is_none_or(|checkout| checkout.top != dir));
}

/// The sources and the lock file are watched, so an edit anywhere in them
/// names the build again.
#[test]
fn the_sources_are_watched() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let watched = watched(&root);
    for name in ["crates", "vendor", "Cargo.toml", "Cargo.lock"] {
        assert!(watched.contains(&root.join(name)), "{name}: {watched:?}");
    }
}
