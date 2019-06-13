//! Caching for retired records until they can be safely dropped and
//! deallocated.
//!
//! # Retired and Retired Bags
//!
//! Pointers to retired records are stored in `Retired` structs. These contain
//! fat pointers, so they do maintain dynamic type information, of which only
//! the concrete `Drop` implementation is actually required.
//! They are stored in `RetiredBag` structs and removed (i.e. dropped and
//! deallocated) only when no thread has an active hazard pointer protecting the
//! same memory address of the reclaimed record.
//!
//! # Abandoned Bags
//!
//! When a thread exits it attempts to reclaim all of its retired records.
//! However, it is possible that some records may not be reclaimed if other
//! threads still have active hazard pointers to these records.
//! In this case, the exiting thread's retired bag with the remaining
//! un-reclaimed records is abandoned, meaning it is stored in a special global
//! queue.
//! Other threads will occasionally attempt to adopt such abandoned records, at
//! which point it becomes the adopting thread's responsibility to reclaim these
//! records.

use core::mem;
use core::ptr::{self, NonNull};
use core::sync::atomic::{
    AtomicPtr,
    Ordering::{Acquire, Relaxed, Release},
};

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec::Vec};

pub(crate) type Retired = reclaim::Retired<crate::HP>;

////////////////////////////////////////////////////////////////////////////////////////////////////
// RetiredBag
////////////////////////////////////////////////////////////////////////////////////////////////////

/// List for caching reclaimed records before they can be finally
/// dropped/deallocated.
///
/// This type also functions as potential list node for the global list of
/// abandoned bags.
/// The internal cache uses a `Vec`, which will have to be reallocated if too
/// many retired records are cached at any time.
#[derive(Debug)]
pub(crate) struct RetiredBag {
    pub inner: Vec<Retired>,
    next: Option<NonNull<RetiredBag>>,
}

impl RetiredBag {
    const DEFAULT_CAPACITY: usize = 256;

    /// Creates a new `RetiredBag` with default capacity for retired records.
    #[inline]
    pub fn new() -> Self {
        Self { inner: Vec::with_capacity(Self::DEFAULT_CAPACITY), next: None }
    }

    /// Merges `self` with the given other `Vec`, which is then dropped
    /// (deallocated).
    ///
    /// If the `other` bag has substantially higher (free) capacity than `self`,
    /// both vectors are swapped before merging.
    /// By keeping the larger vector in this case and dropping the smaller one,
    /// instead, it could be possible to avoid/defer future reallocations, when
    /// more records are retired.
    #[inline]
    pub fn merge(&mut self, mut other: Vec<Retired>) {
        if (other.capacity() - other.len()) > self.inner.capacity() {
            mem::swap(&mut self.inner, &mut other);
        }

        self.inner.append(&mut other);
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// AbandonedBags
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Concurrent queue containing all retired bags abandoned by exited threads
#[derive(Debug)]
pub(crate) struct AbandonedBags {
    head: AtomicPtr<RetiredBag>,
}

impl AbandonedBags {
    /// Creates a new (empty) queue.
    #[inline]
    pub const fn new() -> Self {
        Self { head: AtomicPtr::new(ptr::null_mut()) }
    }

    /// Adds a new abandoned retired bag to the front of the queue.
    #[inline]
    pub fn push(&self, abandoned: Box<RetiredBag>) {
        let leaked: &mut RetiredBag = Box::leak(abandoned); // makes CLion happy

        loop {
            let head = self.head.load(Relaxed);
            leaked.next = NonNull::new(head);

            // (RET:1) this `Release` CAS synchronizes-with the `Acquire` swap in (RET:2)
            if self.head.compare_exchange_weak(head, leaked, Release, Relaxed).is_ok() {
                return;
            }
        }
    }

    /// Takes the entire content of the queue and merges the retired records of
    /// all retired bags into one.
    #[inline]
    pub fn take_and_merge(&self) -> Option<Box<RetiredBag>> {
        // probe first in order to avoid the swap if the stack is empty
        if self.head.load(Relaxed).is_null() {
            return None;
        }

        // (RET:2) this `Acquire` swap synchronizes-with the `Release` CAS in (RET:1)
        let queue = unsafe { self.head.swap(ptr::null_mut(), Acquire).as_mut() };
        queue.map(|bag| {
            let mut boxed = unsafe { Box::from_raw(bag) };

            let mut curr = boxed.next;
            while let Some(ptr) = curr {
                let RetiredBag { inner: bag, next } = unsafe { *Box::from_raw(ptr.as_ptr()) };
                boxed.merge(bag);
                curr = next;
            }

            boxed
        })
    }
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{AbandonedBags, Retired, RetiredBag};

    struct DropCount<'a>(&'a AtomicUsize);
    impl Drop for DropCount<'_> {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn abandoned_bags() {
        let count = AtomicUsize::new(0);

        let mut bag1 = Box::new(RetiredBag::new());

        let rec1 = NonNull::from(Box::leak(Box::new(1)));
        let rec2 = NonNull::from(Box::leak(Box::new(2.2)));
        let rec3 = NonNull::from(Box::leak(Box::new(String::from("String"))));

        bag1.inner.push(unsafe { Retired::new_unchecked(rec1) });
        bag1.inner.push(unsafe { Retired::new_unchecked(rec2) });
        bag1.inner.push(unsafe { Retired::new_unchecked(rec3) });

        let mut bag2 = Box::new(RetiredBag::new());

        let rec4 = NonNull::from(Box::leak(Box::new(vec![1, 2, 3, 4])));
        let rec5 = NonNull::from(Box::leak(Box::new("slice")));

        bag2.inner.push(unsafe { Retired::new_unchecked(rec4) });
        bag2.inner.push(unsafe { Retired::new_unchecked(rec5) });

        let mut bag3 = Box::new(RetiredBag::new());

        let rec6 = NonNull::from(Box::leak(Box::new(DropCount(&count))));
        let rec7 = NonNull::from(Box::leak(Box::new(DropCount(&count))));

        bag3.inner.push(unsafe { Retired::new_unchecked(rec6) });
        bag3.inner.push(unsafe { Retired::new_unchecked(rec7) });

        let abandoned = AbandonedBags::new();
        abandoned.push(bag1);
        abandoned.push(bag2);
        abandoned.push(bag3);

        let merged = abandoned.take_and_merge().unwrap();
        assert_eq!(merged.inner.len(), 7);
        assert_eq!(RetiredBag::DEFAULT_CAPACITY, merged.inner.capacity());
    }
}