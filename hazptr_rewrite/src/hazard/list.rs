use core::mem::{self, MaybeUninit};
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use conquer_util::align::Aligned128 as CacheAligned;

use crate::hazard::{Hazard, Protected, FREE, THREAD_RESERVED};

// the number of elements is chosen so that 31 hazards aligned to 128-byte and
// one likewise aligned next pointer fit into a 4096 byte memory page.
const ELEMENTS: usize = 31;

////////////////////////////////////////////////////////////////////////////////////////////////////
// HazardList
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub(crate) struct HazardList {
    head: AtomicPtr<HazardArrayNode>,
}

/********** impl inherent *************************************************************************/

impl HazardList {
    #[inline]
    pub const fn new() -> Self {
        Self { head: AtomicPtr::new(ptr::null_mut()) }
    }

    #[cold]
    #[inline(never)]
    pub fn get_or_insert_reserved_hazard(&self) -> &Hazard {
        unsafe { self.get_or_insert_unchecked(THREAD_RESERVED, Ordering::Relaxed) }
    }

    #[cold]
    #[inline(never)]
    pub fn get_or_insert_hazard(&self, protected: Protected) -> &Hazard {
        unsafe { self.get_or_insert_unchecked(protected.as_const_ptr(), Ordering::SeqCst) }
    }

    #[inline]
    pub fn iter(&self) -> Iter {
        Iter { idx: 0, curr: unsafe { self.head.load(Ordering::Acquire).as_ref() } }
    }

    #[inline]
    unsafe fn get_or_insert_unchecked(&self, protected: *const (), order: Ordering) -> &Hazard {
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
    ) -> &Hazard {
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
    ) -> Option<&Hazard> {
        for element in &(*node).elements[1..] {
            let hazard = &element.aligned;
            if hazard.protected.load(Ordering::Relaxed) == FREE {
                if hazard
                    .protected
                    .compare_exchange(FREE, protected as *mut (), order, Ordering::Relaxed)
                    .is_ok()
                {
                    return Some(hazard);
                }
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
    type Item = &'a Hazard;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        // loop is executed at most twice
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

////////////////////////////////////////////////////////////////////////////////////////////////////
// HazardArrayNode
////////////////////////////////////////////////////////////////////////////////////////////////////

struct HazardArrayNode {
    elements: [CacheAligned<Hazard>; ELEMENTS],
    next: CacheAligned<AtomicPtr<HazardArrayNode>>,
}

/********** impl inherent *************************************************************************/

impl HazardArrayNode {
    #[inline]
    fn new(protected: *const ()) -> Self {
        let mut elements: [MaybeUninit<CacheAligned<Hazard>>; ELEMENTS] =
            unsafe { MaybeUninit::uninit().assume_init() };

        elements[0] = MaybeUninit::new(CacheAligned::new(Hazard::with_protected(protected)));
        for elem in &mut elements[1..] {
            *elem = MaybeUninit::new(CacheAligned::new(Hazard::new()));
        }

        unsafe {
            Self {
                elements: mem::transmute(elements),
                next: CacheAligned::new(AtomicPtr::default()),
            }
        }
    }
}
