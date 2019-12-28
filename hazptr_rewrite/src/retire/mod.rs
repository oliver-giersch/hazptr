mod global_retire;
mod local_retire;

use core::fmt::Debug;

use conquer_reclaim::RawRetired;

use crate::global::Global;
use crate::hazard::ProtectedPtr;

pub use self::{global_retire::GlobalRetire, local_retire::LocalRetire};

////////////////////////////////////////////////////////////////////////////////////////////////////
// RetireStrategy (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An internal trait for abstracting over different retire strategies.
pub trait RetireStrategy: Debug + Default + 'static {
    /// The memory record header is dependent on the retire strategy.
    type Header: Default + Sync + Sized;
    /// The global state required for the given retire strategy.
    type Global: Debug + Default + Send + Sync + Sized;

    /// Creates a new strategy instance and optionally accesses the global
    /// state.
    fn new(global: &Global<Self>) -> Self;

    /// Drops the strategy instance and optionally accesses the global state.
    fn drop(self, global: &Global<Self>);
    unsafe fn reclaim_all_unprotected(&mut self, global: &Global<Self>, protected: &[ProtectedPtr]);
    unsafe fn retire(&mut self, global: &Global<Self>, retired: RawRetired);
}
