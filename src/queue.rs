/// A simple lock-free stack that uses *compare-and-swap* to insert elements at
/// the head and *swap* (exchange) to consume all elements at once, thereby not
/// requiring any dedicated memory reclamation mechanism.
///
/// The raw implementation is deliberately bare-bones as it is used in two
/// different places for different purposes, which are added on top of the
/// bare-bones (raw) implementation in this module.
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
    /// Creates a new empty `RawQueue`.
    #[inline]
    pub const fn new() -> Self {
        Self { head: AtomicPtr::new(ptr::null_mut()) }
    }
}

impl<N: RawNode> RawQueue<N> {
    /// Returns `true` if the queue is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Relaxed).is_null()
    }

    /// Pushes `node` to the head of the queue.
    ///
    /// # Safety
    ///
    /// `node` must be non-null and valid (alive and not mutably aliased).
    #[inline]
    pub unsafe fn push(&self, node: *mut N) {
        loop {
            let head = self.head.load(Ordering::Relaxed);
            N::set_next(node, head);

            if self.cas_head(head, node) {
                return;
            }
        }
    }

    /// Pushes the sub-list formed by `first` and `last` to the head of the
    /// queue
    ///
    /// # Safety
    ///
    /// `(first, last)` must form the head and the tail of a consecutively
    /// linked sub-list.
    /// Both must be non-null and valid.
    #[inline]
    pub unsafe fn push_many(&self, (first, last): (*mut N, *mut N)) {
        loop {
            let head = self.head.load(Ordering::Relaxed);
            N::set_next(last, head);

            if self.cas_head(head, first) {
                return;
            }
        }
    }

    /// Swaps out the first node and leaves the `RawQueue` empty.
    ///
    /// The returned node (if it is non-`null`) effectively owns all following
    /// nodes and can deallocate or mutate them as desired.
    #[inline]
    pub fn take_all(&self) -> *mut N {
        self.head.swap(ptr::null_mut(), Ordering::Acquire)
    }

    /// Same as take all, but without synchronization or ordering constraints.
    /// Requires exclusive access through the `&mut self` receiver.
    #[inline]
    pub fn take_all_unsync(&mut self) -> *mut N {
        self.head.swap(ptr::null_mut(), Ordering::Relaxed)
    }

    #[inline]
    unsafe fn cas_head(&self, current: *mut N, new: *mut N) -> bool {
        self.head.compare_exchange_weak(current, new, Ordering::Release, Ordering::Relaxed).is_ok()
    }
}
