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

    #[inline]
    pub unsafe fn reclaim_all_unprotected(&self, scan_cache: &[ProtectedPtr]) {
        // take all retired records from the global queue
        let mut curr = self.raw.take_all();
        // these pointers are used to form a simple inline singly linked list structure of all
        // records which can NOT yet be reclaimed and have to be pushed back into the global queue.
        let (mut first, mut last): (*mut Header, *mut Header) = (ptr::null_mut(), ptr::null_mut());

        // iterate over retired records and reclaim all which are no longer protected
        while !curr.is_null() {
            // all retired records point at the entire record (including the header), whereas all
            // hazard pointers point at data, so the offset needs to be calculated before comparing
            let data_addr = (curr as usize) + (*curr).retired.as_ref().unwrap().offset_data();

            // `(*curr).next` must be read HERE because `curr` may be de-allocated in the next step
            let next = Header::next(curr);
            match scan_cache.binary_search_by(|protected| protected.address().cmp(&data_addr)) {
                // the record is still protected by some hazard pointer
                Ok(_) => {
                    if !first.is_null() {
                        // append curr to tail (last)
                        Header::set_next(last, curr);
                        last = curr;
                    } else {
                        // first entry, set first and last
                        first = curr;
                        last = curr;
                    }
                }
                // the record can be reclaimed
                Err(_) => (*curr).retired.take().unwrap().reclaim(),
            }

            curr = next;
        }

        // if not all records were reclaimed, push all others back into the global queue in bulk.
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
