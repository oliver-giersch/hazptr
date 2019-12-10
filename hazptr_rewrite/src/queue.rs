use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicPtr, Ordering};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Node (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

pub trait RawNode {
    unsafe fn next(node: *mut Self) -> *mut Self;
    unsafe fn set_next(node: *mut Self, next: *mut Self);
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Queue
////////////////////////////////////////////////////////////////////////////////////////////////////

// AbandonedBags -> insert: Box<_>, take: Option<Box<_>> (impl Node for RetiredBag {}),
// DynAnyNode (retired records) impl Node for *mut dyn AnyNode
pub struct RawQueue<N> {
    head: AtomicPtr<N>,
}

/********** impl inherent *************************************************************************/

impl<N> RawQueue<N> {
    pub const fn new() -> Self {
        Self { head: AtomicPtr::new(ptr::null_mut()) }
    }
}

impl<N: RawNode> RawQueue<N> {
    #[inline]
    pub unsafe fn push(&self, node: NonNull<N>) {
        // eg. N = dyn AnyNode, N = RetiredBag
        // Node::into_thin_ptr(node) [*mut RetiredBag -> *mut RetiredBag, *mut dyn AnyNode -> *mut DynAnyNode]
        /*loop {
            let head = self.head.load(Ordering::Relaxed);
            leaked.set_next(head);

            if self
                .head
                .compare_exchange_weak(head, leaked, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }*/

        unimplemented!()
    }

    #[inline]
    pub fn take_all(&self) -> Option<NonNull<N>> {
        NonNull::new(self.head.swap(ptr::null_mut(), Ordering::Acquire))
    }
}
