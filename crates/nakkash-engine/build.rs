//! Names the build a fidelity record was made with (`record::Build`): the
//! commit checked out, whether the tree had changes, and the profile.
//!
//! Without git, or outside a checkout, the commit is `unknown` and the tree
//! is clean. The script runs again when the checkout moves: when `HEAD`,
//! the branch it names or the index changes. A change to a file that is
//! not staged moves none of them, so `dirty` holds as of the last of those.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let commit = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    // `--no-optional-locks` keeps `status` from rewriting the index, which
    // would run this script again on every build.
    let dirty = git(&["--no-optional-locks", "status", "--porcelain"])
        .is_some_and(|changes| !changes.is_empty());
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_owned());
    println!("cargo:rustc-env=NAKKASH_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=NAKKASH_BUILD_DIRTY={dirty}");
    println!("cargo:rustc-env=NAKKASH_BUILD_PROFILE={profile}");
    for path in watched() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rerun-if-changed=build.rs");
}

/// What `git args` prints, trimmed; none when git is missing or fails.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The files that move when the checkout does: `HEAD` and the index of this
/// worktree, and the branch `HEAD` names with the packed refs, which a
/// worktree shares with the others. Only files that exist are watched,
/// since cargo runs the script on every build for a missing one.
fn watched() -> Vec<PathBuf> {
    let (Some(own), Some(common)) = (
        git(&["rev-parse", "--absolute-git-dir"]),
        git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]),
    ) else {
        return Vec::new();
    };
    let (own, common) = (PathBuf::from(own), PathBuf::from(common));
    let branch = git(&["symbolic-ref", "-q", "HEAD"]).map(|name| common.join(name));
    [
        own.join("HEAD"),
        own.join("index"),
        common.join("packed-refs"),
    ]
    .into_iter()
    .chain(branch)
    .filter(|path| path.exists())
    .collect()
}
