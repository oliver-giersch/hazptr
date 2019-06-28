//! Thread local state and caches for reserving hazard pointers and storing
//! retired records.

#[cfg(feature = "std")]
use std::error;

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec::Vec};

use core::cell::UnsafeCell;
use core::fmt;
use core::mem::ManuallyDrop;
use core::ptr::{self, NonNull};
use core::sync::atomic::{
    self,
    Ordering::{Release, SeqCst},
};

use arrayvec::{ArrayVec, CapacityError};

use crate::bag::{ReclaimOnDrop, Retired, RetiredBag};
use crate::global;
use crate::hazard::{Hazard, Protected};
use crate::sanitize;

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
// LocalAccess (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A trait for abstracting over different means of accessing thread local state
pub trait LocalAccess
where
    Self: Clone + Copy + Sized,
{
    /// Gets a hazard from local or global storage.
    fn get_hazard(self, protect: Option<NonNull<()>>) -> &'static Hazard;

    /// Attempts to recycle `hazard` in the thread local cache for hazards
    /// reserved for the current thread.
    ///
    /// # Errors
    ///
    /// This operation can fail in two circumstances:
    ///
    /// - the thread local cache is full ([`RecycleErr::Capacity`](RecycleErr::Capacity))
    /// - access to the thread local state fails ([`RecycleErr::Access`](RecycleErr::Access))
    fn try_recycle_hazard(self, hazard: &'static Hazard) -> Result<(), RecycleError>;

    /// Increase the internal count of a threads operations counting towards the
    /// threshold for initiating a new attempt for reclaiming all retired
    /// records.
    fn increase_ops_count(self);
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Local
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Container for all thread local data required for reclamation with hazard
/// pointers.
#[derive(Debug)]
pub struct Local(UnsafeCell<LocalInner>);

impl Default for Local {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Local {
    /// Creates a new container for the thread local state.
    #[inline]
    pub fn new() -> Self {
        Self(UnsafeCell::new(LocalInner {
            ops_count: 0,
            hazard_cache: ArrayVec::new(),
            scan_cache: Vec::with_capacity(SCAN_CACHE),
            retired_bag: match global::GLOBAL.try_adopt_abandoned_records() {
                Some(boxed) => ManuallyDrop::new(boxed),
                None => ManuallyDrop::new(Box::new(RetiredBag::new())),
            },
        }))
    }

    /// Retires a record and increases the operations count.
    ///
    /// If the operations count reaches a threshold, a scan is triggered which
    /// reclaims all records that can be safely reclaimed and resets the
    /// operations count.
    /// Previously, an attempt is made to adopt all globally abandoned records.
    #[inline]
    pub(crate) fn retire_record(&self, record: Retired) {
        let local = unsafe { &mut *self.0.get() };
        local.retired_bag.inner.push(ReclaimOnDrop::from(record));
        #[cfg(not(feature = "count-release"))]
        local.increase_ops_count();
    }
}

impl<'a> LocalAccess for &'a Local {
    /// Attempts to take a reserved hazard from the thread local cache if there
    /// are any.
    #[inline]
    fn get_hazard(self, protect: Option<NonNull<()>>) -> &'static Hazard {
        let local = unsafe { &mut *self.0.get() };
        match local.hazard_cache.pop() {
            Some(hazard) => hazard,
            None => global::GLOBAL.get_hazard(protect),
        }
    }

    /// Attempts to cache `hazard` in the thread local storage.
    ///
    /// # Errors
    ///
    /// The operation can fail if the thread local hazard cache is at maximum
    /// capacity.
    #[inline]
    fn try_recycle_hazard(self, hazard: &'static Hazard) -> Result<(), RecycleError> {
        unsafe { &mut *self.0.get() }.hazard_cache.try_push(hazard)?;

        // (LOC:1) this `Release` store synchronizes-with any `Acquire` load on the
        // `protected` field of the same hazard pointer
        hazard.set_thread_reserved(Release);
        Ok(())
    }

    /// Increases the thread local operations count and triggers a scan if the
    /// threshold is reached.
    #[inline]
    fn increase_ops_count(self) {
        unsafe { &mut *self.0.get() }.increase_ops_count();
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalInner
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
struct LocalInner {
    ops_count: u32,
    hazard_cache: ArrayVec<[&'static Hazard; HAZARD_CACHE]>,
    scan_cache: Vec<Protected>,
    retired_bag: ManuallyDrop<Box<RetiredBag>>,
}

impl LocalInner {
    /// Increases the operations count and triggers a scan if the threshold is
    /// reached.
    #[inline]
    fn increase_ops_count(&mut self) {
        self.ops_count += 1;

        if self.ops_count == SCAN_THRESHOLD {
            // try to adopt and merge any (global) abandoned retired bags
            if let Some(abandoned_bag) = global::GLOBAL.try_adopt_abandoned_records() {
                self.retired_bag.merge(abandoned_bag.inner);
            }

            let _ = self.scan_hazards();
            self.ops_count = 0;
        }
    }

    /// Reclaims all locally retired records that are unprotected and returns
    /// the number of reclaimed records.
    #[inline]
    fn scan_hazards(&mut self) -> usize {
        let len = self.retired_bag.inner.len();
        if len == 0 {
            return 0;
        }

        // (LOC:2) this `SeqCst` fence synchronizes-with the `SeqCst` stores ... and the `SeqCst`
        // CAS (LIS:3P).
        // This enforces a total order between all these operations, which is required in order to
        // ensure that all stores PROTECTING pointers are fully visible BEFORE the hazard pointers
        // are scanned and unprotected retired records are reclaimed.

        // (GLO:1) this `SeqCst` fence synchronizes-with the `SeqCst` stores (LOC:1), (GUA:2),
        // (GUA:4) and the `SeqCst` CAS (LIS:3P). This establishes total order between all these
        // operations, which is required here in order to ensure that all stores protecting pointers
        // have become fully visible when the hazard pointers are scanned and retired records are
        // reclaimed.
        global::GLOBAL.collect_protected_hazards(&mut self.scan_cache, SeqCst);

        self.scan_cache.sort_unstable();
        unsafe { self.reclaim_unprotected_records() };

        len - self.retired_bag.inner.len()
    }

    // this is declared unsafe because in this function the retired records are actually dropped.
    #[allow(unused_unsafe)]
    unsafe fn reclaim_unprotected_records(&mut self) {
        let scan_cache = &self.scan_cache;
        self.retired_bag.inner.retain(|retired| {
            // retain (DON'T drop) all records found within the scan cache of protected hazards
            scan_cache.binary_search_by(|protected| retired.compare_with(protected)).is_ok()
        });
    }
}

impl Drop for LocalInner {
    #[inline]
    fn drop(&mut self) {
        for hazard in &self.hazard_cache {
            hazard.set_free(sanitize::RELAXED_STORE);
        }

        // (LOC:3) this `Release` fence synchronizes-with ...
        atomic::fence(Release);

        let _ = self.scan_hazards();
        // this is safe because the `retired_bag` field is neither accessed afterwards nor dropped
        let bag = unsafe { ptr::read(&*self.retired_bag) };

        if !bag.inner.is_empty() {
            global::GLOBAL.abandon_retired_bag(bag);
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// RecycleErr
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Error type for thread local recycle operations.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RecycleError {
    Access,
    Capacity,
}

impl From<CapacityError<&'static Hazard>> for RecycleError {
    #[inline]
    fn from(_: CapacityError<&'static Hazard>) -> Self {
        RecycleError::Capacity
    }
}

impl fmt::Display for RecycleError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            RecycleError::Access => {
                write!(f, "failed to access already destroyed thread local storage")
            }
            RecycleError::Capacity => {
                write!(f, "thread local cache for hazard pointer already full")
            }
        }
    }
}

#[cfg(feature = "std")]
impl error::Error for RecycleError {}

#[cfg(test)]
mod tests {
    use std::mem;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::bag::Retired;

    use super::{Local, LocalAccess, HAZARD_CACHE, SCAN_CACHE, SCAN_THRESHOLD};

    struct DropCount<'a>(&'a AtomicUsize);
    impl Drop for DropCount<'_> {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn acquire_local() {
        let local = Local::new();
        let ptr = NonNull::from(&());

        (0..HAZARD_CACHE)
            .map(|_| local.get_hazard(Some(ptr.cast())))
            .collect::<Box<[_]>>()
            .iter()
            .try_for_each(|hazard| local.try_recycle_hazard(hazard))
            .unwrap();

        {
            // local hazard cache is full
            let inner = unsafe { &*local.0.get() };
            assert_eq!(0, inner.ops_count);
            assert_eq!(HAZARD_CACHE, inner.hazard_cache.len());
            assert_eq!(SCAN_CACHE, inner.scan_cache.capacity());
            assert_eq!(0, inner.scan_cache.len());
        }

        // takes all hazards out of local cache and then allocates a new one.
        let hazards: Box<[_]> =
            (0..HAZARD_CACHE).map(|_| local.get_hazard(Some(ptr.cast()))).collect();
        let extra = local.get_hazard(Some(ptr.cast()));

        {
            // local hazard cache is empty
            let inner = unsafe { &*local.0.get() };
            assert_eq!(0, inner.ops_count);
            assert_eq!(0, inner.hazard_cache.len());
            assert_eq!(SCAN_CACHE, inner.scan_cache.capacity());
            assert_eq!(0, inner.scan_cache.len());
        }

        hazards.iter().try_for_each(|hazard| local.try_recycle_hazard(*hazard)).unwrap();

        local.try_recycle_hazard(extra).unwrap_err();
    }

    #[test]
    #[cfg_attr(feature = "count-release", ignore)]
    fn retire() {
        const THRESHOLD: usize = SCAN_THRESHOLD as usize;

        let count = AtomicUsize::new(0);
        let local = Local::new();

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
        let local = Local::new();

        (0..BELOW_THRESHOLD)
            .map(|_| Box::new(DropCount(&count)))
            .map(|record| unsafe { Retired::new_unchecked(NonNull::from(Box::leak(record))) })
            .for_each(|retired| local.retire_record(retired));

        // all retired records are reclaimed when local is dropped
        mem::drop(local);
        assert_eq!(BELOW_THRESHOLD, count.load(Ordering::Relaxed));
    }
}
