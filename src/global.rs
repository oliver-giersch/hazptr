use core::sync::atomic::{self, Ordering};

use crate::hazard::{HazardList, HazardPtr, ProtectStrategy, ProtectedPtr, ProtectedResult};
use crate::strategy::GlobalRetireState;

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalRef
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
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

#[derive(Debug)]
pub(crate) struct Global {
    pub(crate) retire_state: GlobalRetireState,
    hazards: HazardList,
}

/********** impl inherent *************************************************************************/

impl Global {
    #[inline]
    pub const fn new(retire_state: GlobalRetireState) -> Self {
        Self { retire_state, hazards: HazardList::new() }
    }

    #[inline]
    pub fn get_hazard(&self, strategy: ProtectStrategy) -> &HazardPtr {
        match strategy {
            ProtectStrategy::ReserveOnly => self.hazards.get_or_insert_reserved_hazard(),
            ProtectStrategy::Protect(protected) => {
                self.hazards.get_or_insert_hazard(protected.into_inner())
            }
        }
    }

    #[inline]
    pub fn collect_protected_hazards(&self, vec: &mut Vec<ProtectedPtr>, order: Ordering) {
        assert_eq!(order, Ordering::SeqCst, "this method must have `SeqCst` ordering");
        vec.clear();

        atomic::fence(Ordering::SeqCst);

        for hazard in self.hazards.iter() {
            match hazard.protected(Ordering::Relaxed) {
                ProtectedResult::Protected(protected) => vec.push(protected),
                ProtectedResult::Abort => return,
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
#[derive(Debug)]
enum Ref<'a> {
    Ref(&'a Global),
    Raw(*const Global),
}
