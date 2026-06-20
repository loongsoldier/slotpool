//! Critical-section backend: a `heapless::Vec` protected by
//! `critical_section::Mutex`.
//!
//! Every `take` / `put` / `fill` runs entirely inside a critical section,
//! so mutual exclusion is trivial and the correctness argument is simple.
//! The cost is a brief interrupt-off window (a few instructions on Cortex-M).
//! This backend works on any target that implements `critical-section`, so
//! it's the most portable choice and the default for `Pool`.

use crate::free_slots::{FillError, FreeSlots};
use core::cell::UnsafeCell;
use critical_section::Mutex;
use heapless::Vec;

pub struct CriticalSlots<const N: usize> {
    inner: Mutex<UnsafeCell<Vec<usize, N>>>,
}

impl<const N: usize> CriticalSlots<N> {
    /// `const fn` constructor.
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(UnsafeCell::new(Vec::new())),
        }
    }
}

impl<const N: usize> FreeSlots<N> for CriticalSlots<N> {
    fn new() -> Self {
        Self::new()
    }

    fn fill(&self, count: usize) -> Result<(), FillError> {
        if count > N {
            return Err(FillError);
        }
        critical_section::with(|cs| {
            // SAFETY: exclusive access inside the critical section.
            let vec = unsafe { &mut *self.inner.borrow(cs).get() };
            vec.clear();
            for i in 0..count {
                // count ≤ N is guaranteed by the caller (Pool); push won't fail.
                let _ = vec.push(i);
            }
        });
        Ok(())
    }

    fn take(&self) -> Option<usize> {
        critical_section::with(|cs| {
            // SAFETY: exclusive access inside the critical section.
            let vec = unsafe { &mut *self.inner.borrow(cs).get() };
            vec.pop()
        })
    }

    fn put(&self, idx: usize) {
        critical_section::with(|cs| {
            // SAFETY: exclusive access inside the critical section.
            let vec = unsafe { &mut *self.inner.borrow(cs).get() };
            // Should never exceed capacity, but we ignore failures defensively
            // since panicking on a drop path is unacceptable.
            let _ = vec.push(idx);
        })
    }
}

// SAFETY: `critical_section::Mutex` is Sync by design.
unsafe impl<const N: usize> Sync for CriticalSlots<N> {}
