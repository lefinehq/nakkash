//! What the checkout a build is made from says of the code: the commit, and
//! the hash of the tree's changes from it. The build script names the
//! `nakkash` build by it ([`identity`]); a fidelity record keeps it, so a
//! record is never reused by other code.
//!
//! Asking git ([`probe`]) is kept apart from deciding what it means
//! ([`identity`]), so the decision is tested on its own.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nakkash_hash::ContentHasher;

/// The tag of a tree's hash.
const TREE: &str = "nakkash.build.tree";

/// What git says of a checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Checkout {
    /// The checkout's top folder, as `git rev-parse --show-toplevel` gives
    /// it, with links resolved.
    pub(crate) top: PathBuf,
    /// The commit checked out.
    pub(crate) commit: String,
    /// The tracked changes from it, staged or not: `git diff HEAD --binary`.
    pub(crate) diff: Vec<u8>,
    /// Each untracked file git does not ignore, by its path from the top
    /// folder, with its bytes, in git's order.
    pub(crate) untracked: Vec<(String, Vec<u8>)>,
}

/// What a build says of its code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Identity {
    /// The commit; none when the build cannot tell what code it is.
    pub(crate) commit: Option<String>,
    /// The hash of the tree's changes from the commit; none when it has
    /// none, or the build cannot tell.
    pub(crate) tree: Option<u128>,
}

/// The identity of a build of the workspace at `root` from `checkout`.
/// Without a checkout (no git, or not a checkout), or from a checkout whose
/// top folder is not the workspace (one this workspace merely sits in), the
/// build is unidentified.
pub(crate) fn identity(checkout: Option<&Checkout>, root: &Path) -> Identity {
    match checkout {
        Some(checkout) if checkout.top == root => Identity {
            commit: Some(checkout.commit.clone()),
            tree: tree(checkout),
        },
        _ => Identity {
            commit: None,
            tree: None,
        },
    }
}

/// The hash of `checkout`'s changes: the diff, then each untracked file's
/// path and bytes; none when there are none.
fn tree(checkout: &Checkout) -> Option<u128> {
    if checkout.diff.is_empty() && checkout.untracked.is_empty() {
        return None;
    }
    let mut hasher = ContentHasher::new(TREE);
    hasher
        .bytes(&checkout.diff)
        .u64(checkout.untracked.len() as u64);
    for (path, bytes) in &checkout.untracked {
        hasher.bytes(path.as_bytes()).bytes(bytes);
    }
    Some(hasher.finish())
}

/// Asks git about the checkout at `root`; none when git is missing, `root`
/// is in no checkout, or an untracked file cannot be read.
pub(crate) fn probe(root: &Path) -> Option<Checkout> {
    let top = text(git(root, &["rev-parse", "--show-toplevel"])?);
    let commit = text(git(root, &["rev-parse", "HEAD"])?);
    // Plain diff: no external diff tool, conversion or colour from the
    // user's settings, so the bytes depend on the tree alone.
    let diff = git(
        root,
        &[
            "diff",
            "HEAD",
            "--binary",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
        ],
    )?;
    let listed = git(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    let top = fs::canonicalize(top).ok()?;
    let untracked = (listed.split(|byte| *byte == 0))
        .filter(|path| !path.is_empty())
        .map(|path| {
            let path = String::from_utf8_lossy(path).into_owned();
            let bytes = fs::read(top.join(&path)).ok()?;
            Some((path, bytes))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Checkout {
        top,
        commit,
        diff,
        untracked,
    })
}

/// The files whose change may change a build of the workspace at `root`
/// or its identity: the sources (`crates/`, `vendor/`, the root manifest
/// and lock file), the log of `HEAD`, which every move of it appends to,
/// and the branches. Only those that exist: cargo runs the script on every
/// build for one that does not.
pub(crate) fn watched(root: &Path) -> Vec<PathBuf> {
    let sources = ["crates", "vendor", "Cargo.toml", "Cargo.lock"].map(|name| root.join(name));
    let own = git(root, &["rev-parse", "--absolute-git-dir"]).map(text);
    let common = git(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map(text);
    let git_files = [
        own.map(|dir| Path::new(&dir).join("logs").join("HEAD")),
        common.map(|dir| Path::new(&dir).join("refs").join("heads")),
    ];
    (sources.into_iter())
        .chain(git_files.into_iter().flatten())
        .filter(|path| path.exists())
        .collect()
}

/// What `git args`, run in `root`, prints; none when git is missing or
/// fails.
fn git(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// `bytes` as text, trimmed.
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).trim().to_owned()
}

#[cfg(test)]
#[path = "../tests/unit/build/checkout.rs"]
mod tests;
