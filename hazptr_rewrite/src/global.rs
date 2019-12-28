use core::convert::AsRef;

use crate::hazard::{HazardList, HazardPtr, ProtectStrategy};
use crate::retire::RetireStrategy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct GlobalHandle<'global, S: RetireStrategy> {
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

#[derive(Debug)]
pub struct Global<S: RetireStrategy> {
    pub(crate) state: S::Global,
    hazards: HazardList,
}

/********** impl inherent *************************************************************************/

impl<S: RetireStrategy> Global<S> {
    #[inline]
    pub fn new() -> Self {
        Self { state: Default::default(), hazards: HazardList::new() }
    }

    #[inline]
    pub(crate) fn get_hazard(&self, strategy: ProtectStrategy) -> &HazardPtr {
        match strategy {
            ProtectStrategy::ReserveOnly => self.hazards.get_or_insert_reserved_hazard(),
            ProtectStrategy::Protect(protected) => self.hazards.get_or_insert_hazard(protected),
        }
    }
}

/********** impl Default **************************************************************************/

impl<S: RetireStrategy> Default for Global<S> {
    #[inline]
    fn default() -> Self {
        Self { state: Default::default(), hazards: HazardList::new() }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalRef
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub(crate) enum GlobalRef<'a, S: RetireStrategy> {
    Ref(&'a Global<S>),
    Raw(*const Global<S>),
}
