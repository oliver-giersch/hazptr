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

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &Hazard> {
        Iter {
            // (LIS:1) this `Acquire` load synchronizes with ...
            current: unsafe { self.head.load(Ordering::Acquire).as_ref() },
        }
        .fuse()
    }

    #[inline]
    pub fn acquire_hazard_for(&self, protect: NonNull<()>) -> &Hazard {
        let mut prev = &self.head;
        let mut curr = self.head.load(Ordering::Acquire);

        while let Some(node) = unsafe { curr.as_ref() } {
            if node.hazard.protected.load(Ordering::Relaxed) == FREE {
                let prev = node.hazard.protected.compare_and_swap(
                    FREE,
                    protect.as_ptr(),
                    Ordering::Release,
                );

                if prev == FREE {
                    return &*node.hazard;
                }
            }

            prev = &node.next;
            curr = node.next.load(Ordering::Acquire);
        }

        self.insert_back(prev, protect)
    }

    #[inline]
    fn insert_back(&self, mut tail: &AtomicPtr<HazardNode>, protect: NonNull<()>) -> &Hazard {
        let node = Box::leak(Box::new(HazardNode {
            hazard: CachePadded::new(Hazard::new(protect)),
            next: CachePadded::new(AtomicPtr::default()),
        }));

        loop {
            // (LIS:2) this `Release` CAS ensures the previous allocation (write) is published and
            // synchronizes with all `Acquire` loads on the same `next` field
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
        // `Relaxed` ordering is sufficient here because no other threads have access during `drop`
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

pub struct Iter<'a> {
    current: Option<&'a HazardNode>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a Hazard;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let next = self.current.take();
        if let Some(node) = next {
            // (LIS:3) this `Acquire` load synchronizes with
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
mod test {
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
            .fuse()
            .all(|hazard| hazard.protected.load(Ordering::Relaxed) == ptr.as_ptr()));
    }
}
