//! An iterable lock-free data structure for storing hazard pointers.

use core::iter::FusedIterator;
use core::mem::{self, MaybeUninit};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicPtr, Ordering};

#[cfg(not(feature = "std"))]
use alloc::boxed::Box;

use conquer_util::align::Aligned128 as CacheAligned;

use crate::hazard::{HazardPtr, FREE, NOT_YET_USED, THREAD_RESERVED};

/// The number of elements is chosen so that 31 hazards aligned to 128-byte and
/// one likewise aligned next pointer fit into a 4096 byte memory page.
const ELEMENTS: usize = 31;

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
    head: AtomicPtr<HazardArrayNode>,
}

/********** impl inherent *************************************************************************/

impl HazardList {
    /// Creates a new empty [`HazardList`].
    #[inline]
    pub const fn new() -> Self {
        Self { head: AtomicPtr::new(ptr::null_mut()) }
    }

    /// Acquires a thread-reserved hazard pointer.
    #[cold]
    #[inline(never)]
    #[must_use = "discarding a reserved hazard pointer without freeing it renders it unusable"]
    pub fn get_or_insert_reserved_hazard(&self) -> &HazardPtr {
        unsafe { self.get_or_insert_unchecked(THREAD_RESERVED, Ordering::Relaxed) }
    }

    /// Acquires a hazard pointer and sets it to point at `protected`.
    #[cold]
    #[inline(never)]
    #[must_use = "discarding a reserved hazard pointer without freeing it renders it unusable"]
    pub fn get_or_insert_hazard(&self, protect: NonNull<()>) -> &HazardPtr {
        unsafe { self.get_or_insert_unchecked(protect.as_ptr() as _, Ordering::SeqCst) }
    }

    /// Returns an iterator over all currently allocated [`HazardPointers`].
    #[inline]
    pub fn iter(&self) -> Iter {
        Iter { idx: 0, curr: unsafe { self.head.load(Ordering::Acquire).as_ref() } }
    }

    #[inline]
    unsafe fn get_or_insert_unchecked(&self, protect: *const (), order: Ordering) -> &HazardPtr {
        let mut prev = &self.head as *const AtomicPtr<HazardArrayNode>;
        let mut curr = (*prev).load(Ordering::Acquire);
        
        // iterate the linked list of hazard nodes
        while !curr.is_null() {
            // try to acquire a hazard pointer in the current node
            if let Some(hazard) = self.try_insert_in_node(curr as *const _, protect, order) {
                return hazard;
            }

            prev = &(*curr).next.aligned as *const _;
            curr = (*prev).load(Ordering::Acquire);
        }

        // no hazard pointer could be acquired in any already allocated node, insert a new node at
        // the tail of the list
        self.insert_back(prev, protect, order)
    }

    #[inline]
    unsafe fn insert_back(
        &self,
        mut tail: *const AtomicPtr<HazardArrayNode>,
        protected: *const (),
        order: Ordering,
    ) -> &HazardPtr {
        // allocates a new hazard node with the first hazard already set to `protected`
        let node = Box::into_raw(Box::new(HazardArrayNode::new(protected)));
        while let Err(tail_node) =
            (*tail).compare_exchange(ptr::null_mut(), node, Ordering::AcqRel, Ordering::Acquire)
        {
            // try insert in tail node, on success return and deallocate node again
            if let Some(hazard) = self.try_insert_in_node(tail_node, protected, order) {
                Box::from_raw(node);
                return hazard;
            }

            // update the local tail pointer
            tail = &(*tail_node).next.aligned;
        }

        &(*node).elements[0].aligned
    }

    #[inline]
    unsafe fn try_insert_in_node(
        &self,
        node: *const HazardArrayNode,
        protected: *const (),
        order: Ordering,
    ) -> Option<&HazardPtr> {
        // attempts to acquire every hazard pointer in the current `node` once
        for element in &(*node).elements[..] {
            let hazard = &element.aligned;
            let current = hazard.protected.load(Ordering::Relaxed);
            let success = (current == FREE || current == NOT_YET_USED)
                && hazard
                    .protected
                    .compare_exchange(current, protected as *mut (), order, Ordering::Relaxed)
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
        let mut curr = self.head.load(Ordering::Relaxed);
        while !curr.is_null() {
            let node = unsafe { Box::from_raw(curr) };
            curr = node.next.aligned.load(Ordering::Relaxed);
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Iter
////////////////////////////////////////////////////////////////////////////////////////////////////

pub(crate) struct Iter<'a> {
    idx: usize,
    curr: Option<&'a HazardArrayNode>,
}

/********** impl Iterator *************************************************************************/

impl<'a> Iterator for Iter<'a> {
    type Item = &'a HazardPtr;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        // this loop is executed at most twice
        while let Some(node) = self.curr {
            if self.idx < ELEMENTS {
                let idx = self.idx;
                self.idx += 1;
                return Some(&node.elements[idx].aligned);
            } else {
                self.curr = unsafe { node.next.aligned.load(Ordering::Acquire).as_ref() };
                self.idx = 0;
            }
        }

        None
    }
}

/********** impl FusedIterator ********************************************************************/

impl FusedIterator for Iter<'_> {}

////////////////////////////////////////////////////////////////////////////////////////////////////
// HazardArrayNode
////////////////////////////////////////////////////////////////////////////////////////////////////

struct HazardArrayNode {
    elements: [CacheAligned<HazardPtr>; ELEMENTS],
    next: CacheAligned<AtomicPtr<HazardArrayNode>>,
}

/********** impl inherent *************************************************************************/

impl HazardArrayNode {
    #[inline]
    fn new(protected: *const ()) -> Self {
        let mut elements: [MaybeUninit<CacheAligned<HazardPtr>>; ELEMENTS] =
            unsafe { MaybeUninit::uninit().assume_init() };

        elements[0] = MaybeUninit::new(CacheAligned::new(HazardPtr::with_protected(protected)));
        for elem in &mut elements[1..] {
            *elem = MaybeUninit::new(CacheAligned::new(HazardPtr::new()));
        }

        Self {
            elements: unsafe { mem::transmute(elements) },
            next: CacheAligned::new(AtomicPtr::default()),
        }
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
}
