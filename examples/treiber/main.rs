use std::mem;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::thread;

mod stack;

use crate::stack::TreiberStack;

#[repr(align(64))]
struct ThreadCount(AtomicUsize);

struct DropCount<'a>(&'a AtomicUsize);
impl Drop for DropCount<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

fn main() {
    const THREADS: usize = 8;
    const PER_THREAD_ALLOCATIONS: usize = 1_000_000 + 1_000;
    static COUNTERS: [ThreadCount; THREADS] = [
        ThreadCount(AtomicUsize::new(0)),
        ThreadCount(AtomicUsize::new(0)),
        ThreadCount(AtomicUsize::new(0)),
        ThreadCount(AtomicUsize::new(0)),
        ThreadCount(AtomicUsize::new(0)),
        ThreadCount(AtomicUsize::new(0)),
        ThreadCount(AtomicUsize::new(0)),
        ThreadCount(AtomicUsize::new(0)),
    ];

    let stack = Arc::new(TreiberStack::new());
    let handles: Vec<_> = (0..THREADS)
        .map(|id| {
            let stack = Arc::clone(&stack);
            thread::spawn(move || {
                let counter = &COUNTERS[id].0;

                for _ in 0..1_000 {
                    stack.push(DropCount(counter));
                }

                for _ in 0..1_000_000 {
                    let _res = stack.pop();
                    stack.push(DropCount(counter));
                }

                println!(
                    "thread {} has deallocated {:7}/{} records before exiting",
                    id,
                    counter.load(Ordering::Relaxed),
                    PER_THREAD_ALLOCATIONS
                );
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    mem::drop(stack);
    let drop_sum = COUNTERS
        .iter()
        .map(|local| local.0.load(Ordering::Relaxed))
        .sum();

    assert_eq!(THREADS * PER_THREAD_ALLOCATIONS, drop_sum);
    println!("total dropped records: {}, no memory was leaked", drop_sum);
}
