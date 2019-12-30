use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

////////////////////////////////////////////////////////////////////////////////////////////////////
// RawNode (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A trait for node types that contain `next` pointers and can be accessed
/// through raw pointers.
pub(crate) trait RawNode {
    /// Returns the `node`'s next pointer.
    ///
    /// # Safety
    ///
    /// The caller has to ensure `node` is a valid pointer to a mutable node and
    /// that the aliasing rules are not violated.
    unsafe fn next(node: *mut Self) -> *mut Self;

    /// Sets the `node`'s next pointer to `next`.
    ///
    /// # Safety
    ///
    /// The caller has to ensure `node` is a valid pointer to a mutable node and
    /// that the aliasing rules are not violated.
    unsafe fn set_next(node: *mut Self, next: *mut Self);
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// RawQueue
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A concurrent linked-list based queue operating on raw pointers that serves
/// as a building block for more specialized data structures.
///
/// Elements are inserted at the front (i.e. in FIFO order) and can only be
/// removed all at once by returning the first node which contains a link to the
/// next node and so on and switching the queue to empty.
#[derive(Debug, Default)]
pub(crate) struct RawQueue<N> {
    head: AtomicPtr<N>,
}

/********** impl inherent *************************************************************************/

impl<N> RawQueue<N> {
    /// Creates a new empty [`RawQueue`].
    #[inline]
    pub const fn new() -> Self {
        Self { head: AtomicPtr::new(ptr::null_mut()) }
    }
}

impl<N: RawNode> RawQueue<N> {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Relaxed).is_null()
    }

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

    #[inline]
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

    /// Swaps out the first node and leaves the [`RawQueue`] empty.
    ///
    /// The returned node (if it is non-`null`) effectively owns all following
    /// nodes and can deallocate or mutate them as required.
    #[inline]
    pub fn take_all(&self) -> *mut N {
        self.head.swap(ptr::null_mut(), Ordering::Acquire)
    }
}
