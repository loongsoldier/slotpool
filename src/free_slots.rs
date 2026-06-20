//! Abstract interface for managing free slot indices.
//! `Pool` depends only on this trait, not on a particular synchronisation
//! mechanism.
//!
//! Two backends are provided: [`CriticalSlots`](crate::critical::CriticalSlots) (default)
//! and [`CasSlots`](crate::cas::CasSlots) (behind feature `cas`):
//!
//! ```ignore
//! use slotpool::Pool;
//! use slotpool::critical::CriticalSlots;
//! #[cfg(feature = "cas")]
//! use slotpool::cas::CasSlots;
//!
//! type MyPool    = Pool<u8, 16>;                      // default: CriticalSlots
//! type MyCasPool = Pool<u8, 16, CasSlots<16>>;        // explicit CAS backend
//! ```

pub trait FreeSlots<const N: usize>: Sync {
    /// Create an empty collection (no free indices).
    fn new() -> Self;

    /// Fill with indices `0..count`.  `count` must be `<= N`.
    /// Must only be called once, during initialisation.
    fn fill(&self, count: usize);

    /// Take a free index; returns `None` when exhausted.
    /// Safe to call concurrently from any execution context.
    fn take(&self) -> Option<usize>;

    /// Return an index to the free set.
    /// Safe to call concurrently from any execution context.
    fn put(&self, idx: usize);
}
