// this is mainly useful for checking the assembly output

use std::sync::atomic::Ordering::{Acquire, Release};

use hazptr::{Guard, Owned};

type Atomic<T> = hazptr::Atomic<T, hazptr::typenum::U0>;

static GLOBAL: Atomic<i32> = Atomic::null();

fn main() {
    init();
    let mut guard = Guard::new();
    let _global = GLOBAL.load(Acquire, &mut guard).unwrap();
}

#[inline(never)]
fn init() {
    GLOBAL.store(Owned::new(1), Release);
}
