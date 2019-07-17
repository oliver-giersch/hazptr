#![feature(test)]

extern crate test;

use std::sync::atomic::Ordering::Relaxed;

use test::Bencher;

use hazptr::{Config, Guard, CONFIG};

type Atomic<T> = hazptr::Atomic<T, hazptr::typenum::U0>;

#[bench]
fn pin_and_load(b: &mut Bencher) {
    CONFIG.init_once(|| Config::with_params(128));

    let atomic = Atomic::new(1);

    b.iter(|| {
        let guard = &mut Guard::new();
        assert_eq!(*atomic.load(Relaxed, guard).unwrap(), 1);
    })
}
