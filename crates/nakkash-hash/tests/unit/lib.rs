//! Unit tests for `src/lib.rs`: a hash pinned to its value, parts told
//! apart by their lengths and tags.

use proptest::prelude::*;

use super::*;

/// The same value in every run and on every machine, so a record written
/// on one day is checked on another.
#[test]
fn a_content_hash_is_the_same_everywhere() {
    assert_eq!(
        content_hash("pixels", b"nakkash"),
        0xc80f_6d55_92c7_8766_8f1d_ecda_27f0_ec85
    );
}

#[test]
fn a_content_hash_is_its_hasher_over_one_part() {
    let hash = ContentHasher::new("mask").bytes(b"ab").finish();
    assert_eq!(hash, content_hash("mask", b"ab"));
}

/// The tag and every byte part are written with their lengths, and numbers
/// are parts of their own.
#[test]
fn parts_are_told_apart() {
    let hashes = [
        content_hash("pixels", b"ab"),
        content_hash("mask", b"ab"),
        content_hash("pixels", b"ac"),
        content_hash("pixels", b"abc"),
        content_hash("pixelsa", b"b"),
        content_hash("pixels", b""),
        ContentHasher::new("pixels")
            .bytes(b"a")
            .bytes(b"b")
            .finish(),
        ContentHasher::new("pixels")
            .u32(3)
            .u32(2)
            .bytes(b"ab")
            .finish(),
        ContentHasher::new("pixels")
            .u32(2)
            .u32(3)
            .bytes(b"ab")
            .finish(),
        ContentHasher::new("pixels").u64(1).finish(),
    ];
    for (i, a) in hashes.iter().enumerate() {
        for b in &hashes[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

proptest! {
    #[test]
    fn hashing_twice_gives_one_value(tag in any::<String>(), bytes in any::<Vec<u8>>()) {
        prop_assert_eq!(content_hash(&tag, &bytes), content_hash(&tag, &bytes));
    }
}
