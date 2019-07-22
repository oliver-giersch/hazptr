#![feature(test)]

extern crate test;

use std::sync::atomic::Ordering::{AcqRel, Relaxed};
use std::thread;

use test::Bencher;

use conquer_once::Lazy;
use hazptr::{ConfigBuilder, CONFIG};

type Atomic<T> = hazptr::Atomic<T, hazptr::typenum::U0>;
type Owned<T> = hazptr::Owned<T, hazptr::typenum::U0>;

#[bench]
fn single_retire(b: &mut Bencher) {
    CONFIG.init_once(|| ConfigBuilder::new().scan_threshold(128).build());

    let global = Atomic::new(1);

    b.iter(|| {
        let unlinked = global.swap(Owned::new(1), Relaxed).unwrap();
        unsafe { unlinked.retire() };
    });
}

#[bench]
fn multi_retire(b: &mut Bencher) {
    const STEPS: u32 = 100_000;
    CONFIG.init_once(|| ConfigBuilder::new().scan_threshold(128).build());

    let global = Atomic::new(1);

    b.iter(|| {
        for _ in 0..STEPS {
            let unlinked = global.swap(Owned::new(1), Relaxed).unwrap();
            unsafe { unlinked.retire() };
        }
    });
}

#[bench]
fn multi_retire_varied(b: &mut Bencher) {
    const STEPS: u32 = 100_000;
    CONFIG.init_once(|| ConfigBuilder::new().scan_threshold(128).build());

    let int = Atomic::new(1);
    let string = Atomic::new(String::from("string"));
    let arr = Atomic::new([0usize; 16]);

    b.iter(|| unsafe {
        for _ in 0..STEPS {
            int.swap(Owned::new(1), Relaxed).unwrap().retire();
            string.swap(Owned::new(String::from("string")), Relaxed).unwrap().retire();
            arr.swap(Owned::new([0usize; 16]), Relaxed).unwrap().retire();
        }
    });
}

#[bench]
fn parallel_retire(b: &mut Bencher) {
    const THREADS: u32 = 8;
    const STEPS: u32 = 100_000;

    static GLOBAL: Lazy<Atomic<u32>> = Lazy::new(|| Atomic::new(0));

    let handles: Vec<_> = (0..THREADS)
        .map(|id| {
            thread::spawn(|| {
                for _ in 0..STEPS {
                    let unlinked = GLOBAL.swap(Owned::new(id), AcqRel).unwrap();
                    unsafe { unlinked.retire() };
                }
            })
        })
        .collect();
}
