//! Operations on globally shared data for hazard pointers and abandoned retired records.

use std::ptr::NonNull;
use std::sync::atomic::{self, Ordering};

use crate::hazard::{HazardList, HazardPtr, Protected};
use crate::retired::{AbandonedBags, RetiredBag};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Global data structures
////////////////////////////////////////////////////////////////////////////////////////////////////

static HAZARDS: HazardList = HazardList::new();
static ABANDONED: AbandonedBags = AbandonedBags::new();

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hazard list
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Infallibly acquires a hazard pointer from the global list.
///
/// This either finds an already allocated one that is currently not in use **or** allocates a new
/// hazard pointer and appends it to the list.
#[inline]
pub fn acquire_hazard_for(ptr: NonNull<()>) -> HazardPtr {
    HazardPtr::from(HAZARDS.acquire_hazard_for(ptr))
}

/// Collects all currently active hazard pointers into the supplied `Vec`.
#[inline]
pub fn collect_protected_hazards(vec: &mut Vec<Protected>) {
    vec.clear();

    // (GLO:1) this `SeqCst` fence establishes a total order with the `SeqCst` store in (HAZ:2) and
    // the `SeqCst` CAS (LIS:3P)
    // sequential consistency is required here in order to ensure that all stores to `protected` for
    // all hazard pointers are totally ordered and thus visible when the hazard pointers are scanned
    atomic::fence(Ordering::SeqCst);

    vec.extend(
        HAZARDS
            .iter()
            .filter_map(|hazard| hazard.protected(Ordering::Relaxed)),
    )
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Abandoned record list
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Abandons a thread's retired bag that still contains records, which could not be reclaimed at the
/// time the thread exits.
#[inline]
pub fn abandon_retired_bag(bag: Box<RetiredBag>) {
    debug_assert!(!bag.inner.is_empty());
    ABANDONED.push(bag);
}

/// Takes and merges all abandoned records and returns them as a single `RetiredBag`.
#[inline]
pub fn try_adopt_abandoned_records() -> Option<Box<RetiredBag>> {
    ABANDONED.take_and_merge()
}
