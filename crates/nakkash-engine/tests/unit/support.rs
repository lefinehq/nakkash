//! Helpers shared by the unit tests.

use std::sync::{Mutex, OnceLock, PoisonError};

use tracing::Dispatch;
use tracing::Subscriber;
use tracing::subscriber::NoSubscriber;

/// Held while a test runs under a scoped subscriber.
///
/// Tracing keeps one global maximum level and interest cache for every
/// dispatcher in the process. `cargo test` runs tests as threads of one
/// process, and a subscriber that one test registers or drops recomputes
/// that state while another test's spans are being created, which can
/// briefly disable them (a timing test then sees only some of its steps).
/// Taking turns removes the race; tests that need no subscriber still run
/// in parallel.
static SCOPED: Mutex<()> = Mutex::new(());

/// Two dispatchers that record nothing, registered for the whole process.
///
/// While at most one dispatcher is registered, tracing-core caches a new
/// callsite's interest from the registering thread's own default, without
/// the lock that orders it after other registrations. A test thread with no
/// subscriber that first draws a step therefore caches "never" for the step
/// span, and when that write lands after a scoped test's rebuild, the
/// scoped test sees none of its steps (it counted 0 renders of 2 in CI).
/// With two dispatchers always registered, every interest is computed under
/// that lock, from every live dispatcher. A registration already under way
/// when the first scoped test runs is not covered; CI runs the tests under
/// nextest, one process per test, where none is.
static KEEPERS: OnceLock<[Dispatch; 2]> = OnceLock::new();

/// Runs `body` with `subscriber` as the scoped default, one such test at a
/// time, with tracing's global interest cache rebuilt for this subscriber.
pub(crate) fn with_subscriber<T>(
    subscriber: impl Subscriber + Send + Sync + 'static,
    body: impl FnOnce() -> T,
) -> T {
    KEEPERS.get_or_init(|| [NoSubscriber::new(), NoSubscriber::new()].map(Dispatch::new));
    // A failed test poisons the lock; the next one may still run.
    let _turn = SCOPED.lock().unwrap_or_else(PoisonError::into_inner);
    tracing::subscriber::with_default(subscriber, || {
        tracing::callsite::rebuild_interest_cache();
        body()
    })
}

/// A one-page PDF file written by hand, `width` by `height` points, whose
/// page holds `page` (more page keys, such as `/ArtBox`) and `resources`
/// and draws `content`.
pub(crate) fn pdf(width: u32, height: u32, page: &str, resources: &str, content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] {page} \
             /Resources << {resources} >> /Contents 4 0 R >>"
        ),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ];
    let mut file = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(file.len());
        file.extend(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = file.len();
    file.extend(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        file.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    file.extend(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    file
}
