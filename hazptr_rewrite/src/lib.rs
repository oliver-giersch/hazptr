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

use core::marker::PhantomData;
use core::sync::atomic::AtomicPtr;

#[cfg(not(feature = "std"))]
use alloc::sync::Arc;
#[cfg(feature = "std")]
use std::sync::Arc;

use conquer_reclaim::{Reclaimer, ReclaimerHandle, Record};

use crate::global::{Global, GlobalHandle, GlobalRef};
use crate::local::LocalHandle;
use crate::policy::Policy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// HP
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct HP<P> {
    global: Arc<Global<P>>,
}

/********** impl Reclaimer ************************************************************************/

unsafe impl<P: Policy> Reclaimer for HP<P> {
    type Global = Global<P>;
    type Header = P::Header;
    type Handle = LocalHandle<'static, 'static, P>;

    #[inline]
    fn create_local_handle(&self) -> Self::Handle {
        LocalHandle::owning(GlobalHandle::from_owned(Arc::clone(&self.global)))
    }
}
