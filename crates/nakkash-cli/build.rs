//! Names the `nakkash` build for the fidelity records it makes: the commit
//! of the checkout, the hash of the tree's changes from it, and the cargo
//! profile (`build/checkout.rs`). The script runs again whenever a source
//! file, the lock file, `HEAD` or a branch changes.

use std::path::Path;

#[path = "build/checkout.rs"]
mod checkout;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    // Unresolvable only if the workspace is gone; then the build is
    // unidentified.
    let root = Path::new(&manifest)
        .join("../..")
        .canonicalize()
        .unwrap_or_default();
    let identity = checkout::identity(checkout::probe(&root).as_ref(), &root);
    let commit = identity.commit.unwrap_or_default();
    let tree = (identity.tree)
        .map(|tree| format!("{tree:032x}"))
        .unwrap_or_default();
    let profile = std::env::var("PROFILE").unwrap_or_default();
    println!("cargo:rustc-env=NAKKASH_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=NAKKASH_BUILD_TREE={tree}");
    println!("cargo:rustc-env=NAKKASH_BUILD_PROFILE={profile}");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build/checkout.rs");
    for path in checkout::watched(&root) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
