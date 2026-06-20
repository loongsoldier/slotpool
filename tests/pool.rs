use slotpool::{BoxGuard, BoxPool, ObjectPool, StaticBoxPool, StaticObjectPool};

// ── BoxPool helpers ──────────────────────────────────────────────

fn init_boxpool() -> &'static BoxPool<u32, 4> {
    Box::leak(Box::new(StaticBoxPool::new())).init_pool()
}

fn init_small_boxpool() -> &'static BoxPool<u32, 2> {
    Box::leak(Box::new(StaticBoxPool::new())).init_pool()
}

// ── BoxPool tests ────────────────────────────────────────────────

#[test]
fn box_alloc_and_drop() {
    let pool = init_boxpool();
    let guard = pool.alloc(42).unwrap();
    assert_eq!(*guard, 42);
    drop(guard);
    let guard2 = pool.alloc(99).unwrap();
    assert_eq!(*guard2, 99);
}

#[test]
fn box_pool_full_returns_err() {
    let pool = init_small_boxpool();
    let _a = pool.alloc(1).unwrap();
    let _b = pool.alloc(2).unwrap();
    let result = pool.alloc(3);
    assert!(result.is_err());
    match result {
        Err(v) => assert_eq!(v, 3),
        _ => unreachable!(),
    }
}

#[test]
fn box_reuse_after_drop() {
    let pool = init_small_boxpool();
    let a = pool.alloc(10).unwrap();
    let b = pool.alloc(20).unwrap();
    drop(a);
    let c = pool.alloc(30).unwrap();
    assert_eq!(*c, 30);
    drop(b);
    drop(c);
}

#[test]
fn box_deref_mut_modifies_value() {
    let pool = init_boxpool();
    let mut guard = pool.alloc(0).unwrap();
    *guard = 100;
    assert_eq!(*guard, 100);
}

#[test]
fn box_try_clone_success() {
    let pool = init_boxpool();
    let orig = pool.alloc(7).unwrap();
    let cloned = orig.try_clone().unwrap();
    assert_eq!(*orig, 7);
    assert_eq!(*cloned, 7);
    drop(orig);
    assert_eq!(*cloned, 7);
    drop(cloned);
}

#[test]
fn box_try_clone_pool_full() {
    let pool = init_small_boxpool();
    let a = pool.alloc(1).unwrap();
    let _b = pool.alloc(2).unwrap();
    let result = a.try_clone();
    assert!(result.is_err());
    match result {
        Err(v) => assert_eq!(v, 1),
        _ => unreachable!(),
    }
    drop(a);
}

#[test]
fn box_into_raw_from_raw_roundtrip() {
    let pool = init_boxpool();
    let guard = pool.alloc(42).unwrap();
    let ptr = BoxGuard::into_raw(guard);

    unsafe {
        assert_eq!(*ptr, 42);
        *ptr = 99;
    }

    let guard = unsafe { BoxGuard::from_raw(pool, ptr) };
    assert_eq!(*guard, 99);
    drop(guard);

    let next = pool.alloc(77).unwrap();
    assert_eq!(*next, 77);
}

#[test]
#[should_panic(expected = "ZST")]
fn box_into_raw_panics_on_zst() {
    let pool: &'static BoxPool<(), 4> = Box::leak(Box::new(StaticBoxPool::new())).init_pool();
    let guard = pool.alloc(()).unwrap();
    let _ptr = BoxGuard::into_raw(guard);
}

#[test]
fn box_alloc_array_type() {
    let pool: &'static BoxPool<[u8; 4], 4> = Box::leak(Box::new(StaticBoxPool::new())).init_pool();

    let guard = pool.alloc([1, 2, 3, 4]).unwrap();
    assert_eq!(*guard, [1, 2, 3, 4]);
}

// ── ObjectPool helpers ───────────────────────────────────────────

fn init_objpool() -> &'static ObjectPool<u32, 4> {
    Box::leak(Box::new(StaticObjectPool::new())).init_pool(0)
}

fn init_small_objpool() -> &'static ObjectPool<u32, 2> {
    Box::leak(Box::new(StaticObjectPool::new())).init_pool(0)
}

// ── ObjectPool tests ─────────────────────────────────────────────

#[test]
fn obj_request_and_drop() {
    let pool = init_objpool();
    let obj = pool.request().unwrap();
    assert_eq!(*obj, 0); // init value
    drop(obj);
    let obj2 = pool.request().unwrap();
    assert_eq!(*obj2, 0); // value persists
}

#[test]
fn obj_data_persists_across_borrows() {
    let pool = init_small_objpool();
    {
        let mut obj = pool.request().unwrap();
        *obj = 42;
    }
    // After drop, data should still be 42
    let obj = pool.request().unwrap();
    assert_eq!(*obj, 42);
}

#[test]
fn obj_pool_exhausted() {
    let pool = init_small_objpool();
    let _a = pool.request().unwrap();
    let _b = pool.request().unwrap();
    assert!(pool.request().is_none());
}

#[test]
fn obj_reuse_after_drop() {
    let pool = init_small_objpool();
    let a = pool.request().unwrap();
    let b = pool.request().unwrap();
    drop(a);
    let c = pool.request().unwrap();
    assert_eq!(*c, 0);
    drop(b);
    drop(c);
}

// ── CAS backend ──────────────────────────────────────────────────

#[cfg(feature = "cas")]
mod cas_tests {
    use slotpool::cas::CasSlots;
    use slotpool::{StaticBoxPool, StaticObjectPool};

    #[test]
    fn cas_box_alloc_and_drop() {
        type P = StaticBoxPool<u32, 4, CasSlots<4>>;
        let pool = Box::leak(Box::new(P::new())).init_pool();

        let guard = pool.alloc(42).unwrap();
        assert_eq!(*guard, 42);
        drop(guard);

        let guard2 = pool.alloc(99).unwrap();
        assert_eq!(*guard2, 99);
    }

    #[test]
    fn cas_box_pool_full() {
        type P = StaticBoxPool<u32, 2, CasSlots<2>>;
        let pool = Box::leak(Box::new(P::new())).init_pool();

        let _a = pool.alloc(1).unwrap();
        let _b = pool.alloc(2).unwrap();
        assert!(pool.alloc(3).is_err());
    }

    #[test]
    fn cas_obj_data_persists() {
        type P = StaticObjectPool<u32, 2, CasSlots<2>>;
        let pool = Box::leak(Box::new(P::new())).init_pool(0);

        {
            let mut obj = pool.request().unwrap();
            *obj = 7;
        }
        let obj = pool.request().unwrap();
        assert_eq!(*obj, 7);
    }
}
