use std::mem::{self, ManuallyDrop};
use std::ptr;
use std::sync::atomic::Ordering;

use hazptr::guarded;

type Atomic<T> = hazptr::Atomic<T, hazptr::typenum::U0>;
type Owned<T> = hazptr::Owned<T, hazptr::typenum::U0>;

#[derive(Default)]
pub struct TreiberStack<T> {
    head: Atomic<Node<T>>,
}

impl<T: 'static> TreiberStack<T> {
    #[inline]
    pub fn new() -> Self {
        Self { head: Atomic::null() }
    }

    #[inline]
    pub fn push(&self, elem: T) {
        let mut node = Owned::new(Node::new(elem));
        let mut guard = guarded();

        loop {
            let head = self.head.load(Ordering::Relaxed, &mut guard);
            node.next.store(head, Ordering::Relaxed);
            match self.head.compare_exchange_weak(head, node, Ordering::Release, Ordering::Relaxed)
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
            //load
            let next = head.next.load_unprotected(Ordering::Relaxed);

            if let Ok(unlinked) =
                self.head.compare_exchange_weak(head, next, Ordering::Release, Ordering::Relaxed)
            {
                unsafe {
                    let res = ptr::read(&*unlinked.elem);
                    unlinked.retire();

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
        Self { elem: ManuallyDrop::new(elem), next: Atomic::null() }
    }
}
