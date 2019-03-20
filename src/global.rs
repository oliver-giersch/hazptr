use std::ptr::NonNull;
use std::sync::atomic::Ordering;

use lazy_static::lazy_static;

use crate::hazard::{HazardList, HazardPtr, Protected};
use crate::retired::{AbandonedBags, RetiredBag};

lazy_static! {
    static ref HAZARDS: HazardList = HazardList::new();
    static ref ABANDONED: AbandonedBags = AbandonedBags::new();
}

/// Infallibly acquires a hazard pointer from the global list.
///
/// This either finds an already allocated one that is not in use or allocates a new hazard pointer
/// and appends it to the list.
#[inline]
pub fn acquire_hazard_for(ptr: NonNull<()>) -> HazardPtr {
    HazardPtr::from(HAZARDS.acquire_hazard_for(ptr))
}

/// Collects all currently acquired hazard pointers into the supplied `Vec`, which is cleared
/// beforehand.
#[inline]
pub fn collect_protected_hazards(vec: &mut Vec<Protected>) {
    vec.clear();
    // (GLO:1) this `Acquire` load synchronizes with all `Release` stores and fences around the
    // same hazard such as (LOC:1), (LOC:2), ...
    vec.extend(
        HAZARDS
            .iter()
            .filter_map(|hazard| hazard.protected(Ordering::Acquire)),
    )
}

#[inline]
pub fn abandon_retired_bag(bag: Box<RetiredBag>) {
    ABANDONED.push(bag);
}

#[inline]
pub fn try_adopt_abandoned_records() -> Option<Box<RetiredBag>> {
    ABANDONED.take_and_merge()
}
