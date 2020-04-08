use core::sync::atomic::{self, Ordering};

use crate::hazard::{HazardList, HazardPtr, ProtectStrategy, ProtectedPtr, ProtectedResult};
use crate::strategy::GlobalRetireState;

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalRef
////////////////////////////////////////////////////////////////////////////////////////////////////

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

pub(crate) struct Global {
    /// The required global state for the chosen retire strategy.
    pub(crate) retire_state: GlobalRetireState,
    /// The global list of all hazard pointers.
    hazards: HazardList,
}

/********** impl inherent *************************************************************************/

impl Global {
    #[inline]
    pub const fn new(retire_state: GlobalRetireState) -> Self {
        Self { retire_state, hazards: HazardList::new() }
    }

    #[cold]
    pub fn get_hazard(&self, strategy: ProtectStrategy) -> &HazardPtr {
        match strategy {
            ProtectStrategy::ReserveOnly => self.hazards.get_or_insert_reserved_hazard(),
            ProtectStrategy::Protect(protected) => {
                self.hazards.get_or_insert_hazard(protected.into_inner())
            }
        }
    }

    #[inline]
    pub fn collect_protected_hazards(&self, scan_cache: &mut Vec<ProtectedPtr>, order: Ordering) {
        debug_assert_eq!(order, Ordering::SeqCst, "this method must have `SeqCst` ordering");
        // clear any entries from previous reclamation attempts
        scan_cache.clear();

        // issue full memory fence before iterating all hazard pointers
        // (glo:1) this seq-cst fence syncs-with the seq-cst CAS (lst:1)
        atomic::fence(Ordering::SeqCst);

        for hazard in self.hazards.iter() {
            match hazard.protected(Ordering::Relaxed) {
                ProtectedResult::Protected(protected) => scan_cache.push(protected),
                ProtectedResult::AbortIteration => return,
                _ => {}
            }
        }
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
