//! The build this binary is, as its build script named it
//! (`build/checkout.rs`): what a fidelity record it makes says of its code.

use nakkash_engine::record::{Build, Hex128};

/// This build.
pub(crate) fn current() -> Build {
    from_script(
        env!("NAKKASH_BUILD_COMMIT"),
        env!("NAKKASH_BUILD_TREE"),
        env!("NAKKASH_BUILD_PROFILE"),
    )
}

/// The build the script named by `commit`, `tree` and `profile`, each empty
/// when it could not tell. A tree the script did not write as a hash leaves
/// the build unidentified, never clean.
fn from_script(commit: &str, tree: &str, profile: &str) -> Build {
    let tree = match tree {
        "" => Ok(None),
        hash => hash.parse::<Hex128>().map(Some),
    };
    let (commit, tree) = match (commit, tree) {
        ("", _) | (_, Err(_)) => (None, None),
        (commit, Ok(tree)) => (Some(commit.to_owned()), tree),
    };
    Build {
        commit,
        tree,
        profile: profile.to_owned(),
    }
}

#[cfg(test)]
#[path = "../tests/unit/build_identity.rs"]
mod tests;
