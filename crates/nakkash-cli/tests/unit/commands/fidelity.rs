//! Unit tests for `src/commands/fidelity.rs`: the record cap as
//! `--record-cap` reads it, and the warnings a build that cannot name its
//! code, or a machine that does not know its system or code generation,
//! gives for the records it writes.

use pretty_assertions::assert_eq;
use rstest::rstest;

use super::*;
use nakkash_engine::record::Hex128;

#[rstest]
#[case::whole("8", 8 << 30)]
#[case::half("0.5", 1 << 29)]
#[case::rounded_down("0.000001", 1073)]
#[case::huge("1e300", u64::MAX)]
fn a_cap_is_read_in_gibibytes(#[case] text: &str, #[case] bytes: u64) {
    assert_eq!(text.parse::<RecordCap>().unwrap().bytes(), bytes);
}

#[rstest]
#[case::zero("0")]
#[case::negative("-1")]
#[case::not_a_number("NaN")]
#[case::infinite("inf")]
#[case::a_word("big")]
#[case::with_a_unit("8GiB")]
#[case::rounds_to_nothing("1e-12")]
fn a_cap_that_is_not_a_positive_size_is_refused(#[case] text: &str) {
    assert_eq!(
        text.parse::<RecordCap>().unwrap_err(),
        format!("`{text}` is not a positive number of gibibytes, such as `8`")
    );
}

#[rstest]
#[case::identified(Some("abc"), false)]
#[case::unidentified(None, true)]
fn only_an_unidentified_build_warns(#[case] commit: Option<&str>, #[case] warns: bool) {
    let build = Build {
        commit: commit.map(str::to_owned),
        tree: None,
        profile: "test".to_owned(),
    };
    let warning = unidentified(&build);
    assert_eq!(warning.is_some(), warns);
    assert!(warning.is_none_or(|w| w.starts_with("warning: ") && w.contains("cannot be reused")));
}

/// A machine that does not know its system or code generation warns that
/// the records it writes cannot be reused, naming the part; one that
/// knows every part does not.
#[rstest]
#[case::known(|_: &mut Platform| {}, None)]
#[case::unknown_system(|p: &mut Platform| p.system = None, Some("its system is unknown"))]
#[case::unknown_codegen(
    |p: &mut Platform| p.codegen = None,
    Some("its code generation is unknown")
)]
fn only_a_platform_with_an_unknown_part_warns(
    #[case] change: fn(&mut Platform),
    #[case] names: Option<&str>,
) {
    let mut platform = Platform {
        target: "aarch64-macos".to_owned(),
        simd: "neon".to_owned(),
        system: Some("macOS 26.3, build 25D125".to_owned()),
        codegen: Some(Hex128(0xc0de)),
    };
    change(&mut platform);
    let warning = unknown_platform(&platform);
    assert_eq!(warning.is_some(), names.is_some(), "{warning:?}");
    if let (Some(warning), Some(names)) = (warning, names) {
        assert!(warning.starts_with("warning: "), "{warning}");
        assert!(warning.contains("cannot be reused"), "{warning}");
        assert!(warning.contains(names), "{warning}");
    }
}
