//! Fixed-capacity object pools for `no_std` / embedded systems.
//!
//! Two pool variants, sharing the same pluggable-backend architecture:
//!
//! | Variant | Allocate | Drop | Use case |
//! |---|---|---|---|
//! | [`BoxPool`] | `alloc(value)` — writes a new value | runs `T`'s destructor | owned objects |
//! | [`ObjectPool`] | `request()` — no value, previous data persists | no destructor | buffers, messages |
//!
//! - Static memory, no heap allocation.
//! - Safe sharing between ISRs and async tasks.
//! - Pluggable backends via generic `B`; default is [`CriticalSlots`](critical::CriticalSlots).
//!   Enable `feature = "cas"` for [`CasSlots`](cas::CasSlots).
//!
//! ## BoxPool — owned values
//!
//! ```ignore
//! use slotpool::{BoxPool, StaticBoxPool};
//!
//! static POOL: StaticBoxPool<[u8; 256], 8> = StaticBoxPool::new();
//! let pool: &'static BoxPool<[u8; 256], 8> = POOL.init_pool();
//!
//! let guard = pool.alloc([0u8; 256]).unwrap();
//! // guard is returned to the pool when dropped; destructor runs
//! ```
//!
//! ## ObjectPool — persistent buffers / messages
//!
//! ```ignore
//! use slotpool::{ObjectPool, StaticObjectPool};
//!
//! static POOL: StaticObjectPool<[u8; 256], 8> = StaticObjectPool::new();
//! let pool: &'static ObjectPool<[u8; 256], 8> = POOL.init_pool([0u8; 256]);
//!
//! let mut buf = pool.request().unwrap();
//! buf[0..4].copy_from_slice(&header);
//! // buf is returned to the pool when dropped; no destructor — data persists
//! ```
//!
//! ## Backend selection
//!
//! ```ignore
//! use slotpool::critical::CriticalSlots;
//! #[cfg(feature = "cas")]
//! use slotpool::cas::CasSlots;
//!
//! type MyPool    = StaticBoxPool<[u8; 256], 8>;                    // default: CriticalSlots
//! type MyCasPool = StaticBoxPool<[u8; 256], 8, CasSlots<8>>;      // CAS backend
//! ```
//!
//! The free-index chain is stored separately from `T`'s slots — this is **not**
//! an intrusive free list, so slot memory layout is unaffected.

#![cfg_attr(not(test), no_std)]

mod free_slots;

#[cfg(feature = "cas")]
pub mod cas;
pub mod critical;

mod boxed;
mod object;

pub use boxed::{BoxGuard, BoxPool, StaticBoxPool};
pub use object::{ObjectGuard, ObjectPool, StaticObjectPool};
