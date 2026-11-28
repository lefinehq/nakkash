//! Unit tests for `src/stats.rs`: the compared frames are counted only by
//! a command that compared frames.

use rstest::rstest;

use super::*;

/// A command that compared no frame (`render`, `check`) prints no line of
/// them; one that drew or reused any prints both counts.
#[rstest]
#[case::none(0, 0, None)]
#[case::drawn(6, 0, Some("stats: compared frames: drawn 6, reused from a record 0"))]
#[case::reused(0, 6, Some("stats: compared frames: drawn 0, reused from a record 6"))]
#[case::both(2, 4, Some("stats: compared frames: drawn 2, reused from a record 4"))]
fn the_compared_frames_are_counted_only_when_there_were_any(
    #[case] drawn: u64,
    #[case] read: u64,
    #[case] line: Option<&str>,
) {
    assert_eq!(frames_line(FrameCounts { drawn, read }).as_deref(), line);
}
