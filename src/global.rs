use std::ptr::NonNull;
use std::sync::atomic::Ordering;

use lazy_static::lazy_static;

use crate::hazard::{HazardList, HazardPtr, Protected};
use crate::retired::{AbandonedBags, RetiredBag};

lazy_static! {
    static ref GLOBAL: Global = Global {
        hazards: HazardList::new(),
        abandoned: AbandonedBags::new(),
    };
}

#[inline]
pub fn acquire_hazard_for(ptr: NonNull<()>) -> HazardPtr {
    HazardPtr::from(GLOBAL.hazards.acquire_hazard_for(ptr))
}

#[inline]
pub fn collect_protected_hazards(vec: &mut Vec<Protected>) {
    vec.clear();
    // (GLO:1) this `Acquire` load synchronizes with all `Release` stores and fences around the
    // same hazard such as (LOC:1), (LOC:2), ...
    vec.extend(
        GLOBAL
            .hazards
            .iter()
            .filter_map(|hazard| hazard.protected(Ordering::Acquire)),
    )
}

#[inline]
pub fn abandon_retired_bag(bag: Box<RetiredBag>) {
    GLOBAL.abandoned.push(bag);
}

#[inline]
pub fn try_adopt_abandoned_records() -> Option<Box<RetiredBag>> {
    GLOBAL.abandoned.take_and_merge()
}

struct Global {
    hazards: HazardList,
    abandoned: AbandonedBags,
}
