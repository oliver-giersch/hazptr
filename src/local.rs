use std::cell::UnsafeCell;
use std::mem::ManuallyDrop;
use std::ptr;
use std::sync::atomic::{self, Ordering};

use arrayvec::ArrayVec;

use crate::global;
use crate::hazard::{Hazard, HazardPtr, Protected};
use crate::retired::{Retired, RetiredBag};

thread_local!(static LOCAL: UnsafeCell<Local> = UnsafeCell::new(Local::new()));

/// Attempts to take a reserved hazard from the thread local cache if there are any.
#[inline]
pub fn acquire_hazard() -> Option<HazardPtr> {
    LOCAL.with(|cell| {
        unsafe { &mut *cell.get() }
            .hazard_cache
            .pop()
            .map(HazardPtr::from)
    })
}

/// Tries to cache the given hazard in the thread local storage.
#[inline]
pub fn try_recycle_hazard(hazard: &'static Hazard) -> Result<(), LocalCacheFull> {
    LOCAL.with(move |cell| {
        let local = unsafe { &mut *cell.get() };
        match local.hazard_cache.try_push(hazard) {
            Ok(_) => {
                // (LOC:1) this `Release` store synchronizes with any `Acquire` load on the
                // `protected` field of the same hazard pointer
                hazard.set_reserved(Ordering::Release);
                Ok(())
            }
            Err(_) => Err(LocalCacheFull),
        }
    })
}

/// Retires the given record and drops/deallocates it once it is safe to do so.
#[inline]
pub fn retire_record(record: Retired) {
    LOCAL.with(move |cell| unsafe { &mut *cell.get() }.retire_record(record));
}

/// Marker type for returning `Err` results.
pub struct LocalCacheFull;

////////////////////////////////////////////////////////////////////////////////////////////////////
/// Local
////////////////////////////////////////////////////////////////////////////////////////////////////

const SCAN_THRESHOLD: u32 = 100;
const HAZARD_CACHE: usize = 16;
const SCAN_CACHE: usize = 128;

/// Container for all thread local data required for reclamation with hazard pointers.
struct Local {
    ops_count: u32,
    hazard_cache: ArrayVec<[&'static Hazard; HAZARD_CACHE]>,
    scan_cache: Vec<Protected>,
    retired_bag: ManuallyDrop<Box<RetiredBag>>,
}

impl Local {
    #[inline]
    fn new() -> Self {
        Self {
            ops_count: 0,
            hazard_cache: ArrayVec::new(),
            scan_cache: Vec::with_capacity(SCAN_CACHE),
            retired_bag: match global::try_adopt_abandoned_records() {
                Some(boxed) => ManuallyDrop::new(boxed),
                None => ManuallyDrop::new(Box::new(RetiredBag::new())),
            },
        }
    }

    #[inline]
    fn retire_record(&mut self, record: Retired) {
        self.retired_bag.inner.push(record);
        self.ops_count += 1;

        if self.ops_count == SCAN_THRESHOLD {
            // try to adopt and merge any (global) abandoned retired bags
            if let Some(abandoned_bag) = global::try_adopt_abandoned_records() {
                self.retired_bag.merge(abandoned_bag.inner);
            }

            self.scan_hazards();
            self.ops_count = 0;
        }
    }

    #[inline]
    fn scan_hazards(&mut self) {
        global::collect_protected_hazards(&mut self.scan_cache);

        let scan_cache = &mut self.scan_cache;
        scan_cache.sort_unstable();

        self.retired_bag.inner.retain(move |retired| {
            scan_cache
                .binary_search_by(|protected| protected.address().cmp(&retired.address()))
                .is_ok()
        })
    }
}

impl Drop for Local {
    #[inline]
    fn drop(&mut self) {
        for hazard in &self.hazard_cache {
            hazard.set_free(Ordering::Relaxed);
        }

        // (LOC:2) this `Release` fence ensures the store operations in the previous loop are
        // published and synchronizes with all `Acquire` loads on the same hazard pointers such as..
        atomic::fence(Ordering::Release);

        self.scan_hazards();
        // this is safe because `retired_bag` is neither accessed anymore nor dropped after this
        let bag = unsafe { ptr::read(&*self.retired_bag) };

        if !bag.inner.is_empty() {
            global::abandon_retired_bag(bag);
        }
    }
}
