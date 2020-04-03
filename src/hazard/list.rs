//! An iterable lock-free data structure for storing hazard pointers.
//!
//! The data structure never de-allocates any nodes until it is dropped but is
//! able to reuse individual elements.

use core::iter::FusedIterator;
use core::mem::MaybeUninit;
use core::ptr::{self, NonNull};
use core::sync::atomic::{
    AtomicPtr,
    Ordering::{self, AcqRel, Acquire, Relaxed, SeqCst},
};

#[cfg(not(feature = "std"))]
use alloc::boxed::Box;

use crate::hazard::{HazardPtr, FREE, NOT_YET_USED, THREAD_RESERVED};

/// The number of hazard pointers in a hazard list node.
const ELEMENTS: usize = 128;

////////////////////////////////////////////////////////////////////////////////////////////////////
// HazardList
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A linked list of [`HazardArrayNode`]s containing re-usable hazard pointers.
///
/// When requesting a hazard pointer, the list is traversed from head to tail
/// and each node is searched for a [`FREE`] hazard pointer.
/// If none can be found a new node is appended to the list's tail.
/// In order to avoid having to deal with memory reclamation the list never
/// shrinks and hence maintains its maximum extent at all times.
#[derive(Debug, Default)]
pub(crate) struct HazardList {
    /// Atomic pointer to the head of the linked list.
    head: AtomicPtr<Node>,
}

/********** impl inherent *************************************************************************/

impl HazardList {
    /// Creates a new empty `HazardList`.
    #[inline]
    pub const fn new() -> Self {
        Self { head: AtomicPtr::new(ptr::null_mut()) }
    }

    /// Acquires a thread-reserved hazard pointer.
    #[cold]
    #[inline(never)]
    #[must_use = "discarding a reserved hazard pointer without freeing it renders it unusable"]
    pub fn get_or_insert_reserved_hazard(&self) -> &HazardPtr {
        // acquires a hazard pointer with a relaxed CAS, setting it to reserved (no synchronization
        // constraints are required because no memory accesses are affected)
        unsafe { self.get_or_insert_unchecked(THREAD_RESERVED, Relaxed) }
    }

    /// Acquires a hazard pointer and sets it to point at `protected`.
    #[cold]
    #[inline(never)]
    #[must_use = "discarding a reserved hazard pointer without freeing it renders it unusable"]
    pub fn get_or_insert_hazard(&self, protect: NonNull<()>) -> &HazardPtr {
        // (lst:1) this seq-cst CAS syncs-with the seq-cst fence (glo:1)
        unsafe { self.get_or_insert_unchecked(protect.as_ptr() as _, SeqCst) }
    }

    /// Returns an iterator over all currently allocated [`HazardPtr`]s.
    #[inline]
    pub fn iter(&self) -> Iter {
        // (lst:2) this acq load syncs-with the acq-rel CAS (lst:4)
        Iter { idx: 0, curr: unsafe { self.head.load(Acquire).as_ref() } }
    }

    #[inline]
    unsafe fn get_or_insert_unchecked(&self, protect: *const (), order: Ordering) -> &HazardPtr {
        let mut prev = &self.head;
        // (lst:3) this acq load syncs-with the acq-rel CAS (lst:4)
        let mut curr = prev.load(Acquire);

        // iterate the linked list of hazard nodes from the start
        while !curr.is_null() {
            // try to acquire a hazard pointer in the current node
            if let Some(hazard) = self.try_insert_in_node(curr as *const _, protect, order) {
                return hazard;
            }

            // ... if no hazard pointers in the node were currently free, advance to the next node
            prev = &(*curr).next;
            // (lst:4) this acq load syncs-with the acq-rel CAS (lst:4)
            curr = prev.load(Acquire);
        }

        // no free hazard pointer in any already allocated node could be acquired, so insert a new
        // node at the tail of the list
        self.insert_back(prev, protect, order)
    }

    #[inline]
    unsafe fn insert_back(
        &self,
        mut tail: *const AtomicPtr<Node>,
        protected: *const (),
        order: Ordering,
    ) -> &HazardPtr {
        // allocates a new hazard node with the first hazard already set to `protected`
        let node = Box::into_raw(Box::new(Node::new(protected)));
        // repeat trying to insert the allocated node at the (current) tail
        // (lst:5) this acq-rel/acq CAS syncs-with the acq loads (lst:2-4) and itself
        // todo: should be rel/acq ordering
        while let Err(node) = (*tail).compare_exchange(ptr::null_mut(), node, AcqRel, Acquire) {
            // the CAS failed, so another thread must have already inserted a different node at the
            // tail, try to acquire a hazard pointer from that node first
            if let Some(hazard) = self.try_insert_in_node(node, protected, order) {
                // a hazard pointer was successfully acquired from the inserted node, so the one
                // allocated by the current thread can be de-allocated again and the hazard pointer
                // returned
                Box::from_raw(node);
                return hazard;
            }

            // no hazard pointer could be acquired, so update the local tail pointer variable and
            // try inserting the allocated node again at the new tail
            tail = &(*node).next;
        }

        // the node was successfully inserted at the tail, so the pre-reserved hazard pointer can
        // be returned
        &(*node).hazards[0]
    }

