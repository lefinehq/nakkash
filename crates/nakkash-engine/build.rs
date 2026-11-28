//! Names the code generation the engine is compiled with, for the fidelity
//! records it makes: the hash of the compiler's version and the flags that
//! change the machine code (`build/codegen.rs`), empty when the compiler
//! cannot say. The script runs again when the compiler, the toolchain or a
//! flag changes.

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
