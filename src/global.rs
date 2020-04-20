use core::sync::atomic::{self, Ordering};

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use crate::hazard::{HazardList, HazardPtr, ProtectStrategy, ProtectedPtr, ProtectedResult};
use crate::strategy::GlobalRetireState;

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalRef
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A reference to the `Global` state.
pub(crate) struct GlobalRef<'global> {
    inner: Ref<'global>,
}

/********** impl inherent *************************************************************************/

impl<'global> GlobalRef<'global> {
    /// Creates a new [`GlobalRef`] from the reference `global` which is
    /// consequently bound to its lifetime.
    #[inline]
    pub fn from_ref(global: &'global Global) -> Self {
        Self { inner: Ref::Ref(global) }
    }

    /// Returns a reference to the `Global` state.
    #[inline]
    pub fn as_ref(&self) -> &'global Global {
        match &self.inner {
            Ref::Ref(global) => global,
            Ref::Raw(ref global) => unsafe { &**global },
        }
    }
}

impl GlobalRef<'_> {
    /// Creates a new [`GlobalRef`] from the raw pointer `global`.
    ///
    /// # Safety
    ///
    /// The caller has to ensure that the resulting [`GlobalRef`] does not
    /// outlive the pointed to [`Global`].
    #[inline]
    pub unsafe fn from_raw(global: *const Global) -> Self {
        Self { inner: Ref::Raw(global) }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Global
////////////////////////////////////////////////////////////////////////////////////////////////////

/// The global state for managing hazard pointers.
pub(crate) struct Global {
    /// The required global state for the chosen retire strategy.
    pub(crate) retire_state: GlobalRetireState,
    /// The global list of all hazard pointers.
    hazards: HazardList,
}

/********** impl inherent *************************************************************************/

impl Global {
    /// Creates a new `Global`.
    #[inline]
    pub const fn new(retire_state: GlobalRetireState) -> Self {
        Self { retire_state, hazards: HazardList::new() }
    }

    /// Acquires a free hazard pointer from the global list.
    #[cold]
    pub fn get_hazard(&self, strategy: ProtectStrategy) -> &HazardPtr {
        match strategy {
            ProtectStrategy::ReserveOnly => self.hazards.get_or_insert_reserved_hazard(),
            ProtectStrategy::Protect(protected) => {
                self.hazards.get_or_insert_hazard(protected.into_inner())
            }
        }
    }

    /// Clears the `scan_cache`, collects all active (protected) hazard pointers
    /// into `scan_cache` and then sorts it.
    #[inline]
    pub fn collect_hazard_pointers(&self, scan_cache: &mut Vec<ProtectedPtr>) {
        // clear any entries from previous reclamation attempts
        scan_cache.clear();

        // issue full memory fence before iterating all hazard pointers (glo:1) this seq-cst fence
        // syncs-with the seq-cst CAS (lst:1)
        atomic::fence(Ordering::SeqCst);

        // iterate all hazard pointers, collect active (protected) ones and abort if one is
        // encountered, which can't have any active ones following it
        for hazard in self.hazards.iter() {
            match hazard.protected(Ordering::Relaxed) {
                ProtectedResult::Protected(protected) => scan_cache.push(protected),
                ProtectedResult::AbortIteration => break,
                _ => {}
            }
        }

        // sort the scan cache for the subsequent binary search
        scan_cache.sort_unstable();
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Ref
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A reference to a [`Global`] that is either safe but lifetime-bound or unsafe
/// and lifetime-independent (a raw pointer).
enum Ref<'global> {
    Ref(&'global Global),
    Raw(*const Global),
}
