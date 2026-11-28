//! Unit tests for `src/build_identity.rs`: the build read from what its
//! script wrote, and never clean when the script could not tell.

use std::path::Path;

use rstest::rstest;

use super::*;

const TREE: &str = "0000000000000000000000000000d1f7";

#[rstest]
#[case::clean("abc", "", Some("abc"), None)]
#[case::dirty("abc", TREE, Some("abc"), Some(Hex128(0xd1f7)))]
#[case::no_commit("", TREE, None, None)]
#[case::not_a_hash("abc", "d1f7", None, None)]
fn the_build_is_read_from_its_script(
    #[case] commit: &str,
    #[case] tree: &str,
    #[case] read_commit: Option<&str>,
    #[case] read_tree: Option<Hex128>,
) {
    let build = from_script(commit, tree, "release");
    assert_eq!(build.commit.as_deref(), read_commit);
    assert_eq!(build.tree, read_tree);
    assert_eq!(build.profile, "release");
}

/// The build names the commit of the checkout the tests were built in, or
/// none without one.
#[test]
fn this_build_names_its_checkout() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let now = crate::checkout::identity(crate::checkout::probe(&root).as_ref(), &root);
    let build = current();
    assert_eq!(build.commit, now.commit);
    assert_eq!(build.identified(), now.commit.is_some());
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    assert_eq!(build.profile, profile);
}
