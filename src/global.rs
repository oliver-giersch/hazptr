//! Operations on globally shared data for hazard pointers and abandoned retired records.

use core::ptr::NonNull;
use core::sync::atomic::{self, Ordering};

#[cfg(feature = "no-std")]
use alloc::{boxed::Box, vec::Vec};

use crate::hazard::{Hazard, HazardList, Protected};
use crate::retired::{AbandonedBags, RetiredBag};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Global
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Global data structures required for managing memory reclamation with hazard pointers.
pub struct Global {
    hazards: HazardList,
    abandoned: AbandonedBags,
}

impl Global {
    /// Creates a new instance of a `Global`.
    #[inline]
    pub const fn new() -> Self {
        Self {
            hazards: HazardList::new(),
            abandoned: AbandonedBags::new(),
        }
    }

    /// Acquires a hazard pointer from the global list and sets it to protect the given pointer.
    ///
    /// This operation traverses the entire list from the head, trying to find an unused hazard.
    /// If it does not find one, it allocates a new one and appends it to the end of the list.
    #[inline]
    pub(crate) fn acquire_hazard_for(&'static self, ptr: NonNull<()>) -> &'static Hazard {
        self.hazards.acquire_hazard_for(ptr)
    }

    /// Collects all currently active hazard pointers into the supplied `Vec`.
    #[inline]
    pub(crate) fn collect_protected_hazards(&'static self, vec: &mut Vec<Protected>) {
        vec.clear();

        // (GLO:1) this `SeqCst` fence establishes a total order with the `SeqCst` store in (HAZ:2)
        // and the `SeqCst` CAS (LIS:3P)
        // sequential consistency is required here in order to ensure that all stores to `protected`
        // for all hazard pointers are totally ordered and thus visible when the hazard pointers are
        // scanned
        atomic::fence(Ordering::SeqCst);

        vec.extend(
            self.hazards
                .iter()
                .filter_map(|hazard| hazard.protected(Ordering::Relaxed)),
        )
    }

    /// Stores an exiting thread's non-empty bag of retired records, which could not be reclaimed at
    /// the time the thread exited.
    #[inline]
    pub(crate) fn abandon_retired_bag(&'static self, bag: Box<RetiredBag>) {
        debug_assert!(!bag.inner.is_empty());
        self.abandoned.push(bag);
    }

    /// Takes and merges all abandoned records and returns them as a single `RetiredBag`.
    #[inline]
    pub(crate) fn try_adopt_abandoned_records(&'static self) -> Option<Box<RetiredBag>> {
        self.abandoned.take_and_merge()
    }
}
