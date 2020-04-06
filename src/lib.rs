#![cfg_attr(not(any(test, feature = "std")), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "global")]
mod default;

mod config;
mod global;
mod guard;
mod hazard;
mod local;
mod queue;
mod strategy;

pub use conquer_reclaim;
pub use conquer_reclaim::typenum;

use conquer_reclaim::Reclaim;

pub use crate::config::{Config, ConfigBuilder, CountStrategy};
#[cfg(feature = "global")]
pub use crate::default::{build_guard, retire_record, GlobalHp, GlobalHpRef, CONFIG};
pub use crate::local::{Local, LocalRef};
pub use crate::strategy::{GlobalRetire, LocalRetire};

use crate::global::{Global, GlobalRef};
use crate::strategy::{GlobalRetireState, RetireStrategy};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hp
////////////////////////////////////////////////////////////////////////////////////////////////////

/// The global state for the hazard pointer memory reclamation scheme.
#[derive(Debug)]
pub struct Hp<S = LocalRetire> {
    /// The reclaimer configuration.
    config: Config,
    /// The global state.
    state: Global,
    /// The retire strategy.
    retire_strategy: S,
}

/********** impl inherent *************************************************************************/

impl Hp<GlobalRetire> {
    /// Creates a new `Hp` instance with the given `config`.
    #[inline]
    pub const fn global_retire(config: Config) -> Self {
        Self {
            config,
            state: Global::new(GlobalRetireState::global_strategy()),
            retire_strategy: GlobalRetire,
        }
    }
}

impl Hp<LocalRetire> {
    /// Creates a new `Hp` instance with the given `config`.
    #[inline]
    pub const fn local_retire(config: Config) -> Self {
        Self {
            config,
            state: Global::new(GlobalRetireState::local_strategy()),
            retire_strategy: LocalRetire,
        }
    }
}

impl<S: RetireStrategy> Hp<S> {
    /// Builds a new instance of a [`Local`] that stores a reference (i.e.
    /// borrows) the internal global state of `self`.
    ///
    /// If `config` wraps a [`Config`] instance this instance is used to
    /// supply the [`Local`]'s internal configuration, otherwise the default
    /// configuration is applied.
    #[inline]
    pub fn build_local(&self, config: Option<Config>) -> Local<'_, Self> {
        Local::new(config.unwrap_or(self.config), GlobalRef::from_ref(&self.state))
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
    pub unsafe fn build_local_unchecked(&self, config: Option<Config>) -> Local<'_, Self> {
        Local::new(config.unwrap_or(self.config), GlobalRef::from_raw(&self.state))
    }
}

/********** impl Default **************************************************************************/

impl Default for Hp<GlobalRetire> {
    #[inline]
    fn default() -> Self {
        Self::global_retire(Config::default())
    }
}

impl Default for Hp<LocalRetire> {
    #[inline]
    fn default() -> Self {
        Self::local_retire(Config::default())
    }
}

/********** impl Reclaim **************************************************************************/

impl Reclaim for Hp<GlobalRetire> {
    type Header = crate::strategy::global_retire::Header;
    type LocalState = LocalRef<'static, 'static, Self>;

    #[inline]
    unsafe fn build_local_state(&self) -> Self::LocalState {
        LocalRef::owning(self.config, GlobalRef::from_raw(&self.state))
    }
}

impl Reclaim for Hp<LocalRetire> {
    type Header = ();
    type LocalState = LocalRef<'static, 'static, Self>;

    #[inline]
    unsafe fn build_local_state(&self) -> Self::LocalState {
        LocalRef::owning(self.config, GlobalRef::from_raw(&self.state))
    }
}
