use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};

use crate::critical::CriticalSlots;
use crate::free_slots::FreeSlots;

/// Owned-value pool — like `heapless::BoxPool`.
///
/// [`alloc`](BoxPool::alloc) writes a value into a free slot and returns a
/// [`BoxGuard`].  When the guard is dropped the slot is freed **and** `T`'s
/// destructor runs.
///
/// Obtain a `&'static BoxPool` via [`StaticBoxPool::init_pool`].
pub struct BoxPool<T, const N: usize, B = CriticalSlots<N>> {
    pub(crate) slots: UnsafeCell<MaybeUninit<[T; N]>>,
    pub(crate) free: B,
}

// SAFETY: `B: FreeSlots<N>` provides its own synchronisation.
unsafe impl<T, const N: usize, B: FreeSlots<N>> Sync for BoxPool<T, N, B> {}

/// Static storage for a [`BoxPool`] — the only way to create one.
///
/// ```ignore
/// use slotpool::StaticBoxPool;
///
/// static MY_POOL: StaticBoxPool<[u8; 256], 8> = StaticBoxPool::new();
/// let pool = MY_POOL.init_pool();
/// ```
pub struct StaticBoxPool<T, const N: usize, B: FreeSlots<N> = CriticalSlots<N>> {
    data: UnsafeCell<MaybeUninit<BoxPool<T, N, B>>>,
}

// SAFETY: written once during initialisation, then shared via &BoxPool (which is Sync).
unsafe impl<T, const N: usize, B: FreeSlots<N>> Sync for StaticBoxPool<T, N, B> {}

impl<T, const N: usize, B: FreeSlots<N>> StaticBoxPool<T, N, B> {
    /// `const fn` constructor.
    ///
    /// ```ignore
    /// static POOL: StaticBoxPool<[u8; 256], 8> = StaticBoxPool::new();
    /// ```
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            data: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// Initialise the pool. Must be called exactly once before any [`BoxPool::alloc`].
    ///
    /// ```ignore
    /// static POOL: StaticBoxPool<[u8; 256], 8> = StaticBoxPool::new();
    /// let pool: &'static BoxPool<[u8; 256], 8> = POOL.init_pool();
    /// ```
    pub fn init_pool(&'static self) -> &'static BoxPool<T, N, B> {
        assert!(N > 0, "Pool capacity must be greater than 0");
        unsafe {
            let ptr: *mut BoxPool<T, N, B> = self.data.get().cast::<BoxPool<T, N, B>>();
            ptr.write(BoxPool::new());
            (*ptr).free.fill(N);
            &*ptr
        }
    }
}

impl<T, const N: usize, B: FreeSlots<N>> BoxPool<T, N, B> {
    fn new() -> Self {
        Self {
            slots: UnsafeCell::new(MaybeUninit::uninit()),
            free: B::new(),
        }
    }

    /// Allocate a slot and write `value` into it.
    /// Returns `Err(value)` when the pool is exhausted.
    pub fn alloc(&'static self, value: T) -> Result<BoxGuard<T, N, B>, T> {
        match self.free.take() {
            Some(idx) => {
                unsafe {
                    // SAFETY: idx is unique and not in use.
                    self.slot_mut_ptr(idx).write(value);
                }
                Ok(BoxGuard {
                    pool: self,
                    index: idx,
                })
            }
            None => Err(value),
        }
    }

    /// Drop the value at `idx` and return the slot to the free set.
    pub(crate) fn release(&self, idx: usize) {
        unsafe {
            self.slot_mut_ptr(idx).drop_in_place();
        }
        self.free.put(idx);
    }

    #[inline]
    pub(crate) unsafe fn slot_mut_ptr(&self, idx: usize) -> *mut T {
        unsafe {
            let base: *mut T = (*self.slots.get()).as_mut_ptr().cast::<T>();
            base.add(idx)
        }
    }

    #[inline]
    pub(crate) unsafe fn slot_ptr(&self, idx: usize) -> *const T {
        unsafe {
            let base: *const T = (*self.slots.get()).as_ptr().cast::<T>();
            base.add(idx)
        }
    }
}

/// A handle to an allocated [`BoxPool`] slot.  Derefs to `&mut T`; returns the
/// slot to the pool **and runs `T`'s destructor** on drop.
#[must_use = "discarding a BoxGuard immediately frees the slot — likely a bug"]
pub struct BoxGuard<T: 'static, const N: usize, B: FreeSlots<N> + 'static = CriticalSlots<N>> {
    pub(crate) pool: &'static BoxPool<T, N, B>,
    pub(crate) index: usize,
}

impl<T, const N: usize, B: FreeSlots<N>> Deref for BoxGuard<T, N, B> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.pool.slot_ptr(self.index) }
    }
}

impl<T, const N: usize, B: FreeSlots<N>> DerefMut for BoxGuard<T, N, B> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.pool.slot_mut_ptr(self.index) }
    }
}

impl<T, const N: usize, B: FreeSlots<N>> Drop for BoxGuard<T, N, B> {
    fn drop(&mut self) {
        self.pool.release(self.index);
    }
}

// SAFETY: BoxGuard holds exclusive ownership of its slot.
unsafe impl<T: Send, const N: usize, B: FreeSlots<N>> Send for BoxGuard<T, N, B> {}

impl<T, const N: usize, B: FreeSlots<N>> BoxGuard<T, N, B> {
    /// Consume the guard and return a raw pointer **without** freeing the slot.
    /// The caller must eventually call [`from_raw`](Self::from_raw) to reconstruct
    /// a `BoxGuard` and return the slot.
    ///
    /// # Panics
    ///
    /// Panics on ZSTs because a raw pointer cannot recover the slot index.
    ///
    /// ```
    /// use slotpool::{BoxPool, BoxGuard, StaticBoxPool};
    ///
    /// static POOL: StaticBoxPool<u32, 4> = StaticBoxPool::new();
    /// let pool: &'static BoxPool<u32, 4> = POOL.init_pool();
    ///
    /// let guard = pool.alloc(42).unwrap();
    /// let ptr = BoxGuard::into_raw(guard);
    /// // … pass ptr to DMA or C code …
    /// let guard = unsafe { BoxGuard::from_raw(pool, ptr) };
    /// ```
    pub fn into_raw(self) -> *mut T {
        assert!(
            core::mem::size_of::<T>() > 0,
            "BoxGuard::into_raw is not supported for zero-sized types (ZST)"
        );
        let ptr = unsafe { self.pool.slot_mut_ptr(self.index) };
        core::mem::forget(self);
        ptr
    }

    /// Reconstruct a `BoxGuard` from a raw pointer previously obtained via
    /// [`into_raw`](Self::into_raw).
    ///
    /// # Safety
    ///
    /// - `ptr` must be the exact pointer returned by `into_raw` on the same `pool`.
    /// - The `T` at `ptr` must still be valid.
    /// - Each slot must be returned exactly once (no double-free, no leak).
    pub unsafe fn from_raw(pool: &'static BoxPool<T, N, B>, ptr: *mut T) -> Self {
        unsafe {
            let base: *const T = (*pool.slots.get()).as_ptr().cast::<T>();
            let index = (ptr as *const T).offset_from(base) as usize;
            debug_assert!(index < N, "ptr is beyond pool.slots range");
            BoxGuard { pool, index }
        }
    }

    /// Clone the value into a fresh slot from the same pool.
    /// Returns `Err(cloned_value)` when the pool is full (no panic).
    pub fn try_clone(&self) -> Result<Self, T>
    where
        T: Clone,
    {
        self.pool.alloc(T::clone(self))
    }
}
