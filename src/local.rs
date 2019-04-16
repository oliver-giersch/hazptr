//! Thread local state and caches for reserving hazard pointers or storing retired records.

use core::cell::UnsafeCell;
use core::mem::ManuallyDrop;
use core::ptr::{self, NonNull};
use core::sync::atomic::{self, Ordering};

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec::Vec};

use arrayvec::ArrayVec;

use crate::global::Global;
use crate::hazard::{Hazard, HazardPtr, Protected};
use crate::retired::{Retired, RetiredBag};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Local
////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(all(
    not(feature = "maximum-reclamation-freq"),
    not(feature = "reduced-reclamation-freq")
))]
const SCAN_THRESHOLD: u32 = 100;
#[cfg(feature = "reduced-reclamation-freq")]
const SCAN_THRESHOLD: u32 = 200;
#[cfg(feature = "maximum-reclamation-freq")]
const SCAN_THRESHOLD: u32 = 1;

const HAZARD_CACHE: usize = 16;
const SCAN_CACHE: usize = 128;

/// Container for all thread local data required for reclamation with hazard pointers.
pub struct Local(UnsafeCell<LocalInner>);

impl Local {
    /// Creates a new container for the thread local state.
    #[inline]
    pub fn new(global: &'static Global) -> Self {
        Self(UnsafeCell::new(LocalInner {
            global,
            ops_count: 0,
            hazard_cache: ArrayVec::new(),
            scan_cache: Vec::with_capacity(SCAN_CACHE),
            retired_bag: match global.try_adopt_abandoned_records() {
                Some(boxed) => ManuallyDrop::new(boxed),
                None => ManuallyDrop::new(Box::new(RetiredBag::new())),
            },
        }))
    }

