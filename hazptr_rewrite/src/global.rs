use core::convert::AsRef;
use core::sync::atomic::{self, Ordering};

use crate::hazard::{HazardList, HazardPtr, ProtectStrategy, ProtectedPtr};
use crate::retire::RetireStrategy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub(crate) struct GlobalHandle<'global, S> {
    inner: GlobalRef<'global, S>,
}

/********** impl inherent *************************************************************************/

impl<'global, S: RetireStrategy> GlobalHandle<'global, S> {
    #[inline]
    pub fn from_ref(global: &'global Global<S>) -> Self {
        Self { inner: GlobalRef::Ref(global) }
    }
}

impl<S: RetireStrategy> GlobalHandle<'_, S> {
    /// Creates a new [`GlobalHandle`] from a raw pointer.
    ///
    /// # Safety
    ///
    /// The caller has to ensure that the resulting [`GlobalHandle`] does not
    /// outlive the [`Global`] it points to.
    #[inline]
    pub unsafe fn from_raw(global: *const Global<S>) -> Self {
        Self { inner: GlobalRef::Raw(global) }
    }
}

/********** impl AsRef ****************************************************************************/

impl<'global, S: RetireStrategy> AsRef<Global<S>> for GlobalHandle<'global, S> {
    #[inline]
    fn as_ref(&self) -> &Global<S> {
        match &self.inner {
            GlobalRef::Ref(global) => *global,
            GlobalRef::Raw(ref global) => unsafe { &**global },
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Global
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Default)]
pub struct Global<S> {
    pub(crate) state: S,
    hazards: HazardList,
}

/********** impl inherent *************************************************************************/

impl<S: RetireStrategy> Global<S> {
    #[inline]
    pub(crate) fn new() -> Self {
        Default::default()
    }

    #[inline]
    pub(crate) fn get_hazard(&self, strategy: ProtectStrategy) -> &HazardPtr {
        match strategy {
            ProtectStrategy::ReserveOnly => self.hazards.get_or_insert_reserved_hazard(),
            ProtectStrategy::Protect(protected) => self.hazards.get_or_insert_hazard(protected),
        }
    }

    #[inline]
    pub(crate) fn collect_protected_hazards(&self, vec: &mut Vec<ProtectedPtr>, order: Ordering) {
        assert_eq!(order, Ordering::SeqCst, "this method must have `SeqCst` ordering");
        vec.clear();

        atomic::fence(Ordering::SeqCst);

        for hazard in self.hazards.iter() {
            if let Some(protected) = hazard.protected(Ordering::Relaxed) {
                vec.push(protected);
            }
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalRef
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A reference to a [`Global`] that is either safe but lifetime-bound or unsafe
/// and lifetime-independent (a raw pointer).
#[derive(Debug)]
enum GlobalRef<'a, S> {
    Ref(&'a Global<S>),
    Raw(*const Global<S>),
}
