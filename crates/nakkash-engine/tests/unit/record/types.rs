//! Unit tests for `src/record/types.rs`: hashes written as 32 hexadecimal
//! digits and nothing else, the one schema name, the platform a record is
//! made on, and what a build says of itself.

use proptest::prelude::*;
use rstest::rstest;

use super::*;
use nakkash_format::Kind;

#[rstest]
#[case::zero(0, "00000000000000000000000000000000")]
#[case::one(1, "00000000000000000000000000000001")]
#[case::high(0xabcd << 112, "abcd0000000000000000000000000000")]
#[case::max(u128::MAX, "ffffffffffffffffffffffffffffffff")]
fn a_hash_is_written_as_32_lowercase_hex_digits(#[case] value: u128, #[case] text: &str) {
    assert_eq!(Hex128(value).to_string(), text);
    assert_eq!(
        serde_json::to_string(&Hex128(value)).unwrap(),
        format!("\"{text}\"")
    );
    assert_eq!(text.parse::<Hex128>().unwrap(), Hex128(value));
}

#[rstest]
#[case::empty("")]
#[case::short("0000000000000000000000000000001")]
#[case::long("000000000000000000000000000000001")]
#[case::upper("0000000000000000000000000000000A")]
#[case::sign("+000000000000000000000000000001")]
#[case::prefix("0x000000000000000000000000000001")]
#[case::blank(" 0000000000000000000000000000001")]
#[case::other("000000000000000000000000000000g1")]
fn a_hash_of_any_other_form_is_refused(#[case] text: &str) {
    let error = text.parse::<Hex128>().unwrap_err();
    assert_eq!(error.kind, Kind::Syntax);
    assert!(error.expected.is_some());
    assert!(serde_json::from_str::<Hex128>(&format!("\"{text}\"")).is_err());
}

#[test]
fn a_hash_that_is_not_text_is_refused() {
    assert!(serde_json::from_str::<Hex128>("1").is_err());
}

#[rstest]
#[case::other_version("\"nakkash.FidelityRecord.2\"")]
#[case::other_case("\"nakkash.fidelityrecord.1\"")]
#[case::empty("\"\"")]
#[case::number("1")]
fn only_the_one_schema_name_is_read(#[case] json: &str) {
    assert_eq!(
        serde_json::from_str::<Schema>("\"nakkash.FidelityRecord.1\"").unwrap(),
        Schema::V1
    );
    assert_eq!(
        serde_json::to_string(&Schema::V1).unwrap(),
        "\"nakkash.FidelityRecord.1\""
    );
    assert!(serde_json::from_str::<Schema>(json).is_err());
}

#[test]
fn the_platform_names_this_processor_and_its_simd_level() {
    let platform = Platform::current();
    assert_eq!(
        platform.target,
        format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
    );
    assert_eq!(platform.simd, nakkash_raster::simd_level());
}

/// A build is identified by its commit; a tree's hash says it was dirty.
#[rstest]
#[case::clean(Some("0123"), None, true, false)]
#[case::dirty(Some("0123"), Some(Hex128(7)), true, true)]
#[case::unidentified(None, None, false, false)]
fn a_build_says_whether_it_is_identified_and_dirty(
    #[case] commit: Option<&str>,
    #[case] tree: Option<Hex128>,
    #[case] identified: bool,
    #[case] dirty: bool,
) {
    let build = Build {
        commit: commit.map(str::to_owned),
        tree,
        profile: "release".to_owned(),
    };
    assert_eq!(build.identified(), identified);
    assert_eq!(build.dirty(), dirty);
    let json = serde_json::to_string(&build).unwrap();
    assert_eq!(serde_json::from_str::<Build>(&json).unwrap(), build);
}

proptest! {
    #[test]
    fn reading_any_text_as_a_hash_never_panics(text in any::<String>()) {
        let _ = text.parse::<Hex128>();
        let _ = serde_json::from_str::<Hex128>(&serde_json::to_string(&text).unwrap());
    }

    #[test]
    fn a_hash_reads_back_as_written(value in any::<u128>()) {
        let text = Hex128(value).to_string();
        prop_assert_eq!(text.parse::<Hex128>().unwrap(), Hex128(value));
    }
}
