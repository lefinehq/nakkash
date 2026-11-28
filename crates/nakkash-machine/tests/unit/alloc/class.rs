use proptest::prelude::*;
use rstest::rstest;

use super::*;

/// The largest page of the systems the store maps on (macOS on Apple
/// silicon): every class is a whole number of them.
const PAGE_MOST: usize = 16 * 1024;

#[rstest]
#[case::below_big_is_the_first(1, 0)]
#[case::big_is_the_first(BIG, 0)]
#[case::one_byte_past_big(BIG + 1, 1)]
#[case::one_step_past_big(BIG + BIG / 16, 1)]
#[case::a_step_and_a_byte(BIG + BIG / 16 + 1, 2)]
#[case::the_first_octave_s_top(2 * BIG - 1, 16)]
#[case::two_mebibytes(2 * BIG, 16)]
#[case::past_two_mebibytes(2 * BIG + 1, 17)]
#[case::a_1080p_float_grid(1920 * 1080 * 16, 16 * 4 + 16)]
#[case::a_1080p_byte_pixmap(1920 * 1080 * 4, 32 + 16)]
fn sizes_fall_in_their_class(#[case] size: usize, #[case] class: usize) {
    assert_eq!(class_of(size), class, "{size} bytes");
    assert!(class_bytes(class) >= size);
}

#[rstest]
#[case(0, BIG)]
#[case(1, BIG + BIG / 16)]
#[case(15, 2 * BIG - BIG / 16)]
#[case(16, 2 * BIG)]
#[case(17, 2 * BIG + BIG / 8)]
#[case(32, 4 * BIG)]
#[case(80, 32 * BIG)]
fn classes_step_by_a_sixteenth_of_their_octave(#[case] class: usize, #[case] bytes: usize) {
    assert_eq!(class_bytes(class), bytes);
}

#[test]
fn the_classes_cover_every_size_a_layout_can_have() {
    let last = CLASSES - 1;
    assert_eq!(class_of(isize::MAX as usize), last);
    assert!(class_bytes(last) >= isize::MAX as usize);
    #[cfg(target_pointer_width = "64")]
    assert_eq!(CLASSES, 689);
}

#[test]
fn every_class_is_page_aligned_and_larger_than_the_one_before() {
    for class in 0..CLASSES {
        let bytes = class_bytes(class);
        assert_eq!(bytes % PAGE_MOST, 0, "class {class}");
        if class > 0 {
            assert!(bytes > class_bytes(class - 1), "class {class}");
        }
        assert_eq!(class_of(bytes), class, "class {class}");
    }
}

proptest! {
    #[test]
    fn a_class_holds_its_size_with_at_most_a_sixteenth_wasted(size in BIG..=isize::MAX as usize) {
        let class = class_of(size);
        let bytes = class_bytes(class);
        prop_assert!(bytes >= size);
        // The smallest class that holds it.
        prop_assert!(class == 0 || class_bytes(class - 1) < size);
        prop_assert!((bytes - size) <= size / 16);
    }

    #[test]
    fn classes_rise_with_size(a in BIG..=isize::MAX as usize, b in BIG..=isize::MAX as usize) {
        let (small, large) = (a.min(b), a.max(b));
        prop_assert!(class_of(small) <= class_of(large));
    }

    #[test]
    fn a_class_s_bytes_are_in_that_class(class in 0..CLASSES) {
        prop_assert_eq!(class_of(class_bytes(class)), class);
        prop_assert_eq!(class_bytes(class) % PAGE_MOST, 0);
    }

    #[test]
    fn sizes_near_a_class_edge_fall_on_its_sides(class in 1..CLASSES, past in 1usize..4096) {
        let edge = class_bytes(class - 1);
        prop_assert_eq!(class_of(edge), class - 1);
        prop_assert_eq!(class_of(edge + past.min(class_bytes(class) - edge)), class);
    }
}
