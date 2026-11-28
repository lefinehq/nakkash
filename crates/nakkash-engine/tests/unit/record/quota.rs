//! Unit tests for `src/record/quota.rs`: a cap that does not fit in the
//! free space is refused before anything is written, a write that would
//! pass the cap is refused before it is made, and a quota once refused
//! stays spent.

use std::sync::Arc;

use pretty_assertions::assert_eq;

use super::*;
use crate::record::tests::scratch;
use nakkash_format::Kind;

#[test]
fn a_cap_must_fit_in_the_free_space() {
    let dir = scratch("quota-free");
    let error = Quota::within(&dir, u64::MAX).unwrap_err();
    assert_eq!(error.kind, Kind::Io, "{error}");
    assert!(error.message.contains("free"), "{error}");
    // A folder not made yet is measured on the folder that would hold it.
    let quota = Quota::within(&dir.join("not/made/yet"), 1000).unwrap();
    assert_eq!((quota.cap(), quota.taken()), (1000, 0));
    // From here too: a relative folder is measured from the working one.
    assert!(Quota::within(Path::new("records-not-made"), 1000).is_ok());
}

#[test]
fn a_write_past_the_cap_is_refused_and_the_quota_spent() {
    let dir = scratch("quota-take");
    let quota = Quota::within(&dir, 1000).unwrap();
    quota.check().unwrap();
    quota.take(&dir.join("a"), 600).unwrap();
    quota.take(&dir.join("b"), 400).unwrap();
    assert_eq!(quota.taken(), 1000);
    let error = quota.take(&dir.join("c"), 1).unwrap_err();
    assert_eq!(error.kind, Kind::Io, "{error}");
    assert_eq!(quota.taken(), 1000, "a refused write takes nothing");
    // Once refused, nothing more is recorded, however small.
    let spent = quota.check().unwrap_err();
    assert_eq!(spent.kind, Kind::Io, "{spent}");
    assert!(quota.take(&dir.join("d"), 0).is_err());
}

/// Threads writing the frames of one run share its quota: together they
/// take no more than its cap.
#[test]
fn threads_share_one_cap() {
    let dir = scratch("quota-threads");
    let quota = Arc::new(Quota::within(&dir, 1000).unwrap());
    let taken: usize = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let (quota, dir) = (&quota, &dir);
                scope.spawn(move || {
                    (0..10)
                        .filter(|j| quota.take(&dir.join(format!("{i}-{j}")), 30).is_ok())
                        .count()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    });
    assert!(taken * 30 <= 1000, "{taken}");
    assert_eq!(quota.taken(), taken as u64 * 30);
}

#[test]
fn every_quota_error_message() {
    let dir = scratch("quota-errors");
    let quota = Quota::within(&dir, 100).unwrap();
    let past = quota.take(&dir.join("f0000.cov"), 101).unwrap_err();
    let text = [
        Quota::within(&dir, u64::MAX).unwrap_err().to_string(),
        past.to_string(),
        quota.check().unwrap_err().to_string(),
    ]
    .join("\n")
    .replace(&dir.display().to_string(), "<scratch>");
    // The free space differs by machine and by the minute.
    let text = without_free(&text);
    insta::assert_snapshot!(text);
}

/// `text` with the count of free bytes written as `<free>`.
fn without_free(text: &str) -> String {
    let start = text.find("holds ").map(|i| i + "holds ".len());
    let end = text.find(" bytes free");
    match (start, end) {
        (Some(start), Some(end)) if start < end => {
            format!("{}<free>{}", &text[..start], &text[end..])
        }
        _ => text.to_owned(),
    }
}
