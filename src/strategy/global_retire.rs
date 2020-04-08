//! Implementation of the global retire strategy.

use core::ptr;

use conquer_reclaim::RetiredPtr;

use crate::hazard::ProtectedPtr;
use crate::queue::{RawNode, RawQueue};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Header
////////////////////////////////////////////////////////////////////////////////////////////////////

/// With a global retire strategy, every record is allocated in a way that
/// allows it to be inserted into a linked list of retired records, so it
/// contains a next pointer, which is initially `null`.
/// The `retired` field is only set once when a record is retired and inserted
/// into the global linked list (queue) of retired records.
/// A [`RawRetired`] is essentially a fat pointer.
/// The first half points at the record itself and the second half points at its
/// `Drop` implementation (its vtable, actually).
/// By storing it in the records header itself, the header contains all relevant
/// information for traversing the linked list and reclaiming the records memory
/// without concern for its concrete type.
#[derive(Debug)]
pub struct Header {
    /// The pointer to the header of the next retired record.
    next: *mut Self,
    /// The handle for the retired record itself, which is set when a record is
    /// retired.
    retired: Option<RetiredPtr>,
}

/*********** impl Default *************************************************************************/

impl Default for Header {
    #[inline]
    fn default() -> Self {
        Self { next: ptr::null_mut(), retired: None }
    }
}

/*********** impl RawNode *************************************************************************/

impl RawNode for Header {
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
// RetiredQueue
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A linked-list based for storing retired records.
///
/// Every record must be allocated with a [`Header`] that allows it to be
/// inserted into the queue and to be later reclaimed.
/// This data-structure forms a singly linked list of record headers of retired
/// records.
pub(crate) struct RetiredQueue {
    raw: RawQueue<Header>,
}

/********** impl inherent *************************************************************************/

impl RetiredQueue {
    /// Creates a new empty `RetiredQueue`.
    #[inline]
    pub const fn new() -> Self {
        Self { raw: RawQueue::new() }
    }

    /// Returns `true` if the queue is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    #[inline]
    pub fn take_all(&self) -> Taken {
        Taken { curr: self.raw.take_all() }
    }

    #[inline]
    pub fn push_back_unreclaimed(&self, unreclaimed: Unreclaimed) {
        unsafe { self.raw.push_many((unreclaimed.first, unreclaimed.last)) };
    }

    /// Pushes `retired` into the queue.
    ///
    /// # Safety
    ///
    /// The caller has to ensure `retired` points at a record that has a header
    /// of the correct type.
    /// Specifically, this requires that `retired` was derived from a
    /// `Retired<Hp<GlobalRetire>>`.
    #[inline]
    pub unsafe fn retire_record(&self, retired: RetiredPtr) {
        // `retired` points to a record, which has layout guarantees regarding field ordering
        // and the record's header is always located at the beginning
        let header = retired.as_ptr() as *mut Header;
        (*header).retired = Some(retired);

        self.raw.push(header);
    }
}

/********** impl Drop *****************************************************************************/

impl Drop for RetiredQueue {
    #[inline(never)]
    fn drop(&mut self) {
        // when the global state is dropped, there can be no longer any active
        // threads and all remaining records can be simply de-allocated.
        let mut curr = self.raw.take_all_unsync();
        while !curr.is_null() {
            unsafe {
                let next = Header::next(curr);
                (*curr).retired.take().unwrap().reclaim();
                curr = next;
            }
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Taken
////////////////////////////////////////////////////////////////////////////////////////////////////

pub(crate) struct Taken {
    curr: *mut Header,
}

impl Taken {
    pub unsafe fn reclaim_all_unprotected(
        mut self,
        scan_cache: &[ProtectedPtr],
    ) -> Result<(), Unreclaimed> {
        // these pointers will form the queue of unreclaimed records that need to be pushed back
        // into the global queue
        let (mut first, mut last): (*mut Header, *mut Header) = (ptr::null_mut(), ptr::null_mut());

        // iterate over retired records and reclaim all which are no longer protected
        while !self.curr.is_null() {
            // `(*curr).next` must be read HERE because `curr` may be de-allocated in the next step
            let next = Header::next(self.curr);
            // all retired records point at the entire record (including the header), whereas all
            // hazard pointers point at data, so the offset needs to be calculated before comparing
            let data_ptr = (*self.curr).retired.as_ref().unwrap().data_ptr();
            match scan_cache.binary_search_by(|protected| protected.compare_with(data_ptr)) {
                // the record is still protected by some hazard pointer
                Ok(_) => {
                    if !first.is_null() {
                        // insert `curr` after `last`
                        Header::set_next(last, self.curr);
                        last = self.curr;
                    } else {
                        // first entry, set first and last
                        first = self.curr;
                        last = self.curr;
                    }
                }
                // the record can be reclaimed
                Err(_) => (*self.curr).retired.take().unwrap().reclaim(),
            }

            self.curr = next;
        }

        // if not all were reclaimed, the unreclaimed ones must be pushed back to the global queue.
        match first {
            ptr if ptr.is_null() => Ok(()),
            _ => Err(Unreclaimed { first, last }),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Unreclaimed
////////////////////////////////////////////////////////////////////////////////////////////////////

pub(crate) struct Unreclaimed {
    first: *mut Header,
    last: *mut Header,
}
