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
mod queue;
mod retire;

use conquer_reclaim::Reclaim;

pub use crate::config::{Config, ConfigBuilder};

use crate::global::Global;
use crate::local::LocalHandle;
use crate::retire::RetireStrategy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hp
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct Hp<S: RetireStrategy> {
    state: Global<S>,
}

/********** impl Default **************************************************************************/

impl<S: RetireStrategy> Default for Hp<S> {
    #[inline]
    fn default() -> Self {
        Self { state: Global::new() }
    }
}

/********** impl Reclaim **************************************************************************/

unsafe impl<S: RetireStrategy> Reclaim for Hp<S> {
    type Header = S::Header;
    type Ref = LocalHandle<'static, 'static, S, Self>;

    #[inline]
    fn new() -> Self {
        Default::default()
    }
}
