#![feature(test)]

extern crate test;

use std::sync::atomic::Ordering::Relaxed;

use test::Bencher;

use hazptr::{Config, CONFIG};

type Atomic<T> = hazptr::Atomic<T, hazptr::typenum::U0>;
type Owned<T> = hazptr::Owned<T, hazptr::typenum::U0>;

#[bench]
fn retire(b: &mut Bencher) {
    CONFIG.init_once(|| Config::with_params(128));

    let global = Atomic::new(1);

    b.iter(|| {
        let unlinked = global.swap(Owned::new(1), Relaxed).unwrap();
        unsafe { unlinked.retire() };
    });
}
