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
mod strategy;

use conquer_reclaim::Reclaim;

pub use crate::config::{Config, ConfigBuilder, Operation};
pub use crate::local::{Local, LocalHandle};
pub use crate::strategy::{GlobalRetire, LocalRetire};

use crate::global::{Global, GlobalRef};
use crate::strategy::global_retire::Header;
use crate::strategy::{GlobalRetireState, RetireStrategy};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hp
////////////////////////////////////////////////////////////////////////////////////////////////////

/// The global state for the hazard pointer memory reclamation scheme.
#[derive(Debug)]
pub struct Hp<S> {
    state: Global,
    retire_strategy: S,
}

/********** impl inherent *************************************************************************/

impl<S: RetireStrategy> Hp<S> {
    /// Builds a new instance of a [`Local`] that stores a reference (i.e.
    /// borrows) the internal global state of `self`.
    ///
    /// If `config` wraps a [`Config`] instance this instance is used to
    /// supply the [`Local`]'s internal configuration, otherwise the default
    /// configuration is applied.
    #[inline]
    pub fn build_local(&self, config: Option<Config>) -> Local {
        Local::new(config.unwrap_or_default(), GlobalRef::from_ref(&self.state))
    }

    /// Builds a new instance of a [`Local`] that stores a pointer (i.e. without
    /// borrowing) the internal global state of `self`.
    ///
    /// If `config` wraps a [`Config`] instance this instance is used to
    /// supply the [`Local`]'s internal configuration, otherwise the default
    /// configuration is applied.
    ///
    /// # Safety
    ///
    /// The resulting [`Local`] is not lifetime-dependent on the [`Hp`] instance
    /// it is derived from, which allows e.g. self-referential types.
    /// The caller is required, however, to ensure that the [`Local`] instance
    /// does not outlive `self`.
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

impl Reclaim for Hp<GlobalRetire> {
    // the global retire strategy requires each record to have a specific
    // header.
    type Header = Header;
    type LocalState = LocalHandle<'static, 'static, Self>;

    #[inline]
    unsafe fn build_local_state(&self) -> Self::LocalState {
        LocalHandle::new(Config::default(), GlobalRef::from_raw(&self.state))
    }
}

impl Reclaim for Hp<LocalRetire> {
    type Header = ();
    type LocalState = LocalHandle<'static, 'static, Self>;

    #[inline]
    unsafe fn build_local_state(&self) -> Self::LocalState {
        LocalHandle::new(Config::default(), GlobalRef::from_raw(&self.state))
    }
}
