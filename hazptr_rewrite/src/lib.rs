#![cfg_attr(not(any(test, feature = "std")), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
mod default;

mod global;
mod guard;
mod hazard;
mod local;
mod policy;
mod queue;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        use std::rc::Rc;
        use std::sync::Arc;
    } else {
        use alloc::rc::Rc;
        use alloc::sync::Arc;
    }
}

use conquer_reclaim::{GenericReclaimer, Reclaimer};

use crate::global::{Global, GlobalHandle};
use crate::local::{Local, LocalHandle};
use crate::policy::Policy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// HPHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct HPHandle<P: Policy> {
    handle: Arc<Global<P>>,
}

impl<P: Policy> Default for HPHandle<P> {
    #[inline]
    fn default() -> Self {
        Self { handle: Arc::new(Global::default()) }
    }
}

/********** impl GenericReclaimer *****************************************************************/

unsafe impl<P: Policy> GenericReclaimer for HPHandle<P> {
    type Handle = LocalHandle<'static, 'static, P, Self>;

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

pub struct HP<P: Policy> {
    global: Global<P>,
}

/********** impl inherent *************************************************************************/

impl<P: Policy> HP<P> {
    #[inline]
    pub fn new() -> Self {
        Self { global: Global::new() }
    }

    #[inline]
    pub fn create_local_handle<'global>(&'global self) -> LocalHandle<'static, 'global, P, Self> {
        let local = Rc::new(Local::new(GlobalHandle::from_ref(&self.global)));
        LocalHandle::from_owned(local)
    }
}

/********** impl Default **************************************************************************/

impl<P: Policy> Default for HP<P> {
    #[inline]
    fn default() -> Self {
        Self { global: Global::new() }
    }
}

/********** impl Reclaimer ************************************************************************/

unsafe impl<P: Policy> Reclaimer for HP<P> {
    type Global = Global<P>;
    type Header = P::Header;
}
