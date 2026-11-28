use super::*;

#[test]
fn the_spin_lock_lets_one_thread_in_at_a_time() {
    let count = Spin::new(0u64);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..10_000 {
                    let mut held = count.lock();
                    // A read and a write apart: a second thread inside would
                    // lose counts.
                    let seen = *held;
                    std::hint::spin_loop();
                    *held = seen + 1;
                }
            });
        }
    });
    assert_eq!(*count.lock(), 80_000);
}
