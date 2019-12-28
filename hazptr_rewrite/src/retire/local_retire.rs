use core::cmp;
use core::mem;
use core::ptr;

cfg_if::cfg_if! {
    if #[cfg(not(feature = "std"))] {
        use alloc::boxed::Box;
        use alloc::vec::Vec;
    }
}

use conquer_reclaim::RawRetired;

use crate::global::Global;
use crate::hazard::ProtectedPtr;
use crate::queue::{RawNode, RawQueue};
use crate::retire::RetireStrategy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalRetire
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Default)]
pub struct LocalRetire(Box<RetireNode>);

/********** impl RetireStrategy *******************************************************************/

impl RetireStrategy for LocalRetire {
    type Global = AbandonedQueue;
    type Header = (); // no additional per-record state is required

    #[inline]
    fn new(global: &Global<Self>) -> Self {
        // adopt abandoned records or allocate new node
        match global.state.take_all_and_merge() {
            Some(node) => Self(node),
            None => Self(Default::default()),
        }
    }

    #[inline]
    fn drop(self, global: &Global<Self>) {
        if !self.0.vec.is_empty() {
            global.state.push(self.0);
        }
    }

    #[inline]
    fn no_retired_records(&self, _: &Global<Self>) -> bool {
        self.0.vec.is_empty()
    }

    #[inline]
    unsafe fn reclaim_all_unprotected(
        &mut self,
        global: &Global<Self>,
        protected: &[ProtectedPtr],
    ) {
        if let Some(node) = global.state.take_all_and_merge() {
            self.0.merge(node.vec)
        }

        self.0.vec.retain(|retired| {
            // retain (i.e. DON'T drop) all records found within the scan cache of protected hazards
            protected.binary_search_by(|&protected| retired.compare_with(protected)).is_ok()
        });
    }

    #[inline]
    unsafe fn retire(&mut self, _: &Global<Self>, retired: RawRetired) {
        self.0.vec.push(ReclaimOnDrop::new(retired));
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// RetireNode
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct RetireNode {
    vec: Vec<ReclaimOnDrop>,
    next: *mut Self,
}

/********** impl inherent *************************************************************************/

impl RetireNode {
    const DEFAULT_INITIAL_CAPACITY: usize = 128;

    #[inline]
    fn merge(&mut self, mut other: Vec<ReclaimOnDrop>) {
        if (other.capacity() - other.len()) > self.vec.capacity() {
            mem::swap(&mut self.vec, &mut other);
        }

        self.vec.append(&mut other);
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
    unsafe fn next(node: *mut Self) -> *mut Self {
        (*node).next
    }

    unsafe fn set_next(node: *mut Self, next: *mut Self) {
        (*node).next = next;
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// AbandonedQueue
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Default)]
pub struct AbandonedQueue {
    raw: RawQueue<RetireNode>,
}

/********** impl inherent *************************************************************************/

impl AbandonedQueue {
    #[inline]
    fn push(&self, node: Box<RetireNode>) {
        let node = Box::leak(node);
        unsafe { self.raw.push(node) };
    }

    #[inline]
    fn take_all_and_merge(&self) -> Option<Box<RetireNode>> {
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

////////////////////////////////////////////////////////////////////////////////////////////////////
// ReclaimOnDrop
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
struct ReclaimOnDrop(RawRetired);

/********** impl inherent *************************************************************************/

impl ReclaimOnDrop {
    #[inline]
    unsafe fn new(retired: RawRetired) -> Self {
        Self(retired)
    }

    #[inline]
    fn compare_with(&self, protected: ProtectedPtr) -> cmp::Ordering {
        protected.address().cmp(&self.0.address())
    }
}

/********** impl Drop *****************************************************************************/

impl Drop for ReclaimOnDrop {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe { self.0.reclaim() };
    }
}
