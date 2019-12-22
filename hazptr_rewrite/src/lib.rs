#![cfg_attr(not(any(test, feature = "std")), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
mod default;

mod config;
mod global;
mod guard;
mod hazard;
mod local;
mod policy;
mod queue;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        use std::sync::Arc;
    } else {
        use alloc::sync::Arc;
    }
}

use conquer_reclaim::Reclaim;

use crate::global::Global;
use crate::local::LocalHandle;
use crate::policy::Policy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// ArcHp
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct ArcHp<P: Policy> {
    handle: Arc<Global<P>>,
}

/********** impl Clone ****************************************************************************/

impl<P: Policy> Clone for ArcHp<P> {
    #[inline]
    fn clone(&self) -> Self {
        Self { handle: Arc::clone(&self.handle) }
    }
}

/********** impl Default **************************************************************************/

impl<P: Policy> Default for ArcHp<P> {
    #[inline]
    fn default() -> Self {
        Self { handle: Arc::new(Default::default()) }
    }
}

/********** impl Reclaim **************************************************************************/

unsafe impl<P: Policy> Reclaim for ArcHp<P> {
    type Header = P::Header;
    type Ref = LocalHandle<'static, 'static, P, Self>;

    #[inline]
    fn new() -> Self {
        Default::default()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hp
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct Hp<P: Policy> {
    state: Global<P>,
}

/********** impl Default **************************************************************************/

impl<P: Policy> Default for Hp<P> {
    #[inline]
    fn default() -> Self {
        Self { state: Global::new() }
    }
}

/********** impl Reclaim **************************************************************************/

unsafe impl<P: Policy> Reclaim for Hp<P> {
    type Header = P::Header;
    type Ref = LocalHandle<'static, 'static, P, Self>;

    #[inline]
    fn new() -> Self {
        Default::default()
    }
}
