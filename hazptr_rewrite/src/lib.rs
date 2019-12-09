#![cfg_attr(not(any(test, feature = "std")), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

mod default;
mod global;
mod guard;
mod hazard;
mod local;
mod policy;
mod queue;

#[cfg(not(feature = "std"))]
use alloc::sync::Arc;
#[cfg(feature = "std")]
use std::sync::Arc;

use conquer_reclaim::{GenericReclaimer, Reclaimer, Record};

use crate::global::{Global, GlobalHandle};
use crate::local::LocalHandle;
use crate::policy::Policy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// HPHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct HPHandle<P> {
    handle: Arc<HP<P>>,
}

/********** impl GenericReclaimer *****************************************************************/

unsafe impl<P: Policy> GenericReclaimer for HPHandle<P> {
    type Handle = LocalHandle<'static, 'static, P>;

    #[inline]
    fn create_local_handle(&self) -> Self::Handle {
        LocalHandle::owning(GlobalHandle::from_owned(Arc::clone(&self.handle)))
    }
}

/********** impl Reclaimer ************************************************************************/

unsafe impl<P: Policy> Reclaimer for HPHandle<P> {
    type Global = Global<P>;
    type Header = P::Header;
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// HP
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct HP<P> {
    global: Global<P>,
}

/********** impl Reclaimer ************************************************************************/

unsafe impl<P: Policy> Reclaimer for HP<P> {
    type Global = Global<P>;
    type Header = P::Header;
}
