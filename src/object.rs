use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};

use crate::critical::CriticalSlots;
use crate::free_slots::FreeSlots;

/// Persistent-object pool — like `heapless::ObjectPool`.
///
/// Slots are initialised once via [`StaticObjectPool::init_pool`], then
/// [`request`](ObjectPool::request) borrows a slot **without** writing a value
/// and **without** running the destructor on drop — the data survives across
/// borrow cycles. Ideal for DMA buffers, message frames, and state machines.
pub struct ObjectPool<T, const N: usize, B = CriticalSlots<N>> {
    pub(crate) slots: UnsafeCell<MaybeUninit<[T; N]>>,
    pub(crate) free: B,
}

// SAFETY: `B: FreeSlots<N>` provides its own synchronisation.
unsafe impl<T, const N: usize, B: FreeSlots<N>> Sync for ObjectPool<T, N, B> {}

/// Static storage for an [`ObjectPool`] — the only way to create one.
///
/// ```ignore
/// use slotpool::StaticObjectPool;
///
/// static MY_POOL: StaticObjectPool<[u8; 256], 8> = StaticObjectPool::new();
/// let pool = MY_POOL.init_pool([0u8; 256]);
/// ```
pub struct StaticObjectPool<T, const N: usize, B: FreeSlots<N> = CriticalSlots<N>> {
    data: UnsafeCell<MaybeUninit<ObjectPool<T, N, B>>>,
}

// SAFETY: written once during initialisation, then shared via &ObjectPool (which is Sync).
unsafe impl<T, const N: usize, B: FreeSlots<N>> Sync for StaticObjectPool<T, N, B> {}

impl<T: Clone, const N: usize, B: FreeSlots<N>> StaticObjectPool<T, N, B> {
    /// `const fn` constructor.
    ///
    /// ```ignore
    /// static POOL: StaticObjectPool<[u8; 256], 8> = StaticObjectPool::new();
    /// ```
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            data: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// Initialise the pool, cloning `init` into every slot.
    /// Must be called exactly once before any [`ObjectPool::request`].
    ///
    /// ```ignore
    /// static POOL: StaticObjectPool<[u8; 256], 8> = StaticObjectPool::new();
    /// let pool: &'static ObjectPool<[u8; 256], 8> = POOL.init_pool([0u8; 256]);
    /// ```
    pub fn init_pool(&'static self, init: T) -> &'static ObjectPool<T, N, B> {
        assert!(N > 0, "Pool capacity must be greater than 0");
        unsafe {
            let ptr: *mut ObjectPool<T, N, B> = self.data.get().cast::<ObjectPool<T, N, B>>();
            ptr.write(ObjectPool {
                slots: UnsafeCell::new(MaybeUninit::uninit()),
                free: B::new(),
            });
            (*ptr).free.fill(N).expect("fill(N) must succeed when count == capacity");
            for i in 0..N {
                (*ptr).slot_mut_ptr(i).write(init.clone());
            }
            &*ptr
        }
    }
}

impl<T, const N: usize, B: FreeSlots<N>> ObjectPool<T, N, B> {
    /// Borrow a slot. The previous contents are preserved — no value is written
    /// and **no destructor runs** when the returned [`ObjectGuard`] is dropped.
    ///
    /// Returns `None` when all slots are in use.
    pub fn request(&'static self) -> Option<ObjectGuard<T, N, B>> {
        self.free
            .take()
            .map(|index| ObjectGuard { pool: self, index })
    }

    /// Return a slot to the free set **without** dropping its contents.
    pub(crate) fn release(&self, idx: usize) {
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

/// A borrow of an [`ObjectPool`] slot.  Derefs to `&mut T`; returns the slot
/// to the pool on drop **without** running `T`'s destructor — the data persists
/// for the next `request()`.
#[must_use = "discarding an ObjectGuard immediately frees the slot — likely a bug"]
pub struct ObjectGuard<T: 'static, const N: usize, B: FreeSlots<N> + 'static = CriticalSlots<N>> {
    pub(crate) pool: &'static ObjectPool<T, N, B>,
    pub(crate) index: usize,
}

impl<T, const N: usize, B: FreeSlots<N>> Deref for ObjectGuard<T, N, B> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.pool.slot_ptr(self.index) }
    }
}

impl<T, const N: usize, B: FreeSlots<N>> DerefMut for ObjectGuard<T, N, B> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.pool.slot_mut_ptr(self.index) }
    }
}

impl<T, const N: usize, B: FreeSlots<N>> Drop for ObjectGuard<T, N, B> {
    fn drop(&mut self) {
        // No drop_in_place — data persists across borrow cycles.
        self.pool.release(self.index);
    }
}

// SAFETY: ObjectGuard holds exclusive access to its slot.
unsafe impl<T: Send, const N: usize, B: FreeSlots<N>> Send for ObjectGuard<T, N, B> {}
