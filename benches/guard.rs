#![feature(test)]

extern crate test;

use std::sync::atomic::Ordering;

use test::Bencher;

use hazptr::conquer_reclaim::LocalState;
use hazptr::typenum::U0;
use hazptr::{build_guard, ConfigBuilder, Hp, LocalRef, CONFIG};

type Atomic<T, R> = hazptr::conquer_reclaim::Atomic<T, R, U0>;

#[bench]
fn guard_global(b: &mut Bencher) {
    CONFIG.write().unwrap().ops_count_threshold = 128;
    let atomic = Atomic::new(0);

    b.iter(|| {
        let guard = &mut build_guard();
        let loaded = atomic.load(guard, Ordering::Relaxed);
        assert_eq!(unsafe { loaded.as_ref() }, Some(&0));
    })
}

#[bench]
fn guard_local(b: &mut Bencher) {
    let hp = Hp::local_retire(ConfigBuilder::new().set_ops_count_threshold(128).build());
    let local = hp.build_local(None);
    let local_ref = LocalRef::from_ref(&local);

    let atomic = Atomic::new(0);

    b.iter(|| {
        let guard = &mut local_ref.build_guard();
        let loaded = atomic.load(guard, Ordering::Relaxed);
        assert_eq!(unsafe { loaded.as_ref() }, Some(&0));
    })
}
