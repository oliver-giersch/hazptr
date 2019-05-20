//! Concurrent linked list implementation for globally storing all allocated
//! hazard pointers.
//!
//! A thread requesting a hazard pointer first traverses this list and searches
//! for an already allocated one that is not currently in use.
//! If there is none, the list allocates a new one, appends it to the end of the
//! list and returns a reference (`&'static Hazard`) to it.
//! Once allocated, hazard pointers are never de-allocated again during the
//! lifetime of the program (i.e. they have `'static` lifetime).
//! When a thread does no longer need an acquired hazard pointer, marks it as
//! no longer in use, which allows other threads to acquire it during the list
//! traversal instead of having to allocate a new one.
//! Additionally, each thread maintains a small cache of previously acquired
//! hazard pointers, which are specifically reserved for use by that thread.
//!
//! # Synchronization
//!
//! ```ignore
//! struct Node {
//!     protected: #[repr(align(64))] AtomicPtr<()>,
//!     next:      #[repr(align(64))] AtomicPtr<HazardNode>,
//! }
//! ```
//!
//! Above is an approximate and simplified description of a node in the global
//! linked list of hazard pointers.
//! Both fields of this struct are aligned to the size of a cache-line in order
//! to prevent false sharing.
//! This is desirable, since the `next` field is effectively constant once a
//! node is inserted and is no longer at the tail, while the `protected` field
//! can be frequently written to.
//!
//! All atomic operations on the `next` field can be synchronized using
//! acquire-release semantics, since all threads are required to synchronize
//! through the **same** variable (i.e. the current tail of the list).
//! All stores to the `protected` field that mark a specific pointer as
//! protected from reclamation, however, **must** establish a total order and
//! thus require sequential consistency (HAZ:2 and LIS:3P).
//! Similarly, the loads on that field made during a scan of all active hazard
//! pointers must also be sequentially consistent (GLO:1).
//! Otherwise, a thread scanning the global list of hazard pointers might not
//! see a consistent view of all protected pointers, since stores to the various
//! `protected` fields are all independent writes.
//! Consequently, a thread might go ahead and deallocate a retired record for
//! which a hazard pointer has previously been successfully acquired but the
//! corresponding store has not yet become visible to the reclaiming thread,
//! potentially leading to a critical **use after free** error.
//! All stores that write a sentinel value (e.g. `0x0` for `FREE` and `0x1` for
//! `RESERVED`) to a `protected` field, on the other hand, do not require such
//! strict ordering constraints.
//! If such a store is delayed and not visible during a thread's scan prior to
//! reclamation the worst-case outcome is a record not being reclaimed that
//! would actually be a valid candidate for reclamation.

use core::iter::FusedIterator;
use core::marker::PhantomData;
use core::mem;
use core::ptr::NonNull;
use core::sync::atomic::{
    self,
    Ordering::{self, Acquire, Relaxed},
};

#[cfg(not(feature = "std"))]
use alloc::boxed::Box;

use reclaim::align::CacheAligned;
use reclaim::leak::{Owned, Shared};
use reclaim::MarkedPointer;

type Atomic<T> = reclaim::leak::Atomic<T, reclaim::typenum::U0>;
type Unprotected<T> = reclaim::leak::Unprotected<T, reclaim::typenum::U0>;

use crate::hazard::{Hazard, FREE};
use crate::sanitize::{RELEASE_CAS_FAILURE, RELEASE_CAS_SUCCESS};

////////////////////////////////////////////////////////////////////////////////////////////////////
// HazardList
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Linked list for storing hazard pointers
#[derive(Debug, Default)]
pub struct HazardList {
    head: Atomic<HazardNode>,
}

impl HazardList {
    /// Creates a new empty list.
    #[inline]
    pub const fn new() -> Self {
        Self { head: Atomic::null() }
    }

    /// Creates a (fused) iterator for the list.
    #[inline]
    pub fn iter(&self) -> Iter {
        Iter {
            // (LIS:1) this `Acquire` load synchronizes-with the `Release` CAS (LIS:5)
            current: self.head.load_unprotected(Acquire),
            _marker: PhantomData,
        }
    }

