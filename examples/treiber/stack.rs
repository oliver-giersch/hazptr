use std::mem::ManuallyDrop;
use std::ptr;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};

use hazptr::typenum::U0;
use hazptr::Guard;

type Atomic<T> = hazptr::Atomic<T, U0>;
type Owned<T> = hazptr::Owned<T, U0>;

#[derive(Default)]
pub struct Stack<T> {
    head: Atomic<Node<T>>,
}

impl<T> Stack<T> {
    #[inline]
    pub const fn new() -> Self {
        Self { head: Atomic::null() }
    }

    #[inline]
    pub fn push(&self, elem: T) {
        let mut node = Owned::new(Node::new(elem));
        let mut guard = Guard::new();

        loop {
            let head = self.head.load(Relaxed, &mut guard);
            node.next.store(head, Relaxed);

            // (TRE:1) this `Release` CAS synchronizes-with the `Acquire` load in (TRE:2)
            match self.head.compare_exchange_weak(head, node, Release, Relaxed) {
                Ok(_) => return,
                Err(fail) => node = fail.input,
            }
        }
    }

    #[inline]
    pub fn pop(&self) -> Option<T> {
        let mut guard = Guard::new();

        // (TRE:2) this `Acquire` load synchronizes with the `Release` CAS in (TRE:1)
        while let Some(head) = self.head.load(Acquire, &mut guard) {
            let next = head.next.load_unprotected(Relaxed);

            // (TRE:3) this `Release` CAS synchronizes-with the `Acquire` load in (TRE:2)
            if let Ok(unlinked) = self.head.compare_exchange_weak(head, next, Release, Relaxed) {
                // the `Drop` code for T is never called for retired nodes, so it is
                // safe to use `retire_unchecked` and not require that `T: 'static`.
                unsafe {
                    let res = ptr::read(&*unlinked.elem);
                    unlinked.retire_unchecked();

                    return Some(res);
                }
            }
        }

        None
    }
}

impl<T> Drop for Stack<T> {
    fn drop(&mut self) {
        let mut curr = self.head.take();

        // it's necessary to manually drop all elements iteratively
        while let Some(mut node) = curr {
            unsafe { ManuallyDrop::drop(&mut node.elem) }
            curr = node.next.take();
        }
    }
}

#[derive(Debug)]
struct Node<T> {
    elem: ManuallyDrop<T>,
    next: Atomic<Node<T>>,
}

impl<T> Node<T> {
    #[inline]
    fn new(elem: T) -> Self {
        Self { elem: ManuallyDrop::new(elem), next: Atomic::null() }
    }
}