    #[inline]
    unsafe fn try_insert_in_node(
        &self,
        node: *const Node,
        protected: *const (),
        order: Ordering,
    ) -> Option<&HazardPtr> {
        // attempts to acquire every hazard pointer in the current `node` once (although the first
        // hazard pointer in each node is pre-reserved on allocation, it may already be free again)
        for hazard in &(*node).hazards[..] {
            let current = hazard.protected.load(Relaxed);

            // if the hazard pointer is not currently in use, try to set it to `protected`
            let success = (current == FREE || current == NOT_YET_USED)
                && hazard
                    .protected
                    .compare_exchange(current, protected as *mut (), order, Relaxed)
                    .is_ok();

            // the hazard pointer was successfully set to `protected`
            if success {
                return Some(hazard);
            }
        }

        None
    }
}

/********** impl Drop *****************************************************************************/

impl Drop for HazardList {
    #[inline(never)]
    fn drop(&mut self) {
        let mut curr = self.head.load(Relaxed);
        while !curr.is_null() {
            let node = unsafe { Box::from_raw(curr) };
            curr = node.next.load(Relaxed);
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Iter
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An iterator over all hazard pointers in a `HazardList`.
///
/// It is advisable to abort the iteration if a hazard pointer that has not yet
/// been used before, because all subsequent hazard pointers are guaranteed to
/// have also not been used.
pub(crate) struct Iter<'a> {
    idx: usize,
    curr: Option<&'a Node>,
}

/********** impl Iterator *************************************************************************/

impl<'a> Iterator for Iter<'a> {
    type Item = &'a HazardPtr;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        // this loop is executed at most twice
        while let Some(node) = self.curr {
            // iteration is at some element of the current node
            if self.idx < ELEMENTS {
                let idx = self.idx;
                self.idx += 1;
                return Some(&node.hazards[idx]);
            } else {
                // set `curr` to its successor and reset iteration index to 0, the subsequent loop
                // iteration must hence go into the first path
                self.curr = unsafe { node.next.load(Acquire).as_ref() };
                self.idx = 0;
            }
        }

        None
    }
}

/********** impl FusedIterator ********************************************************************/

impl FusedIterator for Iter<'_> {}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Node
////////////////////////////////////////////////////////////////////////////////////////////////////

struct Node {
    hazards: [HazardPtr; ELEMENTS],
    next: AtomicPtr<Self>,
}

/********** impl inherent *************************************************************************/

impl Node {
    #[inline]
    fn new(protected: *const ()) -> Self {
        let elements = unsafe {
            let mut elements: MaybeUninit<[HazardPtr; ELEMENTS]> = MaybeUninit::uninit();
            let ptr: *mut HazardPtr = elements.as_mut_ptr().cast();

            ptr.write(HazardPtr::with_protected(protected));
            for idx in 1..ELEMENTS {
                ptr.add(idx).write(HazardPtr::new());
            }

            elements.assume_init()
        };

        Self { hazards: elements, next: AtomicPtr::default() }
    }
}

#[cfg(test)]
mod tests {
    use core::ptr::NonNull;
    use core::sync::atomic::Ordering;

    use super::{HazardList, ELEMENTS};
    use crate::hazard::ProtectedResult::Unprotected;

    #[test]
    fn new() {
        let list = HazardList::new();
        assert!(list.iter().next().is_none());
    }

    #[test]
    fn insert_one() {
        let list = HazardList::new();
        let hazard = list.get_or_insert_reserved_hazard();
        assert_eq!(hazard as *const _, list.iter().next().unwrap() as *const _);
    }

    #[test]
    fn insert_full_node() {
        let list = HazardList::new();

        for _ in 0..ELEMENTS {
            let _ = list.get_or_insert_reserved_hazard();
        }

        let vec: Vec<_> = list.iter().collect();
        assert_eq!(vec.len(), ELEMENTS);
    }

    #[test]
    fn insert_reserved_full_node_plus_one() {
        let list = HazardList::new();

        #[allow(clippy::range_plus_one)]
        for _ in 0..ELEMENTS + 1 {
            let _ = list.get_or_insert_reserved_hazard();
        }

        let hazards: Vec<_> = list.iter().collect();

        assert_eq!(hazards.len(), 2 * ELEMENTS);
        assert_eq!(
            hazards
                .iter()
                .take_while(|hazard| hazard.protected(Ordering::Relaxed) == Unprotected)
                .count(),
            ELEMENTS + 1
        );
    }

    #[test]
    fn insert_protected_full_node_plus_one() {
        let list = HazardList::new();
        let protect = NonNull::from(&mut 1);

        #[allow(clippy::range_plus_one)]
        for _ in 0..ELEMENTS + 1 {
            let _ = list.get_or_insert_hazard(protect.cast());
        }

        let hazards: Vec<_> = list
            .iter()
            .take_while(|hazard| hazard.protected(Ordering::Relaxed).protected().is_some())
            .collect();
        assert_eq!(hazards.len(), ELEMENTS + 1);
    }

    #[test]
    fn reuse_hazard_from_list() {
        let list = HazardList::new();

        for _ in 0..ELEMENTS + (ELEMENTS / 2) {
            let _ = list.get_or_insert_reserved_hazard();
        }

        let hazards: Vec<_> = list.iter().collect();

        let inner_hazard = hazards[ELEMENTS - 2];
        inner_hazard.set_free(Ordering::Relaxed);

        let acquired_hazard = list.get_or_insert_reserved_hazard();
        assert_eq!(inner_hazard as *const _, acquired_hazard as *const _);
    }
}
