#[cfg(debug_assertions)]
use std::cell::Cell;

use rstest::rstest;

use super::*;

/// A layout of `size` bytes aligned to `align`.
fn layout(size: usize, align: usize) -> Layout {
    Layout::from_size_align(size, align).expect("a layout")
}

#[rstest]
#[case::small(layout(64, 8), Route::System)]
#[case::one_byte_short_of_big(layout(BIG - 1, 1), Route::System)]
#[case::big(layout(BIG, 1), Route::Class(0))]
#[case::big_aligned_to_a_page(layout(BIG, 4096), Route::Class(0))]
#[case::big_aligned_past_a_page(layout(BIG, 8192), Route::System)]
#[case::a_1080p_float_grid(layout(1920 * 1080 * 16, 16), Route::Class(80))]
fn a_layout_takes_the_system_s_route_or_a_class(#[case] layout: Layout, #[case] to: Route) {
    #[cfg(unix)]
    assert_eq!(route(layout), to);
    #[cfg(not(unix))]
    assert_eq!(route(layout), Route::System, "{to:?}");
}

#[cfg(debug_assertions)]
#[test]
fn entering_twice_is_seen() {
    let inside = Cell::new(false);
    assert!(!entered(&inside), "the first entry");
    assert!(entered(&inside), "an entry from inside");
    inside.set(false);
    assert!(!entered(&inside), "an entry after leaving");
}

#[cfg(debug_assertions)]
#[test]
fn the_guard_marks_the_thread_inside_until_dropped() {
    assert!(!INSIDE.with(Cell::get));
    {
        let _inside = Inside::enter();
        assert!(INSIDE.with(Cell::get));
    }
    assert!(!INSIDE.with(Cell::get));
}

#[test]
fn the_reentry_message_is_pinned() {
    insta::assert_snapshot!(String::from_utf8_lossy(REENTERED));
}
