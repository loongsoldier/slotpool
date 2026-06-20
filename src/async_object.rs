//! Async wrapper around [`ObjectPool`] — `request` awaits when the pool is full.
//!
//! Requires feature `async`.
//!
//! ## ISR caveat
//!
//! Dropping an [`AsyncObjectGuard`] inside an interrupt handler on **Cortex-M0/M0+**
//! may panic due to non-nestable `cpsid`/`cpsie`.  On M3+ this is safe.
//! For ISR use, prefer the sync [`ObjectPool`](crate::ObjectPool) directly.

use core::cell::UnsafeCell;
use core::mem::{ManuallyDrop, MaybeUninit};
use core::ops::{Deref, DerefMut};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::semaphore::FairSemaphore;
use embassy_sync::semaphore::Semaphore;

use crate::critical::CriticalSlots;
use crate::free_slots::FreeSlots;
use crate::object::{ObjectGuard, ObjectPool};

/// Async variant of [`ObjectPool`](crate::ObjectPool) — `request` awaits when full.
pub struct AsyncObjectPool<T, const N: usize, B = CriticalSlots<N>> {
    inner: ObjectPool<T, N, B>,
    wake: FairSemaphore<CriticalSectionRawMutex, N>,
}

// SAFETY: inner is Sync, FairSemaphore is Sync.
unsafe impl<T, const N: usize, B: FreeSlots<N>> Sync for AsyncObjectPool<T, N, B> {}

/// Static storage for an [`AsyncObjectPool`].
///
/// ```ignore
/// use slotpool::async_object::StaticAsyncObjectPool;
///
/// static POOL: StaticAsyncObjectPool<[u8; 256], 8> = StaticAsyncObjectPool::new();
/// let pool = POOL.init_pool([0u8; 256]);
/// ```
pub struct StaticAsyncObjectPool<T, const N: usize, B: FreeSlots<N> = CriticalSlots<N>> {
    data: UnsafeCell<MaybeUninit<AsyncObjectPool<T, N, B>>>,
}

unsafe impl<T, const N: usize, B: FreeSlots<N>> Sync for StaticAsyncObjectPool<T, N, B> {}

impl<T: Clone, const N: usize, B: FreeSlots<N>> StaticAsyncObjectPool<T, N, B> {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            data: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    pub fn init_pool(&'static self, init: T) -> &'static AsyncObjectPool<T, N, B> {
        assert!(N > 0, "Pool capacity must be greater than 0");
        unsafe {
            let ptr: *mut AsyncObjectPool<T, N, B> =
                self.data.get().cast::<AsyncObjectPool<T, N, B>>();
            ptr.write(AsyncObjectPool {
                inner: ObjectPool {
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
            for i in 0..N {
                (*ptr).inner.slot_mut_ptr(i).write(init.clone());
            }
            &*ptr
        }
    }
}

impl<T, const N: usize, B: FreeSlots<N>> AsyncObjectPool<T, N, B> {
    /// Borrow a slot, awaiting until one is free.
    pub async fn request(&'static self) -> AsyncObjectGuard<T, N, B> {
        loop {
            match self.inner.request() {
                Some(guard) => {
                    return AsyncObjectGuard {
                        guard: ManuallyDrop::new(guard),
                        wake: &self.wake,
                    };
                }
                None => {
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

/// Async borrow of an [`ObjectPool`] slot.  Behaves like [`ObjectGuard`](crate::ObjectGuard)
/// but wakes a waiter on drop.
#[must_use]
pub struct AsyncObjectGuard<T: 'static, const N: usize, B: FreeSlots<N> + 'static> {
    guard: ManuallyDrop<ObjectGuard<T, N, B>>,
    wake: &'static FairSemaphore<CriticalSectionRawMutex, N>,
}

impl<T, const N: usize, B: FreeSlots<N>> Deref for AsyncObjectGuard<T, N, B> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T, const N: usize, B: FreeSlots<N>> DerefMut for AsyncObjectGuard<T, N, B> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T, const N: usize, B: FreeSlots<N>> Drop for AsyncObjectGuard<T, N, B> {
    fn drop(&mut self) {
        // 1. Release the slot (no destructor — data persists).
        unsafe {
            ManuallyDrop::drop(&mut self.guard);
        }
        // 2. Wake one waiter.
        self.wake.release(1);
    }
}

unsafe impl<T: Send, const N: usize, B: FreeSlots<N>> Send for AsyncObjectGuard<T, N, B> {}
