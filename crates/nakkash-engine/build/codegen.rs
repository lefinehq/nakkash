//! What decides the machine code the engine's maths compiles to, besides
//! its source: the compiler, and the flags cargo compiles with. The build
//! script names the engine's code generation by its hash ([`hash`]); a
//! fidelity record keeps it, so a record is never reused by a build whose
//! maths may round to other bits (another toolchain, `target-cpu=native`
//! and its fused multiply-adds, another optimisation level).
//!
//! The flags are the same for every crate of a build, so the engine's are
//! the rasteriser's; a change to them compiles every crate again anyway.
//! Asking the compiler ([`probe`]) is kept apart from hashing what it said
//! ([`hash`]), so each is tested on its own.

use std::process::Command;

use nakkash_hash::ContentHasher;

/// The tag of a code generation's hash.
const TAG: &str = "nakkash.build.codegen";

/// The variables cargo gives a build script that change the machine code,
/// in the order they are hashed: the compiler's flags (with
/// `-C target-cpu` and `-C target-feature`), the target features they
/// enable, the optimisation level and whether debug information is on.
pub(crate) const FLAGS: [&str; 4] = [
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_CFG_TARGET_FEATURE",
    "OPT_LEVEL",
    "DEBUG",
];

/// The variables whose change runs the build script again: the compiler,
/// the toolchain chosen and the flags as given, then [`FLAGS`].
pub(crate) const WATCHED: [&str; 8] = [
    "RUSTC",
    "RUSTUP_TOOLCHAIN",
    "RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    FLAGS[0],
    FLAGS[1],
    FLAGS[2],
    FLAGS[3],
];

/// What a build compiles with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Codegen {
    /// What `$RUSTC -vV` printed: the compiler's version, commit, host and
    /// LLVM version.
    pub(crate) rustc: Vec<u8>,
    /// Each of [`FLAGS`], in order; none when cargo did not set it.
    pub(crate) flags: [Option<String>; 4],
}

/// The hash of `codegen`: the compiler's answer, then each flag, set or
/// not, each part apart; none for a code generation not known.
pub(crate) fn hash(codegen: Option<&Codegen>) -> Option<u128> {
    let codegen = codegen?;
    let mut hasher = ContentHasher::new(TAG);
    hasher.bytes(&codegen.rustc);
    for flag in &codegen.flags {
        hasher
            .u32(u32::from(flag.is_some()))
            .bytes(flag.as_deref().unwrap_or_default().as_bytes());
    }
    Some(hasher.finish())
}

/// Reads the code generation from the build script's environment `var`:
/// asks the compiler `RUSTC` names for `-vV` and reads each of [`FLAGS`].
/// None when `RUSTC` is unset or empty, or the compiler cannot be run,
/// fails or prints nothing: then nothing says what compiled the maths.
pub(crate) fn probe(var: impl Fn(&str) -> Option<String>) -> Option<Codegen> {
    let rustc = var("RUSTC").filter(|rustc| !rustc.is_empty())?;
    let answer = Command::new(rustc).arg("-vV").output().ok()?;
    let rustc = (answer.status.success() && !answer.stdout.is_empty()).then_some(answer.stdout)?;
    Some(Codegen {
        rustc,
        flags: FLAGS.map(&var),
    })
}

#[cfg(test)]
#[path = "../tests/unit/build/codegen.rs"]
mod tests;
