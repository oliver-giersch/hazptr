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
pub use crate::retire::{GlobalRetire, LocalRetire};

use crate::global::{Global, GlobalRef};
use crate::retire::{GlobalRetireState, RetireStrategy};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hp
////////////////////////////////////////////////////////////////////////////////////////////////////

/// The hazard pointer memory reclamation scheme.
#[derive(Debug)]
pub struct Hp<S> {
    state: Global,
    retire_strategy: S,
}

/********** impl inherent *************************************************************************/

impl<S: RetireStrategy> Hp<S> {
    #[inline]
    pub fn build_local(&self, config: Option<Config>) -> Local {
        Local::new(config.unwrap_or_default(), GlobalRef::from_ref(&self.state))
    }

    #[inline]
    pub unsafe fn build_local_unchecked(&self, config: Option<Config>) -> Local<'_> {
        Local::new(config.unwrap_or_default(), GlobalRef::from_raw(&self.state))
    }
}

/********** impl Default **************************************************************************/

impl Default for Hp<GlobalRetire> {
    #[inline]
    fn default() -> Self {
        Self {
            state: Global::new(GlobalRetireState::global_strategy()),
            retire_strategy: GlobalRetire,
        }
    }
}

impl Default for Hp<LocalRetire> {
    #[inline]
    fn default() -> Self {
        Self {
            state: Global::new(GlobalRetireState::local_strategy()),
            retire_strategy: LocalRetire,
        }
    }
}

/********** impl Reclaim **************************************************************************/

unsafe impl Reclaim for Hp<GlobalRetire> {
    // the header type depends on the retire strategy
    type Header = crate::retire::global_retire::Header;
    type Ref = LocalHandle<'static, 'static, Self>;

    #[inline]
    fn new() -> Self {
        Default::default()
    }
}

unsafe impl Reclaim for Hp<LocalRetire> {
    type Header = ();
    type Ref = LocalHandle<'static, 'static, Self>;

    #[inline]
    fn new() -> Self {
        Default::default()
    }
}
