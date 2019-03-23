//! Concurrent linked list implementation for globally storing all allocated hazard pointers.
//!
//! A thread requesting a hazard pointer first traverses this list and searches for an already
//! allocated one that is not currently in use. If there is none, the list allocates a new one,
//! appends it to the end of the list and returns a reference (`&'static Hazard`) to it.
//! Once allocated, hazard pointers are never deallocated again during the lifetime of the program
//! (i.e. they have `'static` lifetime). When a thread does no longer need an acquired hazard
//! pointer, marks it as no longer in use, which allows other threads to acquire it during the list
//! traversal instead of allocating a new one. Additionally, each thread maintains a small cache of
//! previously acquired hazard pointers, which are specifically reserved for use by that thread.
//!
//! # Synchronization
//!
//! ```no_run
//! struct Node {
//!     protected: #[repr(align(64))] AtomicPtr<()>,
//!     next:      #[repr(align(64))] AtomicPtr<HazardNode>,
//! }
//! ```
//!
//! Above is an approximate and simplified description of a node in the global linked list of hazard
//! pointers. Both fields of this struct are aligned to the size of a cache-line in order to prevent
//! false sharing. This is desirable, since the `next` field is effectively constant once a node is
//! inserted and is no longer at the tail, while the `protected` field can be frequently written to.
//!
//! All atomic operations on the `next` field can be synchronized using Acquire-Release semantics,
//! since all threads are required to synchronize through the **same** variable (i.e. the current
//! tail of the list).
//! All stores to the `protected` field that mark a specific pointer as protected from reclamation,
//! however, **must** establish a total order and thus require sequential consistency (HAZ:2 and
//! LIS:3P). Similarly, the loads on that field made during a scan of all active hazard pointers
//! must also be sequentially consistent (GLO:1). Otherwise, a thread scanning the global list of
//! hazard pointers might not see a consistent view of all protected pointers, since stores to the
//! various `protected` fields are all independent writes. Consequently, a thread might go ahead and
//! deallocate a retired record for which a hazard pointer has previously been successfully
//! acquired but the corresponding store has not yet become visible to the reclaiming thread,
//! potentially leading to a critical **use after free** error.
//! All stores that write a sentinel value (i.e. `0x0` for `FREE` and `0x1` for `RESERVED`) to a
//! `protected` field, on the other hand, do not require such strict ordering constraints. If such a
//! store is delayed and not visible during a thread's scan prior to reclamation the worst-case
//! outcome is a record not being reclaimed that would actually be a valid candidate for
//! reclamation.

use std::iter::FusedIterator;
use std::mem;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicPtr, Ordering};

use reclaim::align::CachePadded;

use crate::hazard::{Hazard, FREE};

/// Linked list for hazard pointers
#[derive(Debug, Default)]
pub struct HazardList {
    head: AtomicPtr<HazardNode>,
}

impl HazardList {
    /// Creates a new empty list.
    #[inline]
    pub fn new() -> Self {
        Self {
            head: AtomicPtr::default(),
        }
    }

    /// Creates a (fused) iterator for the list.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &Hazard> {
        Iter {
            // (LIS:1) this `Acquire` load synchronizes-with the `Release` CAS (LIS:5)
            current: unsafe { self.head.load(Ordering::Acquire).as_ref() },
        }
        .fuse()
    }

    /// Acquires an already inserted and inactive hazard pointer or allocates a new one at the tail
    /// and returns a reference to it.
    #[inline]
    pub fn acquire_hazard_for(&self, protect: NonNull<()>) -> &Hazard {
        let mut prev = &self.head;
        // (LIS:2) this `Acquire` load synchronizes-with the `Release` CAS (LIS:5)
        let mut curr = self.head.load(Ordering::Acquire);

        while let Some(node) = unsafe { curr.as_ref() } {
            if node.hazard.protected.load(Ordering::Relaxed) == FREE {
                // (LIS:3P) this `SeqCst` CAS establishes a total order with the `SeqCst` store
                // (HAZ:1) and the `SeqCst` fence (GLO:1)
                let prev = node.hazard.protected.compare_and_swap(
                    FREE,
                    protect.as_ptr(),
                    Ordering::SeqCst,
                );

                if prev == FREE {
                    return &*node.hazard;
                }
            }

            prev = &node.next;
            // (LIS:4) this `Acquire` load synchronizes-with the `Release` CAS (LIS:5)
            curr = node.next.load(Ordering::Acquire);
        }

        self.insert_back(prev, protect)
    }

    /// Allocates and inserts a new node (hazard pointer) at the tail of the list.
    #[inline]
    fn insert_back(&self, mut tail: &AtomicPtr<HazardNode>, protect: NonNull<()>) -> &Hazard {
        let node = Box::leak(Box::new(HazardNode {
            hazard: CachePadded::new(Hazard::new(protect)),
            next: CachePadded::new(AtomicPtr::default()),
        }));

        loop {
            // (LIS:5) this `Release` CAS synchronizes-with the `Acquire` loads on the same `head`
            // or `next` field such as (LIS:1), (LIS:2), (LIS:4) and (LIS:6)
            let res = tail
                .compare_exchange_weak(ptr::null_mut(), node, Ordering::Release, Ordering::Relaxed)
                .map_err(|ptr| unsafe { ptr.as_ref() });

            if let Err(Some(curr)) = res {
                tail = &curr.next;
            } else if res.is_ok() {
                return &*node.hazard;
            }
        }
    }
}

impl Drop for HazardList {
    #[inline]
    fn drop(&mut self) {
        // `Relaxed` ordering is sufficient because `&mut self` ensures no other threads have access
        let mut curr = self.head.load(Ordering::Relaxed);
        while let Some(hazard) = unsafe { curr.as_mut() } {
            curr = hazard.next.load(Ordering::Relaxed);
            mem::drop(unsafe { Box::from_raw(hazard) });
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
/// Iter
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Iterator for a `HazardList`
pub struct Iter<'a> {
    current: Option<&'a HazardNode>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a Hazard;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let next = self.current.take();
        if let Some(node) = next {
            // (LIS:6) this `Acquire` load synchronizes with the `Release` CAS (LIS:5)
            self.current = unsafe { node.next.load(Ordering::Acquire).as_ref() };
        }

        next.map(|node| &*node.hazard)
    }
}

impl<'a> FusedIterator for Iter<'a> {}

////////////////////////////////////////////////////////////////////////////////////////////////////
/// HazardNode
////////////////////////////////////////////////////////////////////////////////////////////////////

struct HazardNode {
    hazard: CachePadded<Hazard>,
    next: CachePadded<AtomicPtr<HazardNode>>,
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;
    use std::sync::atomic::Ordering;

    use super::HazardList;

    #[test]
    fn insert_one() {
        let ptr = NonNull::new(0xDEADBEEF as *mut ()).unwrap();

        let list = HazardList::new();
        let hazard = list.acquire_hazard_for(ptr);
        assert_eq!(
            hazard.protected.load(Ordering::Relaxed),
            0xDEADBEEF as *mut ()
        );
    }

    #[test]
    fn iter() {
        let ptr = NonNull::new(0xDEADBEEF as *mut ()).unwrap();

        let list = HazardList::new();
        let _ = list.acquire_hazard_for(ptr);
        let _ = list.acquire_hazard_for(ptr);
        let _ = list.acquire_hazard_for(ptr);

        assert!(list
            .iter()
            .all(|hazard| hazard.protected.load(Ordering::Relaxed) == ptr.as_ptr()));
    }
}
