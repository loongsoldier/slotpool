//! Async wrapper around [`BoxPool`] — `alloc` awaits when the pool is full.
//!
//! Requires feature `async`.
//!
//! ## ISR caveat
//!
//! Dropping an [`AsyncBoxGuard`] inside an interrupt handler on **Cortex-M0/M0+**
//! may panic due to non-nestable `cpsid`/`cpsie`.  On M3+ this is safe.
//! For ISR use, prefer the sync [`BoxPool`](crate::BoxPool) directly.

use core::cell::UnsafeCell;
use core::mem::{ManuallyDrop, MaybeUninit};
use core::ops::{Deref, DerefMut};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::semaphore::FairSemaphore;
use embassy_sync::semaphore::Semaphore;

use crate::boxed::{BoxGuard, BoxPool};
use crate::critical::CriticalSlots;
use crate::free_slots::FreeSlots;

/// Async variant of [`BoxPool`](crate::BoxPool) — `alloc` awaits when full.
pub struct AsyncBoxPool<T, const N: usize, B = CriticalSlots<N>> {
    inner: BoxPool<T, N, B>,
    wake: FairSemaphore<CriticalSectionRawMutex, N>,
}

// SAFETY: inner is Sync, FairSemaphore is Sync.
unsafe impl<T, const N: usize, B: FreeSlots<N>> Sync for AsyncBoxPool<T, N, B> {}

/// Static storage for an [`AsyncBoxPool`].
///
/// ```ignore
/// use slotpool::async_boxed::StaticAsyncBoxPool;
///
/// static POOL: StaticAsyncBoxPool<[u8; 256], 8> = StaticAsyncBoxPool::new();
/// let pool = POOL.init_pool();
/// ```
pub struct StaticAsyncBoxPool<T, const N: usize, B: FreeSlots<N> = CriticalSlots<N>> {
    data: UnsafeCell<MaybeUninit<AsyncBoxPool<T, N, B>>>,
}

unsafe impl<T, const N: usize, B: FreeSlots<N>> Sync for StaticAsyncBoxPool<T, N, B> {}

impl<T, const N: usize, B: FreeSlots<N>> StaticAsyncBoxPool<T, N, B> {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            data: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    pub fn init_pool(&'static self) -> &'static AsyncBoxPool<T, N, B> {
        assert!(N > 0, "Pool capacity must be greater than 0");
        unsafe {
            let ptr: *mut AsyncBoxPool<T, N, B> = self.data.get().cast::<AsyncBoxPool<T, N, B>>();
            ptr.write(AsyncBoxPool {
                inner: BoxPool {
                    slots: UnsafeCell::new(MaybeUninit::uninit()),
                    free: B::new(),
                },
                wake: FairSemaphore::new(0),
            });
            (*ptr)
                .inner
                .free
                .fill(N)
                .expect("fill(N) must succeed when count == capacity");
            &*ptr
        }
    }
}

impl<T, const N: usize, B: FreeSlots<N>> AsyncBoxPool<T, N, B> {
    /// Allocate a slot, awaiting until one is free.
    pub async fn alloc(&'static self, mut value: T) -> AsyncBoxGuard<T, N, B> {
        loop {
            match self.inner.alloc(value) {
                Ok(guard) => {
                    return AsyncBoxGuard {
                        guard: ManuallyDrop::new(guard),
                        wake: &self.wake,
                    };
                }
                Err(val) => {
                    value = val;
                    let _ = self
                        .wake
                        .acquire(1)
                        .await
                        .expect("semaphore acquire(1) must not fail");
                }
            }
        }
    }
}

/// Async handle to a [`BoxPool`] slot.  Behaves like [`BoxGuard`](crate::BoxGuard)
/// but wakes a waiter on drop.
#[must_use]
pub struct AsyncBoxGuard<T: 'static, const N: usize, B: FreeSlots<N> + 'static> {
    guard: ManuallyDrop<BoxGuard<T, N, B>>,
    wake: &'static FairSemaphore<CriticalSectionRawMutex, N>,
}

impl<T, const N: usize, B: FreeSlots<N>> Deref for AsyncBoxGuard<T, N, B> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T, const N: usize, B: FreeSlots<N>> DerefMut for AsyncBoxGuard<T, N, B> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T, const N: usize, B: FreeSlots<N>> Drop for AsyncBoxGuard<T, N, B> {
    fn drop(&mut self) {
        // 1. Release the slot (runs T's destructor).
        unsafe {
            ManuallyDrop::drop(&mut self.guard);
        }
        // 2. Wake one waiter.
        self.wake.release(1);
    }
}

unsafe impl<T: Send, const N: usize, B: FreeSlots<N>> Send for AsyncBoxGuard<T, N, B> {}
