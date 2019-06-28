//! Operations on globally shared data for hazard pointers and abandoned retired
//! records.

use core::ptr::NonNull;
use core::sync::atomic::{
    self,
    Ordering::{self, SeqCst},
};

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec::Vec};

use crate::hazard::{Hazard, HazardList, Protected};
use crate::retired::{AbandonedBags, RetiredBag};
use crate::sanitize;

/// The single static `Global` instance
pub(crate) static GLOBAL: Global = Global::new();

////////////////////////////////////////////////////////////////////////////////////////////////////
// Global
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Global data structures required for managing memory reclamation with hazard
/// pointers.
#[derive(Debug)]
pub(crate) struct Global {
    hazards: HazardList,
    abandoned: AbandonedBags,
}

impl Global {
    /// Creates a new instance of a `Global`.
    #[inline]
    pub const fn new() -> Self {
        Self { hazards: HazardList::new(), abandoned: AbandonedBags::new() }
    }

    /// Acquires a hazard pointer from the global list and reserves it for the
    /// thread requesting it.
    ///
    /// This operation traverses the entire list from the head, trying to find
    /// an unused hazard.
    /// If it does not find one, it allocates a new one and appends it to the
    /// end of the list.
    #[inline]
    pub fn get_hazard(&'static self, protect: Option<NonNull<()>>) -> &'static Hazard {
        self.hazards.get_hazard(protect)
    }

    /// Collects all currently active hazard pointers into the supplied `Vec`.
    #[inline]
    pub fn collect_protected_hazards(&'static self, vec: &mut Vec<Protected>, order: Ordering) {
        debug_assert_eq!(order, SeqCst, "must only be called with `SeqCst`");
        vec.clear();

        atomic::fence(order);

        for hazard in self.hazards.iter().fuse() {
            if let Some(protected) = hazard.protected(sanitize::RELAXED_LOAD) {
                vec.push(protected);
            }
        }
    }

    /// Stores an exiting thread's (non-empty) bag of retired records, which
    /// could not be reclaimed at the time the thread exited.
    #[inline]
    pub fn abandon_retired_bag(&'static self, bag: Box<RetiredBag>) {
        debug_assert!(!bag.inner.is_empty());
        self.abandoned.push(bag);
    }

    /// Takes and merges all abandoned records and returns them as a single
    /// `RetiredBag`.
    #[inline]
    pub fn try_adopt_abandoned_records(&'static self) -> Option<Box<RetiredBag>> {
        self.abandoned.take_and_merge()
    }
}
