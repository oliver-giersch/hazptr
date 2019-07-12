// this is mainly useful for checking the assembly output

use std::sync::atomic::Ordering::Acquire;

use hazptr::Guard;

type Atomic<T> = hazptr::Atomic<T, hazptr::typenum::U0>;

static GLOBAL: Atomic<i32> = Atomic::null();

fn main() {
    let mut guard = Guard::new();
    let _global = GLOBAL.load(Acquire, &mut guard);
}
