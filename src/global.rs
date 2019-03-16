use std::sync::atomic::Ordering;

use lazy_static::lazy_static;

use crate::hazard::{Hazard, HazardList, Protected};
use crate::retired::{AbandonedBags, RetiredBag};
use std::ptr::NonNull;

#[inline]
pub fn acquire_hazard_for(ptr: NonNull<()>) -> Hazard {
    Hazard::from(GLOBAL.hazards.acquire_hazard(ptr))
}

#[inline]
pub fn collect_protected_hazards(vec: &mut Vec<Protected>) {
    vec.clear();
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

lazy_static! {
    static ref GLOBAL: Global = Global {
        hazards: HazardList::new(),
        abandoned: AbandonedBags::new(),
    };
}

struct Global {
    hazards: HazardList,
    abandoned: AbandonedBags,
}
