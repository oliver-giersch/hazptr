//! Operations on globally shared data for hazard pointers and abandoned retired
//! records.

use core::ptr::NonNull;
use core::sync::atomic::{self, Ordering::SeqCst};

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec::Vec};

use crate::bag::{AbandonedBags, RetiredBag};
use crate::hazard::{Hazard, HazardList, Protected};
use crate::sanitize;

/// The single static `Global` instance
pub(crate) static GLOBAL: Global = Global::new();

////////////////////////////////////////////////////////////////////////////////////////////////////
// Global
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Global data structures required for managing memory reclamation with hazard
/// pointers.
#[derive(Debug)]
pub struct Global {
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
    pub(crate) fn get_hazard(&'static self, protect: Option<NonNull<()>>) -> &'static Hazard {
        self.hazards.get_hazard(protect)
    }

    /// Collects all currently active hazard pointers into the supplied `Vec`.
    #[inline]
    pub(crate) fn collect_protected_hazards(&'static self, vec: &mut Vec<Protected>) {
        vec.clear();

        // (GLO:1) this `SeqCst` fence synchronizes-with the `SeqCst` stores (LOC:1), (GUA:2),
        // (GUA:4) and the `SeqCst` CAS (LIS:3P). This establishes total order between all these
        // operations, which is required here in order to ensure that all stores protecting pointers
        // have become fully visible when the hazard pointers are scanned and retired records are
        // reclaimed.
        atomic::fence(SeqCst);

        for hazard in self.hazards.iter().fuse() {
            if let Some(protected) = hazard.protected(sanitize::RELAXED_LOAD) {
                vec.push(protected);
            }
        }
    }

    /// Stores an exiting thread's non-empty bag of retired records, which could
    /// not be reclaimed at the time the thread exited.
    #[inline]
    pub(crate) fn abandon_retired_bag(&'static self, bag: Box<RetiredBag>) {
        self.abandoned.push(bag);
    }

    /// Takes and merges all abandoned records and returns them as a single
    /// `RetiredBag`.
    #[inline]
    pub(crate) fn try_adopt_abandoned_records(&'static self) -> Option<Box<RetiredBag>> {
        self.abandoned.take_and_merge()
    }
}
