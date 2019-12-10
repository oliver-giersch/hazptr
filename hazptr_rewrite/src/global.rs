use core::convert::AsRef;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        use std::sync::Arc;
    } else {
        use alloc::sync::Arc;
    }
}

use crate::hazard::{Hazard, HazardList, ProtectStrategy};
use crate::policy::Policy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct GlobalHandle<'global, P: Policy> {
    inner: GlobalRef<'global, P>,
}

/********** impl inherent *************************************************************************/

impl<P: Policy> GlobalHandle<'static, P> {
    #[inline]
    pub fn from_owned(global: Arc<Global<P>>) -> Self {
        Self { inner: GlobalRef::Arc(global) }
    }

    #[inline]
    pub unsafe fn from_raw(global: *const Global<P>) -> Self {
        Self { inner: GlobalRef::Raw(global) }
    }
}

impl<'global, P: Policy> GlobalHandle<'global, P> {
    #[inline]
    pub fn from_ref(global: &'global Global<P>) -> Self {
        Self { inner: GlobalRef::Ref(global) }
    }
}

/********** impl AsRef ****************************************************************************/

impl<'global, P: Policy> AsRef<Global<P>> for GlobalHandle<'global, P> {
    #[inline]
    fn as_ref(&self) -> &Global<P> {
        match &self.inner {
            GlobalRef::Arc(global) => global.as_ref(),
            GlobalRef::Ref(global) => *global,
            GlobalRef::Raw(global) => unsafe { &*global },
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Global
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct Global<P: Policy> {
    hazards: HazardList,
    state: P::GlobalState,
}

/********** impl inherent *************************************************************************/

impl<P: Policy> Global<P> {
    #[inline]
    pub fn get_hazard(&self, strategy: ProtectStrategy) -> &Hazard {
        match strategy {
            ProtectStrategy::ReserveOnly => self.hazards.get_or_insert_reserved_hazard(),
            ProtectStrategy::Protect(protected) => {
                self.hazards.get_or_insert_protecting_hazard(protected)
            }
        }
    }
}

/********** impl Default **************************************************************************/

impl<P: Policy> Default for Global<P> {
    #[inline]
    fn default() -> Self {
        unimplemented!()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalRef
////////////////////////////////////////////////////////////////////////////////////////////////////

pub(crate) enum GlobalRef<'a, P: Policy> {
    Arc(Arc<Global<P>>),
    Ref(&'a Global<P>),
    Raw(*const Global<P>),
}
