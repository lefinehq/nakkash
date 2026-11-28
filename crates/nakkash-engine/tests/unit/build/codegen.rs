//! Unit tests for `build/codegen.rs`: the code generation hashed from the
//! compiler's version and every flag that changes the machine code, each
//! part apart; and unknown, never hashed, when the compiler cannot say.

use std::collections::BTreeMap;

use rstest::rstest;

use super::*;

/// What a build on a plain release profile compiles with.
fn release() -> Codegen {
    Codegen {
        rustc: b"rustc 1.92.0 (ded5c06cf 2025-12-08)\nhost: aarch64-apple-darwin\nLLVM version: 21.1.3\n"
            .to_vec(),
        flags: [
            Some(String::new()),
            Some("aes,crc,neon".to_owned()),
            Some("3".to_owned()),
            Some("false".to_owned()),
        ],
    }
}

/// A build script's environment holding `vars`.
fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let vars: BTreeMap<String, String> = (vars.iter())
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    move |name| vars.get(name).cloned()
}

#[test]
fn the_same_code_generation_hashes_the_same() {
    assert_eq!(hash(Some(&release())), hash(Some(&release())));
    assert!(hash(Some(&release())).is_some());
}

/// Every part changes the hash: another compiler, target CPU, feature,
/// optimisation level or debug setting compiles other machine code, which
/// may round the same maths to other bits.
#[rstest]
#[case::compiler(|c: &mut Codegen| c.rustc = b"rustc 1.92.1 (aaaaaaaaa 2026-01-15)\n".to_vec())]
#[case::rustflags(|c: &mut Codegen| c.flags[0] = Some("-Ctarget-cpu=native".to_owned()))]
#[case::target_features(|c: &mut Codegen| c.flags[1] = Some("aes,crc,fma,neon".to_owned()))]
#[case::opt_level(|c: &mut Codegen| c.flags[2] = Some("2".to_owned()))]
#[case::debug(|c: &mut Codegen| c.flags[3] = Some("true".to_owned()))]
#[case::unset_from_empty(|c: &mut Codegen| c.flags[0] = None)]
#[case::moved_between_parts(|c: &mut Codegen| {
    c.flags[0] = Some("aes,crc,neon".to_owned());
    c.flags[1] = Some(String::new());
})]
fn every_part_changes_the_hash(#[case] change: fn(&mut Codegen)) {
    let mut changed = release();
    change(&mut changed);
    assert_ne!(hash(Some(&changed)), hash(Some(&release())));
}

/// A build whose compiler did not say what it is has no code generation
/// to name: unknown, never the hash of nothing.
#[test]
fn an_unknown_code_generation_has_no_hash() {
    assert_eq!(hash(None), None);
}

/// The flags are read from the variables cargo sets, in their order.
#[test]
fn the_probe_reads_each_flag_and_asks_the_compiler() {
    let probed = probe(env(&[
        ("RUSTC", "rustc"),
        ("CARGO_ENCODED_RUSTFLAGS", "-Ctarget-cpu=native"),
        ("CARGO_CFG_TARGET_FEATURE", "neon"),
        ("OPT_LEVEL", "3"),
    ]))
    .expect("rustc answers -vV");
    assert!(probed.rustc.starts_with(b"rustc "), "{:?}", probed.rustc);
    assert!(
        String::from_utf8_lossy(&probed.rustc).contains("\nhost: "),
        "{:?}",
        probed.rustc
    );
    assert_eq!(
        probed.flags,
        [
            Some("-Ctarget-cpu=native".to_owned()),
            Some("neon".to_owned()),
            Some("3".to_owned()),
            None,
        ]
    );
}

/// Without a compiler to ask, or one that fails or says nothing, the code
/// generation is unknown.
#[rstest]
#[case::no_compiler(&[("OPT_LEVEL", "3")])]
#[case::missing_compiler(&[("RUSTC", "/nakkash/no/such/rustc")])]
#[case::empty_compiler(&[("RUSTC", "")])]
#[case::failing_compiler(&[("RUSTC", "false")])]
#[case::silent_compiler(&[("RUSTC", "true")])]
fn without_a_compiler_that_answers_the_code_generation_is_unknown(#[case] vars: &[(&str, &str)]) {
    assert_eq!(probe(env(vars)), None);
}

/// The build script runs again when any input of the hash may have
/// changed: the compiler, the toolchain chosen, and every flag.
#[test]
fn every_input_is_watched() {
    for name in FLAGS
        .iter()
        .chain(&["RUSTC", "RUSTUP_TOOLCHAIN", "RUSTFLAGS"])
    {
        assert!(WATCHED.contains(name), "{name} is not watched");
    }
}