    /// Acquires an already inserted and inactive hazard pointer or allocates a new one at the tail
    /// and returns a reference to it.
    #[inline]
    pub fn get_hazard(&self, protect: NonNull<()>) -> &Hazard {
        let mut prev = &self.head;
        // (LIS:2) this `Acquire` load synchronizes-with the `Release` CAS (LIS:5)
        let mut curr = prev.load_unprotected(Acquire);

        while let Some(node) =
            curr.map(|unprotected| unsafe { &*unprotected.as_marked_ptr().decompose_ptr() })
        {
            if node.hazard.protected.load(Relaxed) == FREE {
                // (LIS:3P) this `SeqCst` CAS synchronizes-with the `SeqCst` fence (GLO:1)
                let prev = node.hazard.protected.compare_and_swap(
                    FREE,
                    protect.as_ptr(),
                    Ordering::SeqCst,
                );

                if prev == FREE {
                    return &*node.hazard;
                }
            }

            prev = &*node.next;
            // (LIS:4) this `Acquire` load synchronizes-with the `Release` CAS (LIS:5)
            curr = node.next.load_unprotected(Acquire);
        }

        self.insert_back(prev, protect)
    }

    /// Allocates and inserts a new node (hazard pointer) at the tail of the list.
    #[inline]
    fn insert_back(&self, mut tail: &Atomic<HazardNode>, protect: NonNull<()>) -> &Hazard {
        let node = Owned::leak_unprotected(Owned::new(HazardNode {
            hazard: CacheAligned(Hazard::new(protect)),
            next: CacheAligned(Atomic::null()),
        }));

        let hazard = unsafe { &node.deref_unprotected().hazard };

        loop {
            // TODO: check comment
            // (LIS:5) this `Release` CAS synchronizes-with the `Acquire` loads on the same `head`
            // or `next` field such as (LIS:1), (LIS:2), (LIS:4) and (LIS:7)
            match tail.compare_exchange_weak(
                Shared::none(),
                node,
                RELEASE_CAS_SUCCESS,
                RELEASE_CAS_FAILURE,
            ) {
                Ok(_) => return &*hazard,
                Err(fail) => {
                    // (LIS:6) this `Acquire` fence synchronizes-with the `Release` CAS (LIS:5)
                    atomic::fence(Ordering::Acquire);

                    // this is safe because nodes are never retired or reclaimed
                    if let Some(curr_tail) = unsafe { fail.loaded.as_ref() } {
                        tail = &curr_tail.next;
                    }
                }
            }
        }
    }
}

impl Drop for HazardList {
    #[inline]
    fn drop(&mut self) {
        let mut curr = self.head.take();
        while let Some(mut owned) = curr {
            curr = owned.next.take();
            mem::drop(owned);
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Iter
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Iterator for a `HazardList`
pub struct Iter<'a> {
    current: Option<Unprotected<HazardNode>>,
    _marker: PhantomData<&'a HazardList>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a Hazard;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.current.take().map(|unprotected| {
            let node = unsafe { &*unprotected.as_marked_ptr().decompose_ptr() };
            self.current = node.next.load_unprotected(Acquire);
            &*node.hazard
        })
    }
}

impl<'a> FusedIterator for Iter<'a> {}

////////////////////////////////////////////////////////////////////////////////////////////////////
// HazardNode
////////////////////////////////////////////////////////////////////////////////////////////////////

struct HazardNode {
    hazard: CacheAligned<Hazard>,
    next: CacheAligned<Atomic<HazardNode>>,
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;
    use std::sync::atomic::Ordering;

    use super::HazardList;

    #[test]
    fn insert_one() {
        let ptr = NonNull::new(0xDEAD_BEEF as *mut ()).unwrap();

        let list = HazardList::new();
        let hazard = list.get_hazard(ptr);
        assert_eq!(hazard.protected.load(Ordering::Relaxed), 0xDEAD_BEEF as *mut ());
    }

    #[test]
    fn iter() {
        let ptr = NonNull::new(0xDEAD_BEEF as *mut ()).unwrap();

        let list = HazardList::new();
        let _ = list.get_hazard(ptr);
        let _ = list.get_hazard(ptr);
        let _ = list.get_hazard(ptr);

        assert!(list
            .iter()
            .fuse()
            .all(|hazard| hazard.protected.load(Ordering::Relaxed) == ptr.as_ptr()));
    }
}
