use std::mem::{self, ManuallyDrop};
use std::ptr;
use std::sync::atomic::Ordering;

use hazptr::guarded;

type Atomic<T> = hazptr::Atomic<T, reclaim::U0>;
type Owned<T> = hazptr::Owned<T, reclaim::U0>;

#[derive(Default)]
pub struct TreiberStack<T> {
    head: Atomic<Node<T>>,
}

impl<T: 'static> TreiberStack<T> {
    #[inline]
    pub fn new() -> Self {
        Self {
            head: Atomic::null(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.head.load_unprotected(Ordering::Relaxed).is_some()
    }

    #[inline]
    pub fn push(&self, elem: T) {
        let mut node = Owned::new(Node::new(elem));
        let mut guard = guarded();

        loop {
            let head = self.head.load(Ordering::Relaxed, &mut guard);
            node.next.store(head, Ordering::Relaxed);
            match self
                .head
                .compare_exchange_weak(head, node, Ordering::Release, Ordering::Relaxed)
            {
                Ok(_) => return,
                Err(fail) => node = fail.input,
            }
        }
    }

    #[inline]
    pub fn pop(&self) -> Option<T> {
        let mut guard = guarded();

        while let Some(head) = self.head.load(Ordering::Acquire, &mut guard) {
            let next = unsafe { head.deref() }
                .next
                .load_unprotected(Ordering::Relaxed);

            if let Ok(unlinked) =
                self.head
                    .compare_exchange_weak(head, next, Ordering::Release, Ordering::Relaxed)
            {
                unsafe {
                    let res = ptr::read(&*unlinked.deref().elem);
                    unlinked.reclaim();

                    return Some(res);
                }
            }
        }

        None
    }
}

impl<T> Drop for TreiberStack<T> {
    fn drop(&mut self) {
        let mut curr = self.head.take();

        // it's necessary to manually drop all elements iteratively
        while let Some(mut node) = curr {
            unsafe { ManuallyDrop::drop(&mut node.elem) }
            curr = node.next.take();
            mem::drop(node);
        }
    }
}

struct Node<T> {
    elem: ManuallyDrop<T>,
    next: Atomic<Node<T>>,
}

impl<T> Node<T> {
    fn new(elem: T) -> Self {
        Self {
            elem: ManuallyDrop::new(elem),
            next: Atomic::null(),
        }
    }
}

fn main() {
    use std::sync::Arc;
    use std::thread;

    const THREADS: usize = 8;

    let stack = Arc::new(TreiberStack::new());

    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let stack = Arc::clone(&stack);
            thread::spawn(move || {
                for i in 0..100 {
                    stack.push(i);
                }

                for i in 0..1_000_000 {
                    let res = stack.pop();
                    stack.push(i);

                    match res {
                        Some(_) => {
                            stack.pop();
                        }
                        None => {
                            stack.push(i);
                            stack.push(i);
                        }
                    };
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let mut vec = Vec::new();
    while let Some(i) = stack.pop() {
        vec.push(i);
    }

    println!("{:?}", vec);
}
