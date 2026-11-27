//! Unit tests for `src/record/types.rs`: hashes written as 32 hexadecimal
//! digits and nothing else, the one schema name, and the platform and build
//! a record is made on.

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

/// The build script names the commit checked out when it ran, which the
/// tests were built at.
#[test]
fn the_build_names_this_checkout() {
    let build = Build::current();
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    assert_eq!(build.commit, head.unwrap_or_else(|| "unknown".to_owned()));
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    assert_eq!(build.profile, profile);
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
