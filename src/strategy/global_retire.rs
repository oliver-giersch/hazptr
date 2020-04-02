//! Implementation of the global retire strategy.
//!
//! With this strategy, all threads store their retired records in a single
//! global data structure.
//! This means, that all threads can potentially reclaim records by all other
//! threads, which is especially useful when only certain threads ever retire
//! any records but all threads should be able to help in reclaiming these
//! records.
//! It can also be applicable if records are only retired fairly infrequently.
//!
//! The disadvantages for this strategy lie in the increased synchronization
//! overhead, since every retired record requires a synchronized access to a
//! single global shared data structure, which limits scalability.

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
#[derive(Debug, Default)]
pub(crate) struct RetiredQueue {
    raw: RawQueue<Header>,
}

/********** impl inherent *************************************************************************/

impl RetiredQueue {
    /// Creates a new empty [`RetiredQueue`].
    #[inline]
    pub const fn new() -> Self {
        Self { raw: RawQueue::new() }
    }

    /// Returns `true` if the [`RetiredQueue`] is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
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
    pub unsafe fn retire(&self, retired: RetiredPtr) {
        // `retired` points to a record, which has layout guarantees regarding field ordering
        // and the record's header is always first
        let header = retired.as_ptr() as *mut Header;
        // store the retired record in the header itself, because it is necessary for later
        // reclamation
        (*header).retired = Some(retired);
        self.raw.push(header);
    }

    #[inline]
    pub unsafe fn reclaim_all_unprotected(&self, protected: &[ProtectedPtr]) {
        // take all retired records from the global queue
        let mut curr = self.raw.take_all();
        // these variables are used to create a simple inline linked list structure
        // all records which can not be reclaimed are put back into this list and are
        // eventually pushed back into the global queue.
        let (mut first, mut last): (*mut Header, *mut Header) = (ptr::null_mut(), ptr::null_mut());

        // iterate all retired records and reclaim all which are no longer protected
        while !curr.is_null() {
            let addr = curr as usize;
            let next = (*curr).next;
            match protected.binary_search_by(|protected| protected.address().cmp(&addr)) {
                // the record is still protected by some hazard pointer
                Ok(_) => {
                    // the next pointer must be zeroed since it may still point at some record
                    // from the global queue
                    (*curr).next = ptr::null_mut();
                    if first.is_null() {
                        first = curr;
                        last = curr;
                    } else {
                        (*last).next = curr;
                        last = curr;
                    }
                }
                // the record can be reclaimed
                Err(_) => (*curr).retired.take().unwrap().reclaim(),
            }

            curr = next;
        }

        // not all records were reclaimed, push all others back into the global queue in bulk.
        if !first.is_null() {
            self.raw.push_many((first, last));
        }
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
                (*curr).retired.take().unwrap().reclaim();
                curr = Header::next(curr);
            }
        }
    }
}
