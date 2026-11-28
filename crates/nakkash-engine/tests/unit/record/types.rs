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
fn the_platform_names_this_processor_its_simd_level_system_and_code_generation() {
    let platform = Platform::current();
    assert_eq!(
        platform.target,
        format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
    );
    assert_eq!(platform.simd, nakkash_raster::simd_level());
    assert_eq!(platform.system, nakkash_machine::system());
    // The tests were built by cargo, whose compiler answers `-vV`.
    assert!(platform.codegen.is_some(), "{platform:?}");
    assert_eq!(Platform::current(), platform);
}

/// The code generation is read from what the build script wrote: a hash,
/// or nothing when the script could not tell; anything else is unknown
/// too, never a hash.
#[rstest]
#[case::hash("0000000000000000000000000000c0de", Some(Hex128(0xc0de)))]
#[case::unknown("", None)]
#[case::short("c0de", None)]
#[case::upper("0000000000000000000000000000C0DE", None)]
fn the_code_generation_is_read_from_the_build_script(
    #[case] script: &str,
    #[case] read: Option<Hex128>,
) {
    assert_eq!(codegen_from_script(script), read);
}

/// A platform of this machine, as another machine might have written it.
fn platform() -> Platform {
    Platform {
        target: "aarch64-macos".to_owned(),
        simd: "neon".to_owned(),
        system: Some("macOS 26.3, build 25D125".to_owned()),
        codegen: Some(Hex128(0xc0de)),
    }
}

/// Two platforms match when every part is known and equal. The first part
/// that differs, or is unknown on either side, is named: an unknown
/// system or code generation matches nothing, not even another unknown.
#[rstest]
#[case::same(|_: &mut Platform| {}, |_: &mut Platform| {}, None)]
#[case::target(
    |p: &mut Platform| p.target = "x86_64-linux".to_owned(),
    |_: &mut Platform| {},
    Some("its target is x86_64-linux, and this machine's is aarch64-macos")
)]
#[case::simd(
    |p: &mut Platform| p.simd = "avx2".to_owned(),
    |_: &mut Platform| {},
    Some("its SIMD level is avx2, and this machine's is neon")
)]
#[case::system(
    |p: &mut Platform| p.system = Some("macOS 26.2, build 25C56".to_owned()),
    |_: &mut Platform| {},
    Some("its system is macOS 26.2, build 25C56, and this machine's is macOS 26.3, build 25D125")
)]
#[case::codegen(
    |p: &mut Platform| p.codegen = Some(Hex128(0xbad)),
    |_: &mut Platform| {},
    Some("its code generation is 00000000000000000000000000000bad, and this machine's is 0000000000000000000000000000c0de")
)]
#[case::system_unknown_there(
    |p: &mut Platform| p.system = None,
    |_: &mut Platform| {},
    Some("its system is unknown, and an unknown system matches none")
)]
#[case::system_unknown_here(
    |_: &mut Platform| {},
    |p: &mut Platform| p.system = None,
    Some("this machine's system is unknown, and an unknown system matches none")
)]
#[case::system_unknown_on_both(
    |p: &mut Platform| p.system = None,
    |p: &mut Platform| p.system = None,
    Some("its system is unknown, and an unknown system matches none")
)]
#[case::codegen_unknown_on_both(
    |p: &mut Platform| p.codegen = None,
    |p: &mut Platform| p.codegen = None,
    Some("its code generation is unknown, and an unknown code generation matches none")
)]
#[case::first_part_named(
    |p: &mut Platform| {
        p.simd = "avx2".to_owned();
        p.codegen = None;
    },
    |_: &mut Platform| {},
    Some("its SIMD level is avx2, and this machine's is neon")
)]
fn a_platform_matches_only_when_every_part_is_known_and_equal(
    #[case] there: fn(&mut Platform),
    #[case] here: fn(&mut Platform),
    #[case] unlike: Option<&str>,
) {
    let (mut theirs, mut ours) = (platform(), platform());
    there(&mut theirs);
    here(&mut ours);
    assert_eq!(theirs.unlike(&ours).as_deref(), unlike);
}

/// A platform's system and code generation are required, null or not, so
/// a record written before they were named is refused, not read as one
/// whose parts are unknown.
#[rstest]
#[case::no_system(r#"{"target":"aarch64-macos","simd":"neon","codegen":null}"#)]
#[case::no_codegen(r#"{"target":"aarch64-macos","simd":"neon","system":null}"#)]
#[case::extra(
    r#"{"target":"aarch64-macos","simd":"neon","system":null,"codegen":null,"cpu":"m4"}"#
)]
fn a_platform_without_every_part_is_refused(#[case] json: &str) {
    assert!(serde_json::from_str::<Platform>(json).is_err(), "{json}");
    let unknown = r#"{"target":"aarch64-macos","simd":"neon","system":null,"codegen":null}"#;
    let read: Platform = serde_json::from_str(unknown).unwrap();
    assert_eq!((read.system, read.codegen), (None, None));
    let json = serde_json::to_string(&platform()).unwrap();
    assert_eq!(serde_json::from_str::<Platform>(&json).unwrap(), platform());
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
