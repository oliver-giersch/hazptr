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

use conquer_reclaim::{OwningReclaimer, Reclaimer};

use crate::global::{Global, GlobalHandle};
use crate::local::{Local, LocalHandle};
use crate::policy::Policy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// ArcHp
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct ArcHp<P: Policy> {
    handle: Arc<Global<P>>,
}

/********** impl inherent *************************************************************************/

impl<P: Policy> ArcHp<P> {
    #[inline]
    pub fn owning_local_handle(&self) -> LocalHandle<'_, '_, P, Self> {
        LocalHandle::owning(GlobalHandle::from_owned(Arc::clone(&self.handle)))
    }
}

/********** impl Default **************************************************************************/

impl<P: Policy> Default for ArcHp<P> {
    #[inline]
    fn default() -> Self {
        Self { handle: Arc::new(Global::default()) }
    }
}

/********** impl GenericReclaimer *****************************************************************/

unsafe impl<P: Policy> OwningReclaimer for ArcHp<P> {
    type Handle = LocalHandle<'static, 'static, P, Self>;

    #[inline]
    fn owning_local_handle(&self) -> Self::Handle {
        LocalHandle::owning(GlobalHandle::from_owned(Arc::clone(&self.handle)))
    }
}

/********** impl Reclaimer ************************************************************************/

unsafe impl<P: Policy> Reclaimer for ArcHp<P> {
    type Global = Global<P>;
    type Header = P::Header;

    #[inline]
    fn new() -> Self {
        ArcHp::default()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hp
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct Hp<P: Policy> {
    global: Global<P>,
}

/********** impl inherent *************************************************************************/

impl<P: Policy> Hp<P> {
    #[inline]
    pub fn new() -> Self {
        Self { global: Global::new() }
    }

    /// fixme: CANT MAKE NO OWNING HANDLE
    #[inline]
    pub fn owning_local_handle<'global>(&'global self) -> LocalHandle<'_, 'global, P, Self> {
        let local = Rc::new(Local::new(GlobalHandle::from_ref(&self.global)));
        LocalHandle::from_owned(local)
    }

    #[inline]
    pub fn ref_local_handle<'local, 'global>(
        &'global self,
        local: &'local Local<'global, P>,
    ) -> LocalHandle<'local, 'global, P, Self> {
        LocalHandle::from_ref(local)
    }

    #[inline]
    pub unsafe fn raw_local_handle(&self) -> LocalHandle<'_, '_, P, Self> {
        let local = Rc::new(Local::new(GlobalHandle::from_raw(&self.global)));
        LocalHandle::from_owned(local)
    }
}

/********** impl Default **************************************************************************/

impl<P: Policy> Default for Hp<P> {
    #[inline]
    fn default() -> Self {
        Self { global: Global::new() }
    }
}

/********** impl Reclaimer ************************************************************************/

unsafe impl<P: Policy> Reclaimer for Hp<P> {
    type Global = Global<P>;
    type Header = P::Header;

    #[inline]
    fn new() -> Self {
        Self::default()
    }
}
