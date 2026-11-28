//! The spin lock that guards each class's stack in the block store: the
//! allocator may not take a lock that could allocate or sleep in the
//! system, and each is held for a few instructions.

use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};

/// A spin lock: what guards each class's stack, since the allocator may
/// not take a lock that could allocate or sleep in the system, and each is
/// held for a few instructions.
pub(super) struct Spin<T> {
    /// Whether a thread holds it.
    locked: AtomicBool,
    /// What it guards.
    value: UnsafeCell<T>,
}

// SAFETY: the value is reached only through a guard, and `lock` hands out
// one guard at a time, so threads share it as they would through a mutex.
unsafe impl<T: Send> Sync for Spin<T> {}

impl<T> Spin<T> {
    /// A lock, not held, guarding `value`.
    pub(super) const fn new(value: T) -> Self {
        Spin {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Waits until no other thread holds the lock, then holds it until the
    /// guard is dropped.
    pub(super) fn lock(&self) -> Held<'_, T> {
        let mut spins = 0u32;
        while self
            .locked
            .compare_exchange_weak(false, true, Acquire, Relaxed)
            .is_err()
        {
            // Wait on reads, which share the cache line, until the lock
            // looks free, then try again.
            while self.locked.load(Relaxed) {
                spins = spins.wrapping_add(1);
                match spins % SPINS_BEFORE_YIELD {
                    // A holder the system stopped: give it the processor.
                    0 => std::thread::yield_now(),
                    _ => std::hint::spin_loop(),
                }
            }
        }
        Held {
            lock: self,
            _not_shared: PhantomData,
        }
    }
}

/// How many times a waiting thread spins before it yields the processor.
const SPINS_BEFORE_YIELD: u32 = 64;

/// A held [`Spin`]: the guarded value, and the lock let go when dropped.
pub(super) struct Held<'a, T> {
    /// The lock held.
    lock: &'a Spin<T>,
    /// Shared between threads only as a `&mut T` would be: a guard of a
    /// value that is `Send` but not `Sync` must not be.
    _not_shared: PhantomData<&'a mut T>,
}

impl<T> Deref for Held<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: the lock is held, so no other reference to the value is.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for Held<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the lock is held, so no other reference to the value is.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for Held<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Release);
    }
}

#[cfg(test)]
#[path = "../../tests/unit/alloc/spin.rs"]
mod tests;
