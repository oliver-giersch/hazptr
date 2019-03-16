use std::cell::UnsafeCell;
use std::mem::ManuallyDrop;
use std::sync::atomic::Ordering;

use arrayvec::ArrayVec;

use crate::global;
use crate::hazard::{Hazard, HazardPair, Protected};
use crate::retired::{Retired, RetiredBag};

#[inline]
pub fn acquire_hazard() -> Option<Hazard> {
    LOCAL.with(|cell| {
        unsafe { &mut *cell.get() }
            .hazard_cache
            .pop()
            .map(Hazard::from)
    })
}

#[inline]
pub fn try_recycle_hazard(hazard: &'static HazardPair) -> Result<(), &'static HazardPair> {
    LOCAL.with(move |cell| {
        let local = unsafe { &mut *cell.get() };
        match local.hazard_cache.try_push(hazard) {
            Ok(_) => {
                // (1) this `Release` store synchronizes with any load on the same hazard pointer
                hazard.set_reserved(Ordering::Release);
                Ok(())
            }
            Err(_) => Err(hazard),
        }
    })
}

#[inline]
pub fn retire_record(record: Retired) {
    LOCAL.with(move |cell| unsafe { &mut *cell.get() }.retire_record(record));
}

////////////////////////////////////////////////////////////////////////////////////////////////////
/// Local
////////////////////////////////////////////////////////////////////////////////////////////////////

thread_local! {
    static LOCAL: UnsafeCell<Local> = UnsafeCell::new(Local::new());
}

const SCAN_THRESHOLD: u32 = 100;
const HAZARD_CACHE: usize = 16;
const SCAN_CACHE: usize = 128;

struct Local {
    ops_count: u32,
    hazard_cache: ArrayVec<[&'static HazardPair; HAZARD_CACHE]>,
    scan_cache: Vec<Protected>,
    retired_bag: ManuallyDrop<Box<RetiredBag>>,
}

impl Local {
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

    fn retire_record(&mut self, record: Retired) {
        self.retired_bag.inner.push(record);
        self.ops_count += 1;

        if self.ops_count == SCAN_THRESHOLD {
            self.scan_hazards();
            self.ops_count = 0;
        }
    }

    fn scan_hazards(&mut self) {
        // adopt and merge any abandoned bags
        if let Some(abandoned_bag) = global::try_adopt_abandoned_records() {
            self.retired_bag.merge(abandoned_bag.inner);
        }

        global::collect_protected_hazards(&mut self.scan_cache);

        let mut iter = self.scan_cache.iter();
        self.retired_bag
            .inner
            .retain(move |retired| iter.any(|hazard| hazard.address() == retired.address()));
    }
}

impl Drop for Local {
    fn drop(&mut self) {
        for hazard in &self.hazard_cache {
            // (2) this `Release` store synchronizes with any load on the same hazard pointer
            hazard.set_free(Ordering::Release);
        }

        // this is safe because `retired_bag` is not accessed anymore after this
        let bag = unsafe { ManuallyDrop::take(&mut self.retired_bag) };
        global::abandon_retired_bag(bag);
    }
}
