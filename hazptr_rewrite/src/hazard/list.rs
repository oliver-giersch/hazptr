//! An iterable lock-free data structure for storing hazard pointers.

use core::iter::FusedIterator;
use core::mem::{self, MaybeUninit};
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

#[cfg(not(feature = "std"))]
use alloc::boxed::Box;

use conquer_util::align::Aligned128 as CacheAligned;

use crate::hazard::{HazardPtr, FREE, NOT_YET_USED, THREAD_RESERVED};
use std::ptr::NonNull;

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

    #[inline]
    pub fn iter(&self) -> Iter {
        Iter { idx: 0, curr: unsafe { self.head.load(Ordering::Acquire).as_ref() } }
    }

    #[inline]
    unsafe fn get_or_insert_unchecked(&self, protect: *const (), order: Ordering) -> &HazardPtr {
        let mut prev = &self.head as *const AtomicPtr<HazardArrayNode>;
        let mut curr = (*prev).load(Ordering::Acquire);
        while !curr.is_null() {
            if let Some(hazard) = self.try_insert_in_node(curr as *const _, protect, order) {
                return hazard;
            }

            prev = &(*curr).next.aligned as *const _;
            curr = (*prev).load(Ordering::Acquire);
        }

        self.insert_back(prev, protect, order)
    }

    #[inline]
    unsafe fn insert_back(
        &self,
        mut tail: *const AtomicPtr<HazardArrayNode>,
        protected: *const (),
        order: Ordering,
    ) -> &HazardPtr {
        let node = Box::into_raw(Box::new(HazardArrayNode::new(protected)));
        while let Err(tail_node) =
            (*tail).compare_exchange(ptr::null_mut(), node, Ordering::AcqRel, Ordering::Acquire)
        {
            // try insert in tail node, if success return and deallocate node again
            if let Some(hazard) = self.try_insert_in_node(tail_node, protected, order) {
                Box::from_raw(node);
                return hazard;
            }

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
        for element in &(*node).elements[1..] {
            let hazard = &element.aligned;
            let curr = hazard.protected.load(Ordering::Relaxed);
            let success = (curr == FREE || curr == NOT_YET_USED)
                && hazard
                    .protected
                    .compare_exchange(curr, protected as *mut (), order, Ordering::Relaxed)
                    .is_ok();

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
    fn insert_reserved_full_node_plus_one() {}

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
