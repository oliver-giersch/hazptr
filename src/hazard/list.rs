use std::iter::FusedIterator;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::hazard::{HazardPair, FREE};
use std::ptr::NonNull;

#[derive(Debug, Default)]
pub struct HazardList {
    head: AtomicPtr<HazardPair>,
}

impl HazardList {
    #[inline]
    pub fn new() -> Self {
        Self {
            head: AtomicPtr::default(),
        }
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &HazardPair> {
        unsafe {
            Iter {
                // (x) this `Acquire` load synchronizes with ...
                current: self.head.load(Ordering::Acquire).as_ref(),
            }
            .fuse()
        }
    }

    pub fn acquire_hazard(&self, ptr: NonNull<()>) -> &HazardPair {
        let mut prev = &self.head;
        let mut curr = self.head.load(Ordering::Acquire);

        while let Some(hazard) = unsafe { curr.as_ref() } {
            if hazard.protected.load(Ordering::Relaxed) == FREE {
                if hazard
                    .protected
                    .compare_and_swap(FREE, ptr.as_ptr(), Ordering::Release)
                    == FREE
                {
                    return hazard;
                }
            }

            prev = &hazard.next;
            curr = hazard.next.load(Ordering::Acquire);
        }

        self.insert_back(prev, ptr)
    }

    fn insert_back(&self, mut tail: &AtomicPtr<HazardPair>, ptr: NonNull<()>) -> &HazardPair {
        let hazard = Box::leak(Box::new(HazardPair::new(ptr)));

        loop {
            // this `Release` CAS ensures the previous allocation (write) is published and
            // synchronizes with all `Acquire` loads on the same `next` field
            let res = tail
                .compare_exchange_weak(
                    ptr::null_mut(),
                    hazard,
                    Ordering::Release,
                    Ordering::Relaxed,
                )
                .map_err(|ptr| unsafe { ptr.as_ref() });

            if let Ok(_) = res {
                return hazard;
            } else if let Err(Some(curr)) = res {
                tail = &curr.next;
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

pub struct Iter<'a> {
    current: Option<&'a HazardPair>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a HazardPair;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let next = self.current.take();
        if let Some(hazard) = next {
            // (x) this `Acquire` load synchronizes with
            self.current = unsafe { hazard.next.load(Ordering::Acquire).as_ref() };
        }

        next
    }
}

impl<'a> FusedIterator for Iter<'a> {}

#[cfg(test)]
mod test {
    use std::ptr::{self, NonNull};
    use std::sync::atomic::Ordering;

    use super::HazardList;

    #[test]
    fn insert_one() {
        let list = HazardList::new();
        let hazard = list.acquire_hazard(NonNull::new(0xDEADBEEF as *mut ()).unwrap());
        assert_eq!(hazard.protected.load(Ordering::Relaxed), 1 as *mut ());
        assert_eq!(hazard.next.load(Ordering::Relaxed), ptr::null_mut());
    }

    #[test]
    fn iter() {
        let ptr = NonNull::new(0xDEADBEEF as *mut ()).unwrap();

        let list = HazardList::new();
        let _ = list.acquire_hazard(ptr);
        let _ = list.acquire_hazard(ptr);
        let _ = list.acquire_hazard(ptr);

        assert!(list
            .iter()
            .fuse()
            .all(|hazard| hazard.protected.load(Ordering::Relaxed) == ptr.as_ptr()));
    }
}
