use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicPtr, Ordering};

////////////////////////////////////////////////////////////////////////////////////////////////////
// RawNode (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

pub trait RawNode {
    unsafe fn next(node: *mut Self) -> *mut Self;
    unsafe fn set_next(node: *mut Self, next: *mut Self);
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// RawQueue
////////////////////////////////////////////////////////////////////////////////////////////////////

// AbandonedBags -> insert: Box<_>, take: Option<Box<_>> (impl Node for RetiredBag {}),
// DynAnyNode (retired records) impl Node for *mut dyn AnyNode
#[derive(Debug, Default)]
pub struct RawQueue<N> {
    head: AtomicPtr<N>,
}

/********** impl inherent *************************************************************************/

impl<N> RawQueue<N> {
    #[inline]
    pub const fn new() -> Self {
        Self { head: AtomicPtr::new(ptr::null_mut()) }
    }
}

impl<N: RawNode> RawQueue<N> {
    #[inline]
    pub unsafe fn push(&self, node: *mut N) {
        loop {
            let head = self.head.load(Ordering::Relaxed);
            N::set_next(node, head);

            if self
                .head
                .compare_exchange_weak(head, node, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }

    pub unsafe fn push_many(&self, (first, last): (*mut N, *mut N)) {
        loop {
            let head = self.head.load(Ordering::Relaxed);
            N::set_next(last, head);

            if self
                .head
                .compare_exchange_weak(head, first, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }

    #[inline]
    pub fn take_all(&self) -> *mut N {
        self.head.swap(ptr::null_mut(), Ordering::Acquire)
    }
}
