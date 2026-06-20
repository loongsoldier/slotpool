# slotpool

[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)

Fixed-capacity object pools for `no_std` / embedded Rust — static memory, no heap allocation.

## Two pool variants

| Variant | Allocate | Drop behaviour | Use case |
|---|---|---|---|
| `BoxPool` | `alloc(value)` — writes a value | runs `T`'s destructor | owned objects |
| `ObjectPool` | `request()` — no copy, data persists | no destructor | buffers, messages |

Both share the same pluggable-backend architecture (`CriticalSlots` by default, `CasSlots` behind feature `cas`).

## Quick start

### BoxPool — owned values

```rust
use slotpool::{BoxPool, StaticBoxPool};

static POOL: StaticBoxPool<[u8; 256], 8> = StaticBoxPool::new();
let pool: &'static BoxPool<[u8; 256], 8> = POOL.init_pool();

let guard = pool.alloc([0u8; 256]).unwrap();
// guard is returned when dropped — destructor runs
```

### ObjectPool — persistent buffers

```rust
use slotpool::{ObjectPool, StaticObjectPool};

static POOL: StaticObjectPool<[u8; 256], 8> = StaticObjectPool::new();
let pool: &'static ObjectPool<[u8; 256], 8> = POOL.init_pool([0u8; 256]);

let mut buf = pool.request().unwrap();
buf[0..4].copy_from_slice(&header);
// buf is returned when dropped — no destructor, data persists for next request()
```

## Backends

```rust
use slotpool::critical::CriticalSlots;     // default — critical-section based
#[cfg(feature = "cas")]
use slotpool::cas::CasSlots;              // lock-free CAS (Treiber stack)

type MyPool    = StaticBoxPool<[u8; 256], 8>;                    // CriticalSlots
type MyCasPool = StaticBoxPool<[u8; 256], 8, CasSlots<8>>;      // CasSlots
```

| Backend | Mechanism | Portability |
|---|---|---|
| `CriticalSlots` | `critical_section::Mutex` + `heapless::Vec` | any target with `critical-section` |
| `CasSlots` | lock-free Treiber stack (atomic CAS) | Cortex-M3+, RISC-V, x86 |

## Features

- **`cas`** — enables the `CasSlots` lock-free backend (requires hardware CAS instructions).
- **`async`** — enables `AsyncBoxPool` / `AsyncObjectPool` with `.await` support via `embassy-sync`.

## Design

- Free-index chain is **separate** from `T`'s slot memory — not an intrusive free list, so slot layout is unaffected.
- Pools must live in `static` memory; `alloc` / `request` take `&'static self`, eliminating lifetime parameters on guards — convenient for async frameworks like Embassy.
- `BoxPool::alloc` / `ObjectPool::request` block and await when the pool is full (feature `async`).
- `BoxGuard` supports `try_clone` (fallible, no panic on exhaustion) and `into_raw` / `from_raw` for DMA / FFI.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