    /// Attempts to take a reserved hazard from the thread local cache if there are any.
    #[inline]
    pub(crate) fn acquire_hazard_for(&self, protect: NonNull<()>) -> &'static Hazard {
        let local = unsafe { &mut *self.0.get() };
        if let Some(hazard) = local.hazard_cache.pop() {
            // this operation issues a full `SeqCst` memory fence
            hazard.set_protected(protect);

            hazard
        } else {
            local.global.acquire_hazard_for(protect)
        }
    }

    /// Attempts to cache `hazard` in the thread local storage.
    ///
    /// # Errors
    ///
    /// The recycle can fail if the thread local hazard cache is at capacity.
    #[inline]
    pub(crate) fn try_recycle_hazard(&self, hazard: &'static Hazard) -> Result<(), RecycleErr> {
        match unsafe { &mut *self.0.get() }.hazard_cache.try_push(hazard) {
            Ok(_) => {
                // (LOC:1) this `Release` store synchronizes-with any `Acquire` load on the
                // `protected` field of the same hazard pointer
                hazard.set_reserved(Ordering::Release);
                Ok(())
            }
            Err(_) => Err(RecycleErr::Capacity),
        }
    }

    /// Retires a record and increases the operations count.
    ///
    /// If the operations count reaches a threshold, a scan is triggered which reclaims all records
    /// than can be safely reclaimed and resets the operations count. Beforehand, the thread
    /// attempts to adopt all globally abandoned records.
    #[inline]
    pub(crate) fn retire_record(&self, record: Retired) {
        let local = unsafe { &mut *self.0.get() };
        local.retired_bag.inner.push(record);
        #[cfg(not(feature = "count-release"))]
        local.increase_ops_count();
    }

    /// Increases the thread local operations count and triggers a scan if the threshold is reached.
    #[inline]
    pub(crate) fn increase_ops_count(&self) {
        unsafe { &mut *self.0.get() }.increase_ops_count();
    }

    #[cfg(test)]
    #[inline]
    pub(crate) fn cached_hazards_count(&self) -> usize {
        unsafe { &*self.0.get() }.hazard_cache.len()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalInner
////////////////////////////////////////////////////////////////////////////////////////////////////

struct LocalInner {
    global: &'static Global,
    ops_count: u32,
    hazard_cache: ArrayVec<[&'static Hazard; HAZARD_CACHE]>,
    scan_cache: Vec<Protected>,
    retired_bag: ManuallyDrop<Box<RetiredBag>>,
}

impl LocalInner {
    /// Increases the operations count and triggers a scan if the threshold is reached.
    #[inline]
    fn increase_ops_count(&mut self) {
        self.ops_count += 1;

        if self.ops_count == SCAN_THRESHOLD {
            // try to adopt and merge any (global) abandoned retired bags
            if let Some(abandoned_bag) = self.global.try_adopt_abandoned_records() {
                self.retired_bag.merge(abandoned_bag.inner);
            }

            let _ = self.scan_hazards();
            self.ops_count = 0;
        }
    }

    /// Reclaims all locally retired records that are unprotected and returns the number of
    /// reclaimed records.
    #[inline]
    fn scan_hazards(&mut self) -> usize {
        let len = self.retired_bag.inner.len();
        if len == 0 {
            return 0;
        }

        self.global.collect_protected_hazards(&mut self.scan_cache);

        let scan_cache = &mut self.scan_cache;
        scan_cache.sort_unstable();

        self.retired_bag.inner.retain(move |retired| {
            scan_cache
                .binary_search_by(|protected| protected.address().cmp(&retired.address()))
                .is_ok()
        });

        len - self.retired_bag.inner.len()
    }
}

impl Drop for LocalInner {
    #[inline]
    fn drop(&mut self) {
        for hazard in &self.hazard_cache {
            hazard.set_free(Ordering::Relaxed);
        }

        // (LOC:2) this `Release` fence synchronizes-with the `SeqCst` fence (GLO:1)
        atomic::fence(Ordering::Release);

        self.scan_hazards();
        // this is safe because the `retired_bag` field is neither accessed afterwards nor dropped
        let bag = unsafe { ptr::read(&*self.retired_bag) };

        if !bag.inner.is_empty() {
            self.global.abandon_retired_bag(bag);
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalAccess
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A trait for abstracting over different means of accessing thread local state
pub trait LocalAccess
where
    Self: Copy + Sized,
{
    /// Acquires a hazard either from thread local storage or globally and wraps it in a
    /// [`HazardPtr`](crate::hazard::HazardPtr).
    fn acquire_hazard_for(access: Self, protect: NonNull<()>) -> HazardPtr<Self>;

    /// Attempts to recycle `hazard` in the thread local cache for hazards reserved for the current
    /// thread.
    ///
    /// # Errors
    ///
    /// This operation can fail in two circumstances:
    ///
    /// - the thread local cache is at capacity ([`RecycleErr::Capacity`](RecycleErr::Capacity))
    /// - access to the thread local state fails ([`RecycleErr::Access`](RecycleErr::Access))
    fn try_recycle_hazard(access: Self, hazard: &'static Hazard) -> Result<(), RecycleErr>;

    /// Increase the internal count of a threads operations counting towards the threshold for
    /// initiating a new attempt for reclaiming all retired records.
    fn increase_ops_count(access: Self);
}

impl<'a> LocalAccess for &'a Local {
    #[inline]
    fn acquire_hazard_for(access: Self, protect: NonNull<()>) -> HazardPtr<Self> {
        HazardPtr::new(access.acquire_hazard_for(protect), access)
    }

    #[inline]
    fn try_recycle_hazard(access: Self, hazard: &'static Hazard) -> Result<(), RecycleErr> {
        access.try_recycle_hazard(hazard)
    }

    #[inline]
    fn increase_ops_count(access: Self) {
        access.increase_ops_count();
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// RecycleErr
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Error type for thread local recycle operations.
#[derive(Debug)]
pub enum RecycleErr {
    Access,
    Capacity,
}

#[cfg(test)]
mod tests {
    /*use std::mem;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    use crate::acquire_hazard_for;
    use crate::retired::Retired;

    use super::*;

    struct DropCount<'a>(&'a AtomicUsize);
    impl Drop for DropCount<'_> {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn acquire_local() {
        assert!(acquire_hazard().is_none());
        let ptr = NonNull::from(&1);

        let _hazards: Box<[HazardPtr]> = (0..HAZARD_CACHE)
            .map(|_| acquire_hazard_for(ptr.cast()))
            .collect();
        mem::drop(_hazards);

        // thread local hazard cache is full
        LOCAL.with(|cell| {
            let local = unsafe { &*cell.get() };
            assert_eq!(0, local.ops_count);
            assert_eq!(HAZARD_CACHE, local.hazard_cache.len());
            assert_eq!(SCAN_CACHE, local.scan_cache.capacity());
            assert_eq!(0, local.scan_cache.len());
        });

        let _hazards: Box<[HazardPtr]> = (0..HAZARD_CACHE)
            .map(|_| acquire_hazard_for(ptr.cast()))
            .collect();

        // thread local hazard cache is empty
        LOCAL.with(|cell| {
            let local = unsafe { &*cell.get() };
            assert_eq!(0, local.ops_count);
            assert_eq!(0, local.hazard_cache.len());
            assert_eq!(SCAN_CACHE, local.scan_cache.capacity());
            assert_eq!(0, local.scan_cache.len());
        });
    }

    #[test]
    fn retire() {
        const THRESHOLD: usize = SCAN_THRESHOLD as usize;

        let count = AtomicUsize::new(0);

        // retire THRESHOLD - 1 records
        for _ in 0..THRESHOLD - 1 {
            let retired = unsafe {
                Retired::new_unchecked(NonNull::from(Box::leak(Box::new(DropCount(&count)))))
            };
            retire_record(retired);
        }

        LOCAL.with(|cell| {
            let local = unsafe { &*cell.get() };
            assert_eq!(THRESHOLD - 1, local.ops_count as usize);
            assert_eq!(THRESHOLD - 1, local.retired_bag.inner.len());
        });

        assert_eq!(0, count.load(Ordering::Relaxed));

        // retire another record, trigger scan which deallocates records
        let retired = unsafe {
            Retired::new_unchecked(NonNull::from(Box::leak(Box::new(DropCount(&count)))))
        };
        retire_record(retired);

        LOCAL.with(|cell| {
            let local = unsafe { &*cell.get() };
            assert_eq!(0, local.ops_count as usize);
            assert_eq!(0, local.retired_bag.inner.len());
        });

        assert_eq!(THRESHOLD, count.load(Ordering::Relaxed));
    }

    #[test]
    fn drop() {
        const BELOW_THRESHOLD: usize = SCAN_THRESHOLD as usize / 2;
        static COUNT: AtomicUsize = AtomicUsize::new(0);

        let


        let handle = thread::spawn(|| {
            for _ in 0..BELOW_THRESHOLD {
                let retired = unsafe {
                    Retired::new_unchecked(NonNull::from(Box::leak(Box::new(DropCount(&COUNT)))))
                };
                retire_record(retired);
            }
        });

        // thread is joined, LOCAL is dropped and all retired records are reclaimed
        handle.join().unwrap();
        assert_eq!(BELOW_THRESHOLD, COUNT.load(Ordering::Relaxed));
    }*/
}
