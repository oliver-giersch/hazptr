use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time;

use conquer_reclaim::examples::treiber::ArcStack;
use conquer_reclaim::Reclaim;
use hazptr::{GlobalHp, GlobalRetire, Hp};

const PRE_FILL_OPS: usize = 1_000;
const PUSH_OPS: usize = 1_000_000;
const THREADS: usize = 4;

static COUNTERS: [ThreadCount; THREADS] = [
    ThreadCount(AtomicUsize::new(0)),
    ThreadCount(AtomicUsize::new(0)),
    ThreadCount(AtomicUsize::new(0)),
    ThreadCount(AtomicUsize::new(0)),
    // ThreadCount(AtomicUsize::new(0)),
    // ThreadCount(AtomicUsize::new(0)),
    // ThreadCount(AtomicUsize::new(0)),
    // ThreadCount(AtomicUsize::new(0)),
];

fn main() {
    println!("example: Treiber's lock-free stack with hazard pointers");

    /*let stack: ArcStack<_, GlobalHp> = ArcStack::new();
    let now = time::Instant::now();
    run_example(stack);
    println!("time with global reclaimer: {} ms", now.elapsed().as_millis());

    for ThreadCount(counter) in &COUNTERS {
        counter.store(0, Ordering::Relaxed);
    }

    let stack: ArcStack<_, Hp> = ArcStack::new();
    let now = time::Instant::now();
    run_example(stack);
    println!("time with local reclaimer: {} ms", now.elapsed().as_millis());

    for ThreadCount(counter) in &COUNTERS {
        counter.store(0, Ordering::Relaxed);
    }*/

    let stack: ArcStack<_, Hp<GlobalRetire>> = ArcStack::new();
    let now = time::Instant::now();
    run_example(stack);
    println!(
        "time with local reclaimer and global retire strategy: {} ms",
        now.elapsed().as_millis()
    );
}

#[inline]
fn run_example<R: Reclaim>(stack: ArcStack<DropCount<'static>, R>) {
    let handles: Vec<_> = (0..THREADS)
        .map(|id| {
            let stack = ArcStack::clone(&stack);
            thread::spawn(move || {
                let ThreadCount(counter) = &COUNTERS[id];
                for _ in 0..PRE_FILL_OPS {
                    stack.push(DropCount(counter));
                }

                for _ in 0..PUSH_OPS {
                    let _ = stack.pop();
                    stack.push(DropCount(counter));
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    println!("all joined");

    mem::drop(stack);
    let drop_sum = COUNTERS.iter().map(|ThreadCount(count)| count.load(Ordering::Relaxed)).sum();

    assert_eq!(THREADS * (PRE_FILL_OPS + PUSH_OPS), drop_sum);
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// ThreadCount
////////////////////////////////////////////////////////////////////////////////////////////////////

#[repr(align(64))]
struct ThreadCount(AtomicUsize);

////////////////////////////////////////////////////////////////////////////////////////////////////
// DropCount
////////////////////////////////////////////////////////////////////////////////////////////////////

struct DropCount<'a>(&'a AtomicUsize);
impl Drop for DropCount<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}
