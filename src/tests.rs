use std::mem;
use std::sync::{atomic::Ordering, Arc, Barrier};
use std::thread;

use reclaim::{typenum::U0, MarkedPointer, MarkedPtr, Protect};

use super::{guarded, Atomic, Owned};

#[test]
fn empty_guarded() {
    let guarded = guarded::<i32, U0>();
    assert!(guarded.shared().is_none());
}

#[test]
fn acquire_null() {
    let null: Atomic<i32, U0> = Atomic::null();
    let atomic: Atomic<i32, U0> = Atomic::new(1);

    let mut guarded = guarded();

    assert!(null.load(Ordering::Relaxed, &mut guarded).is_none());
    assert!(guarded.shared().is_none());

    assert!(atomic.load(Ordering::Relaxed, &mut guarded).is_some());
    assert!(guarded.shared().is_some());
    guarded.release();

    assert!(guarded.shared().is_none());
    assert_eq!(crate::default::cached_hazards_count(), 0);
    mem::drop(guarded);
    assert_eq!(crate::default::cached_hazards_count(), 1);
}

#[test]
fn acquire_load() {
    let atomic: Atomic<i32, U0> = Atomic::new(1);
    let mut guarded = guarded();

    let shared = atomic.load(Ordering::Relaxed, &mut guarded).unwrap();
    assert_eq!(&1, unsafe { shared.deref() });
    let shared = guarded.shared().map(|shared| unsafe { shared.deref() });
    assert_eq!(Some(&1), shared);
}

#[test]
fn acquire_if_equal() {
    let atomic: Atomic<i32, U0> = Atomic::null();
    let mut guarded = guarded();

    let res = guarded.acquire_if_equal(&atomic, MarkedPtr::null(), Ordering::Relaxed);
    assert_eq!(Ok(None), res);
    assert!(guarded.shared().is_none());

    let owned = Owned::new(1);
    let marked = owned.as_marked();
    atomic.store(owned, Ordering::Relaxed);
    let res = guarded.acquire_if_equal(&atomic, MarkedPtr::null(), Ordering::Relaxed);
    assert!(res.is_err());
    assert!(guarded.shared().is_none());

    let res = guarded.acquire_if_equal(&atomic, marked, Ordering::Relaxed);
    assert!(res.is_ok());
    assert_eq!(guarded.shared().unwrap().as_marked(), marked);
}

#[test]
#[cfg_attr(feature = "count-release", ignore)]
fn abandon_on_panic() {
    static RECORD1: Atomic<i32, U0> = Atomic::null();
    static RECORD2: Atomic<i32, U0> = Atomic::null();

    RECORD1.store(Owned::new(1), Ordering::Relaxed);
    RECORD2.store(Owned::new(1), Ordering::Relaxed);

    let barrier1 = Arc::new(Barrier::new(2));
    let barrier2 = Arc::new(Barrier::new(2));

    let thread1 = {
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

    let thread2 = thread::spawn(move || {
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

        panic!("explicit panic: release all acquired hazards and abandon retired records")
    });

    // thread 2 has panicked and abandoned two retired records
    thread2.join().unwrap_err();
    // adopt records before thread 1 exits and adopts them
    let abandoned = crate::default::GLOBAL
        .try_adopt_abandoned_records()
        .unwrap();
    barrier2.wait();
    thread1.join().unwrap();

    assert_eq!(abandoned.inner.len(), 2);
    // the records can be safely dropped since thread 1 is already gone
}
