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

pub use crate::config::{Config, ConfigBuilder, Operation};
pub use crate::local::{Local, LocalHandle};
pub use crate::retire::{GlobalRetire, LocalRetire, RetireStrategy};

use crate::global::{Global, GlobalHandle};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hp
////////////////////////////////////////////////////////////////////////////////////////////////////

/// The hazard pointer memory reclamation scheme.
#[derive(Debug)]
pub struct Hp<S> {
    state: Global<S>,
}

/********** impl inherent *************************************************************************/

impl<S: RetireStrategy> Hp<S> {
    #[inline]
    pub fn build_local(&self, config: Option<Config>) -> Local<S> {
        Local::new(config.unwrap_or_default(), GlobalHandle::from_ref(&self.state))
    }

    #[inline]
    pub unsafe fn build_local_unchecked(&self, config: Option<Config>) -> Local<'_, S> {
        Local::new(config.unwrap_or_default(), GlobalHandle::from_raw(&self.state))
    }
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
    type Header = S::Header; // the header type depends on the retire strategy
    type Ref = LocalHandle<'static, 'static, S>;

    #[inline]
    fn new() -> Self {
        Default::default()
    }
}
