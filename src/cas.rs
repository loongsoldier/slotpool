//! CAS-based lock-free backend (Treiber stack).
//!
//! Requires hardware CAS instructions (compilation will fail, not silently
//! degrade, on targets like thumbv6m that lack them).  Controlled by feature
//! `cas`.
//!
//! ## How it works
//!
//! - `next[i]` stores the index *below* node `i` in the free stack.
//! - `head` is an `AtomicUsize` pointing to the stack top; `N` (the capacity)
//!   acts as the sentinel for "empty".
//! - `take`: read `head`, record its `next`, CAS `head` ← `next`.
//! - `put`:  read `head`, set `next[idx]` to old `head`, CAS `head` ← `idx`.
//!
//! The "next" pointers live in a separate `[AtomicUsize; N]` array — **not
//! intrusive** into `T`'s slot memory.
//!
//! ## ABA
//!
//! Classic Treiber stacks are vulnerable to ABA when a node is popped, reused,
//! and pushed back so that `head` looks unchanged to a stale CAS.  In this
//! implementation indices are never reused across different slots (each index
//! permanently owns its `next` entry), so the "reallocated memory" flavour of
//! ABA is avoided.  On single-core systems (ISR vs. main loop) the hardware
//! LDREX/STREX pair is atomic and cannot be preempted mid-sequence.  On
//! multi-core SMP targets, however, the full formal proof is still an open
//! question — **review carefully or switch to the `critical` backend**.

use crate::free_slots::{FillError, FreeSlots};
use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};

pub struct CasSlots<const N: usize> {
    /// `next[i]` = index below node `i` in the free stack (or sentinel).
    /// Wrapped in `UnsafeCell<MaybeUninit<…>>` so `new()` can be `const fn`.
    next: UnsafeCell<MaybeUninit<[AtomicUsize; N]>>,
    /// Stack top; `N` is the empty-stack sentinel.
    head: AtomicUsize,
}

/// `N` itself is the sentinel value (valid indices are `0..N`).
const fn sentinel<const N: usize>() -> usize {
    N
}

impl<const N: usize> CasSlots<N> {
    /// `const fn` constructor.
    ///
    /// ```ignore
    /// use slotpool::cas::CasSlots;
    /// use slotpool::StaticBoxPool;
    ///
    /// static POOL: StaticBoxPool<[u8; 256], 8, CasSlots<8>> = StaticBoxPool::new();
    /// ```
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            next: UnsafeCell::new(MaybeUninit::uninit()),
            head: AtomicUsize::new(sentinel::<N>()),
        }
    }

    /// Borrow the `next` array.  Only safe after `fill()` has initialised it.
    #[inline]
    unsafe fn next_array(&self) -> &[AtomicUsize; N] {
        unsafe { &*(*self.next.get()).as_ptr() }
    }
}

impl<const N: usize> FreeSlots<N> for CasSlots<N> {
    fn new() -> Self {
        Self::new()
    }

    fn fill(&self, count: usize) -> Result<(), FillError> {
        if count > N {
            return Err(FillError);
        }

        unsafe {
            // Build the chain: count-1 → count-2 → … → 0 → sentinel.
            // Indices ≥ count stay at sentinel (never used).
            let next_ptr: *mut AtomicUsize = (*self.next.get()).as_mut_ptr().cast::<AtomicUsize>();
            for i in 0..N {
                let next = if i < count {
                    if i == 0 { sentinel::<N>() } else { i - 1 }
                } else {
                    sentinel::<N>()
                };
                next_ptr.add(i).write(AtomicUsize::new(next));
            }
        }

        let head_val = if count > 0 {
            count - 1
        } else {
            sentinel::<N>()
        };
        self.head.store(head_val, Ordering::Release);
        Ok(())
    }

    fn take(&self) -> Option<usize> {
        // SAFETY: fill() must be called before any take/put.
        let next = unsafe { self.next_array() };
        loop {
            let head = self.head.load(Ordering::Acquire);
            if head == sentinel::<N>() {
                return None;
            }
            let next_val = next[head].load(Ordering::Acquire);
            match self
                .head
                .compare_exchange(head, next_val, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Some(head),
                Err(_) => continue,
            }
        }
    }

    fn put(&self, idx: usize) {
        // SAFETY: fill() must be called before any take/put.
        let next = unsafe { self.next_array() };
        loop {
            let head = self.head.load(Ordering::Acquire);
            next[idx].store(head, Ordering::Release);
            match self
                .head
                .compare_exchange(head, idx, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return,
                Err(_) => continue,
            }
        }
    }
}

// SAFETY: all shared state is accessed via AtomicUsize.
unsafe impl<const N: usize> Sync for CasSlots<N> {}
