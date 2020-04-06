use core::cmp;
use core::mem;
use core::ptr;

cfg_if::cfg_if! {
    if #[cfg(not(feature = "std"))] {
        use alloc::boxed::Box;
        use alloc::vec::Vec;
    }
}

use conquer_reclaim::RetiredPtr;

use crate::hazard::ProtectedPtr;
use crate::queue::{RawNode, RawQueue};

////////////////////////////////////////////////////////////////////////////////////////////////////
// RetireNode
////////////////////////////////////////////////////////////////////////////////////////////////////

/// The storage for locally retired records, which can be later stored in the
/// global (linked list) queue of abandoned records, when the owning thread
/// exits and there are still some un-reclaimed records present in the storage.
#[derive(Debug)]
pub(crate) struct RetireNode {
    vec: Vec<ReclaimOnDrop>,
    next: *mut Self,
}

/********** impl inherent *************************************************************************/

impl RetireNode {
    /// The initial capacity of the `Vec` of retired record pointers
    pub const DEFAULT_INITIAL_CAPACITY: usize = 128;

    /// Returns the inner `Vec` of retired records.
    #[inline]
    pub fn into_inner(self) -> Vec<ReclaimOnDrop> {
        self.vec
    }

    /// Returns `true` if the `Vec` of retired records is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.vec.is_empty()
    }

    /// Merges the node's retired records with the `Vec` of retired records
    /// extracted from another `RetireNode`.
    #[inline]
    pub fn merge(&mut self, mut other: Vec<ReclaimOnDrop>) {
        if (other.capacity() - other.len()) > self.vec.capacity() {
            mem::swap(&mut self.vec, &mut other);
        }

        self.vec.append(&mut other);
    }

    #[inline]
    pub unsafe fn retire_record(&mut self, retired: RetiredPtr) {
        self.vec.push(ReclaimOnDrop::new(retired));
    }

    #[inline]
    pub unsafe fn reclaim_all_unprotected(&mut self, protected: &[ProtectedPtr]) {
        self.vec.retain(|retired| {
            // retain (i.e. DON'T drop) all records found within the scan cache of protected hazards
            protected.binary_search_by(|&protected| retired.compare_with(protected)).is_ok()
        });
    }
}

/********** impl Default **************************************************************************/

impl Default for RetireNode {
    #[inline]
    fn default() -> Self {
        Self { vec: Vec::with_capacity(Self::DEFAULT_INITIAL_CAPACITY), next: ptr::null_mut() }
    }
}

/********** impl RawNode **************************************************************************/

impl RawNode for RetireNode {
    #[inline]
    unsafe fn next(node: *mut Self) -> *mut Self {
        (*node).next
    }

    #[inline]
    unsafe fn set_next(node: *mut Self, next: *mut Self) {
        (*node).next = next;
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// AbandonedQueue
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Default)]
pub(crate) struct AbandonedQueue {
    raw: RawQueue<RetireNode>,
}

/********** impl inherent *************************************************************************/

impl AbandonedQueue {
    #[inline]
    pub const fn new() -> Self {
        Self { raw: RawQueue::new() }
    }

    #[inline]
    pub fn push(&self, node: Box<RetireNode>) {
        let node = Box::leak(node);
        unsafe { self.raw.push(node) };
    }

    #[inline]
    pub fn take_all_and_merge(&self) -> Option<Box<RetireNode>> {
        unsafe {
            match self.raw.take_all() {
                ptr if ptr.is_null() => None,
                ptr => {
                    let mut boxed = Box::from_raw(ptr);
                    let mut curr = boxed.next;
                    while !curr.is_null() {
                        let RetireNode { vec: container, next } = *Box::from_raw(curr);
                        boxed.merge(container);
                        curr = next;
                    }

                    Some(boxed)
                }
            }
        }
    }
}

/********** impl Drop *****************************************************************************/

impl Drop for AbandonedQueue {
    #[inline(never)]
    fn drop(&mut self) {
        // when the global state is dropped, there can be no longer any active
        // threads and all remaining records can be simply de-allocated.
        let mut curr = self.raw.take_all_unsync();
        while !curr.is_null() {
            unsafe {
                // the box will de-allocated together with the vector containing all retired
                // records, which will likewise be reclaimed upon being dropped.
                let boxed = Box::from_raw(curr);
                curr = boxed.next;
            }
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// ReclaimOnDrop
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A wrapper for a `RetiredPtr` that is reclaimed when it is dropped.
#[derive(Debug)]
pub(crate) struct ReclaimOnDrop {
    retired: RetiredPtr,
}

/********** impl inherent *************************************************************************/

impl ReclaimOnDrop {
    /// Creates a new `ReclaimOnDrop` wrapper for the given `retired`.
    ///
    /// # Safety
    ///
    /// The returned wrapper must not be de-allocated before the reclaimer has
    /// determined that no thread is still holding a hazard pointer for the
    /// retired record.
    ///
    /// Dropping must only occur at two places:
    /// - in the course of `RetireNode::reclaim_all_unprotected` during the call
    ///   to `Vec::retain`.
    /// - when the global `AbandonedQueue` is dropped will still containing
    ///   abandoned `RetireNode`s.
    #[inline]
    unsafe fn new(retired: RetiredPtr) -> Self {
        Self { retired }
    }

    /// Compares the address of the retired record with the `protected` address.
    #[inline]
    fn compare_with(&self, protected: ProtectedPtr) -> cmp::Ordering {
        protected.address().cmp(&self.retired.address())
    }
}

/********** impl Drop *****************************************************************************/

impl Drop for ReclaimOnDrop {
    #[inline(always)]
    fn drop(&mut self) {
        // safety: This is only safe if the invariants of construction are maintained.
        unsafe { self.retired.reclaim() };
    }
}
