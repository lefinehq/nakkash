//! Names the code generation the engine is compiled with, for the fidelity
//! records it makes: the hash of the compiler's version and the flags that
//! change the machine code (`build/codegen.rs`), empty when the compiler
//! cannot say.
//!
//! The script runs again when a variable of cargo's own environment that
//! chooses the compiler, the toolchain or the flags changes
//! (`codegen::WATCHED`). The variables cargo sets for the script from
//! those (`codegen::FLAGS`) cannot be watched, as cargo watches only its
//! own environment; a change of flags or profile reaches the script
//! anyway, as cargo then builds every unit again, this script's run among
//! them.

#[path = "build/codegen.rs"]
mod codegen;

fn main() {
    let var = |name: &str| std::env::var(name).ok();
    let hash = codegen::hash(codegen::probe(var).as_ref())
        .map(|hash| format!("{hash:032x}"))
        .unwrap_or_default();
    println!("cargo:rustc-env=NAKKASH_CODEGEN={hash}");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build/codegen.rs");
    for name in codegen::WATCHED {
        println!("cargo:rerun-if-env-changed={name}");
    }
}
