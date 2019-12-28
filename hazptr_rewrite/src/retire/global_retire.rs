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

use conquer_reclaim::RawRetired;

use crate::global::Global;
use crate::hazard::ProtectedPtr;
use crate::queue::{RawNode, RawQueue};
use crate::retire::RetireStrategy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalRetire
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A handle type for the global retire strategy.
///
/// It is a ZST because this strategy does not require any additional
/// thread-local state.
#[derive(Debug, Default)]
pub struct GlobalRetire;

/********** impl RetireStrategy *******************************************************************/

impl RetireStrategy for GlobalRetire {
    type Global = RetiredQueue;
    type Header = Header;

    #[inline]
    fn new(_: &Global<Self>) -> Self {
        Self
    }

    #[inline]
    fn drop(self, _: &Global<Self>) {}

    #[inline]
    fn no_retired_records(&self, global: &Global<Self>) -> bool {
        global.state.raw.is_empty()
    }

    #[inline]
    unsafe fn reclaim_all_unprotected(
        &mut self,
        global: &Global<Self>,
        protected: &[ProtectedPtr],
    ) {
        // take all retired records from the global queue
        let mut curr = global.state.raw.take_all();
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
            global.state.raw.push_many((first, last));
        }
    }

    #[inline]
    unsafe fn retire(&mut self, global: &Global<Self>, retired: RawRetired) {
        // retired points to a record, which have layout guarantees regarding field ordering
        // and the record's header is always first
        let header = retired.as_ptr() as *mut () as *mut Header;
        // store the retired record in the header itself, because it is necessary for later
        // reclamation
        (*header).retired = Some(retired);
        global.state.raw.push(header);
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// RetiredQueue
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Default)]
pub struct RetiredQueue {
    raw: RawQueue<Header>,
}

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
    next: *mut Self,
    retired: Option<RawRetired>,
}

/********** impl Sync *****************************************************************************/

unsafe impl Sync for Header {}

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

    unsafe fn set_next(node: *mut Self, next: *mut Self) {
        (*node).next = next;
    }
}
