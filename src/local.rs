//! Thread local state and caches for reserving hazard pointers or storing retired records.

use core::cell::UnsafeCell;
use core::mem::ManuallyDrop;
use core::ptr::{self, NonNull};
use core::sync::atomic::{self, Ordering};

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec::Vec};

use arrayvec::{ArrayVec, CapacityError};

use crate::global::Global;
use crate::hazard::{Hazard, HazardPtr, Protected};
use crate::retired::{Retired, RetiredBag};

////////////////////////////////////////////////////////////////////////////////////////////////////
// constants
////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(all(not(feature = "maximum-reclamation-freq"), not(feature = "reduced-reclamation-freq")))]
const SCAN_THRESHOLD: u32 = 100;
#[cfg(feature = "reduced-reclamation-freq")]
const SCAN_THRESHOLD: u32 = 200;
#[cfg(feature = "maximum-reclamation-freq")]
const SCAN_THRESHOLD: u32 = 1;

const HAZARD_CACHE: usize = 16;
const SCAN_CACHE: usize = 128;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Local
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Container for all thread local data required for reclamation with hazard pointers.
#[derive(Debug)]
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
    pub(crate) fn get_hazard(&self, protect: NonNull<()>) -> &'static Hazard {
        let local = unsafe { &mut *self.0.get() };
        if let Some(hazard) = local.hazard_cache.pop() {
            // (LOC:1) this `SeqCst` store synchronizes-with the `SeqCst` fence (GLO:1).
            hazard.set_protected(protect, Ordering::SeqCst);

            hazard
        } else {
            local.global.get_hazard(protect)
        }
    }

    /// Attempts to cache `hazard` in the thread local storage.
    ///
    /// # Errors
    ///
    /// The operation can fail if the thread local hazard cache is at maximum capacity.
    #[inline]
    pub(crate) fn try_recycle_hazard(&self, hazard: &'static Hazard) -> Result<(), RecycleErr> {
        unsafe { &mut *self.0.get() }.hazard_cache.try_push(hazard)?;

        // (LOC:2) this `Release` store synchronizes-with any `Acquire` load on the
        // `protected` field of the same hazard pointer
        hazard.set_reserved(Ordering::Release);
        Ok(())
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
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalInner
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
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

        self.scan_cache.sort_unstable();
        let scan_cache = &self.scan_cache;

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
            hazard.set_free(crate::sanitize::RELAXED_STORE);
        }

        // (LOC:3) this `Release` fence synchronizes-with the `SeqCst` fence (GLO:1)
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
    Self: Clone + Copy + Sized,
{
    /// Gets and Wraps a hazard in a [`HazardPtr`](crate::hazard::HazardPtr).
    fn wrap_hazard(self, protect: NonNull<()>) -> HazardPtr<Self>;

    /// Attempts to recycle `hazard` in the thread local cache for hazards reserved for the current
    /// thread.
    ///
    /// # Errors
    ///
    /// This operation can fail in two circumstances:
    ///
    /// - the thread local cache is at capacity ([`RecycleErr::Capacity`](RecycleErr::Capacity))
    /// - access to the thread local state fails ([`RecycleErr::Access`](RecycleErr::Access))
    fn try_recycle_hazard(self, hazard: &'static Hazard) -> Result<(), RecycleErr>;

    /// Increase the internal count of a threads operations counting towards the threshold for
    /// initiating a new attempt for reclaiming all retired records.
    fn increase_ops_count(self);
}

impl<'a> LocalAccess for &'a Local {
    #[inline]
    fn wrap_hazard(self, protect: NonNull<()>) -> HazardPtr<Self> {
        HazardPtr::new(Local::get_hazard(self, protect), self)
    }

    #[inline]
    fn try_recycle_hazard(self, hazard: &'static Hazard) -> Result<(), RecycleErr> {
        Local::try_recycle_hazard(self, hazard)
    }

    #[inline]
    fn increase_ops_count(self) {
        Local::increase_ops_count(self);
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

impl From<CapacityError<&'static Hazard>> for RecycleErr {
    #[inline]
    fn from(_: CapacityError<&'static Hazard>) -> Self {
        RecycleErr::Capacity
    }
}

#[cfg(test)]
mod tests {
    use std::mem;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::global::Global;
    use crate::hazard::HazardPtr;
    use crate::retired::Retired;

    use super::{Local, HAZARD_CACHE, SCAN_CACHE, SCAN_THRESHOLD};

    static GLOBAL: Global = Global::new();

    struct DropCount<'a>(&'a AtomicUsize);
    impl Drop for DropCount<'_> {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn acquire_local() {
        let local = Local::new(&GLOBAL);
        let ptr = NonNull::from(&());

        let hazards: Box<[_]> = (0..HAZARD_CACHE)
            .map(|_| local.get_hazard(ptr.cast()))
            .map(|hazard| HazardPtr::new(hazard, &local))
            .collect();
        mem::drop(hazards);

        {
            // local hazard cache is full
            let inner = unsafe { &*local.0.get() };
            assert_eq!(0, inner.ops_count);
            assert_eq!(HAZARD_CACHE, inner.hazard_cache.len());
            assert_eq!(SCAN_CACHE, inner.scan_cache.capacity());
            assert_eq!(0, inner.scan_cache.len());
        }

        let _hazards: Box<[_]> = (0..HAZARD_CACHE)
            .map(|_| local.get_hazard(ptr.cast()))
            .map(|hazard| HazardPtr::new(hazard, &local))
            .collect();

        {
            // local hazard cache is empty
            let inner = unsafe { &*local.0.get() };
            assert_eq!(0, inner.ops_count);
            assert_eq!(0, inner.hazard_cache.len());
            assert_eq!(SCAN_CACHE, inner.scan_cache.capacity());
            assert_eq!(0, inner.scan_cache.len());
        }
    }

    #[test]
    #[cfg_attr(feature = "count-release", ignore)]
    fn retire() {
        const THRESHOLD: usize = SCAN_THRESHOLD as usize;

        let count = AtomicUsize::new(0);
        let local = Local::new(&GLOBAL);

        // allocate & retire (THRESHOLD - 1) records
        (0..THRESHOLD - 1)
            .map(|_| Box::new(DropCount(&count)))
            .map(|record| unsafe { Retired::new_unchecked(NonNull::from(Box::leak(record))) })
            .for_each(|retired| local.retire_record(retired));

        {
            let inner = unsafe { &*local.0.get() };
            assert_eq!(THRESHOLD - 1, inner.ops_count as usize);
            assert_eq!(THRESHOLD - 1, inner.retired_bag.inner.len());
        }

        // nothing has been dropped so far
        assert_eq!(0, count.load(Ordering::Relaxed));

        // retire another record, triggering a scan which deallocates all records
        local.retire_record(unsafe {
            Retired::new_unchecked(NonNull::from(Box::leak(Box::new(DropCount(&count)))))
        });

        {
            let inner = unsafe { &*local.0.get() };
            assert_eq!(0, inner.ops_count as usize);
            assert_eq!(0, inner.retired_bag.inner.len());
        }

        assert_eq!(THRESHOLD, count.load(Ordering::Relaxed));
    }

    #[test]
    #[cfg_attr(feature = "max-reclamation-freq", ignore)]
    fn drop() {
        const BELOW_THRESHOLD: usize = SCAN_THRESHOLD as usize / 2;

        let count = AtomicUsize::new(0);
        let local = Local::new(&GLOBAL);

        (0..BELOW_THRESHOLD)
            .map(|_| Box::new(DropCount(&count)))
            .map(|record| unsafe { Retired::new_unchecked(NonNull::from(Box::leak(record))) })
            .for_each(|retired| local.retire_record(retired));

        // all retired records are reclaimed when local is dropped
        mem::drop(local);
        assert_eq!(BELOW_THRESHOLD, count.load(Ordering::Relaxed));
    }
}
