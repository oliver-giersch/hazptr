use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Barrier,
};
use std::thread;

use hazptr::reclaim::Protect;
use hazptr::typenum::U0;
use hazptr::{guarded, Owned};

type Atomic<T> = hazptr::Atomic<T, U0>;
type Unlinked<T> = hazptr::Unlinked<T, U0>;

struct DropCount(Arc<AtomicUsize>);
impl Drop for DropCount {
    #[inline]
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn abandon_on_panic() {
    let drop_count = Arc::new(AtomicUsize::new(0));

    let records = Arc::new([
        Atomic::new(DropCount(Arc::clone(&drop_count))),
        Atomic::new(DropCount(Arc::clone(&drop_count))),
        Atomic::new(DropCount(Arc::clone(&drop_count))),
    ]);

    let barrier1 = Arc::new(Barrier::new(2));
    let barrier2 = Arc::new(Barrier::new(2));

    let t1 = {
        let records = Arc::clone(&records);
        let barrier1 = Arc::clone(&barrier1);
        let barrier2 = Arc::clone(&barrier2);
        thread::spawn(move || {
            let mut guard1 = guarded();
            let mut guard2 = guarded();

            let r1 = records[0].load(Ordering::Relaxed, &mut guard1);
            let r2 = records[1].load(Ordering::Relaxed, &mut guard2);

            barrier1.wait();
            barrier2.wait();

            assert!(r1.is_some() && r2.is_some(), "references must still be valid");
        })
    };

    let t2 = {
        let records = Arc::clone(&records);
        let barrier = Arc::clone(&barrier1);
        thread::spawn(move || {
            barrier.wait();
            unsafe {
                Unlinked::retire(records[0].swap(Owned::none(), Ordering::Relaxed).unwrap());
                Unlinked::retire(records[1].swap(Owned::none(), Ordering::Relaxed).unwrap());
                Unlinked::retire(records[2].swap(Owned::none(), Ordering::Relaxed).unwrap());
            }

            panic!("explicit panic: thread 2 abandons all retired records it can't reclaim");
        })
    };

    t2.join().unwrap_err();

    // thread 1 still holds two protected references, so only one record must have been reclaimed
    // when the thread panicked
    assert_eq!(drop_count.load(Ordering::Relaxed), 1);

    barrier2.wait();

    t1.join().unwrap();

    // "count-release" and "maximum--reclamation-freq" ensures that thread 1 initiates two GC scans
    // when r1 and r2 go out of scope, the first of which adopts the retired records abandoned by
    // thread 2 and reclaims them
    assert_eq!(drop_count.load(Ordering::Relaxed), 3);
}

#[test]
fn release_on_panic() {}
