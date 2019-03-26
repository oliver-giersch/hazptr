use std::sync::{atomic::Ordering, Arc, Barrier};
use std::thread;

use reclaim::U0;

use super::*;

#[test]
fn empty_guarded() {
    let guard: Guarded<i32, U0> = Guarded::new();
    assert!(guard.hazard.is_none());
    assert!(guard.shared().is_none());
}

#[test]
fn acquire_null() {
    let null: Atomic<i32, U0> = Atomic::null();
    let atomic: Atomic<i32, U0> = Atomic::new(1);

    let mut guard = Guarded::new();

    assert!(null.load(Ordering::Relaxed, &mut guard).is_none());
    assert!(guard.shared().is_none());
    // no hazard must be acquired when acquiring a null pointer
    assert_eq!(
        local::cached_hazards_count(),
        0,
        "acquisition of a null pointer must not acquire a hazard"
    );

    assert!(atomic.load(Ordering::Relaxed, &mut guard).is_some());
    assert!(guard.shared().is_some());
    guard.release();
    assert!(guard.shared().is_none());
    assert_eq!(local::cached_hazards_count(), 1);
}

#[test]
fn acquire_load() {
    let atomic: Atomic<i32, U0> = Atomic::new(1);
    let mut guard = Guarded::new();

    let reference = atomic.load(Ordering::Relaxed, &mut guard).unwrap();
    assert_eq!(&1, unsafe { reference.deref() });
    let reference = guard.shared().map(|shared| unsafe { shared.deref() });
    assert_eq!(Some(&1), reference);
    assert!(guard.hazard.is_some());
}

#[test]
fn acquire_direct() {
    let atomic: Atomic<i32, U0> = Atomic::new(1);
    let mut guard = Guarded::new();
    guard.acquire(&atomic, Ordering::Relaxed);

    let reference = atomic.load(Ordering::Relaxed, &mut guard).unwrap();
    assert_eq!(&1, unsafe { reference.deref() });
    let reference = guard.shared().map(|shared| unsafe { shared.deref() });
    assert_eq!(Some(&1), reference);
    assert!(guard.hazard.is_some());
}

#[test]
#[cfg_attr(feature = "count-release", ignore)]
fn abandon_on_panic() {
    static RECORD1: Atomic<i32, U0> = Atomic::null();
    static RECORD2: Atomic<i32, U0> = Atomic::null();

    RECORD1.store(Owned::new(1), Ordering::Relaxed);
    RECORD2.store(Owned::new(2), Ordering::Relaxed);

    let barrier1 = Arc::new(Barrier::new(2));
    let barrier2 = Arc::new(Barrier::new(2));

    let h1 = {
        let barrier1 = Arc::clone(&barrier1);
        let barrier2 = Arc::clone(&barrier2);
        thread::spawn(move || {
            let mut guard1 = guarded();
            let mut guard2 = guarded();

            RECORD1.load(Ordering::Relaxed, &mut guard1);
            RECORD2.load(Ordering::Relaxed, &mut guard2);

            barrier1.wait();
            barrier2.wait()
        })
    };

    let h2 = thread::spawn(move || {
        barrier1.wait();
        unsafe {
            RECORD1
                .swap(Owned::none(), Ordering::Relaxed)
                .unwrap()
                .retire();
            RECORD2
                .swap(Owned::none(), Ordering::Relaxed)
                .unwrap()
                .retire();
        }

        panic!("on panic: release all acquired hazards and abandon retired records")
    });

    // thread 2 has panicked and abandoned two retired records
    h2.join().unwrap_err();
    // adopt records before thread 1 exits and adopts them
    let abandoned = global::try_adopt_abandoned_records().unwrap();
    barrier2.wait();
    h1.join().unwrap();

    assert_eq!(abandoned.inner.len(), 2);
    // the records can be safely dropped since thread 1 is already gone
}
