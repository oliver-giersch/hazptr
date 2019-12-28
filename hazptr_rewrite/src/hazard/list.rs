//! An iterable lock-free data structure for storing hazard pointers.

use core::iter::FusedIterator;
use core::mem::{self, MaybeUninit};
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        use std::boxed::Box;
    } else {
        use alloc::boxed::Box;
    }
}

use conquer_util::align::Aligned128 as CacheAligned;

use crate::hazard::{HazardPtr, ProtectedPtr, FREE, THREAD_RESERVED};

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
    pub fn get_or_insert_reserved_hazard(&self) -> &HazardPtr {
        unsafe { self.get_or_insert_unchecked(THREAD_RESERVED, Ordering::Relaxed) }
    }

    /// Acquires a hazard pointer and sets it to point at `protected`.
    #[cold]
    #[inline(never)]
    pub fn get_or_insert_hazard(&self, protected: ProtectedPtr) -> &HazardPtr {
        unsafe { self.get_or_insert_unchecked(protected.as_const_ptr(), Ordering::SeqCst) }
    }

    #[inline]
    pub fn iter(&self) -> Iter {
        Iter { idx: 0, curr: unsafe { self.head.load(Ordering::Acquire).as_ref() } }
    }

    #[inline]
    unsafe fn get_or_insert_unchecked(&self, protected: *const (), order: Ordering) -> &HazardPtr {
        let mut prev = &self.head as *const AtomicPtr<HazardArrayNode>;
        let mut curr = (*prev).load(Ordering::Acquire); // acquire
        while !curr.is_null() {
            if let Some(hazard) = self.try_insert_in_node(curr as *const _, protected, order) {
                return hazard;
            }

            prev = &(*curr).next.aligned as *const _;
            curr = (*prev).load(Ordering::Acquire);
        }

        self.insert_back(prev, protected, order)
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
            // try insert in tail_node, if success return and deallocate
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
            let success = hazard.protected.load(Ordering::Relaxed) == FREE
                && hazard
                    .protected
                    .compare_exchange(FREE, protected as *mut (), order, Ordering::Relaxed)
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
                return Some(&node.elements[self.idx].aligned);
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

        unsafe {
            Self {
                elements: mem::transmute(elements),
                next: CacheAligned::new(AtomicPtr::default()),
            }
        }
    }
}
