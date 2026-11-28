//! Unit tests for `src/commands/fidelity.rs`: the record cap as
//! `--record-cap` reads it, and the warning a build that cannot name its
//! code gives for the records it writes.

use pretty_assertions::assert_eq;
use rstest::rstest;

use super::*;

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
