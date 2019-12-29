mod global_retire;
mod local_retire;

use core::fmt::Debug;

use conquer_reclaim::RawRetired;

use crate::hazard::ProtectedPtr;

pub use self::{global_retire::GlobalRetire, local_retire::LocalRetire};

////////////////////////////////////////////////////////////////////////////////////////////////////
// RetireStrategy (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An internal trait for abstracting over different retire strategies.
pub trait RetireStrategy: Debug + Default + Send + Sync + Sized + 'static {
    /// The memory record header is dependent on the retire strategy.
    type Header: Default + Sync + Sized;
    /// The local state required for the given retire strategy.
    type Local: Debug + Default + 'static;

    /// Creates a new strategy instance and optionally accesses the global
    /// state.
    fn build_local(&self) -> Self::Local;

    /// Drops the strategy instance and optionally accesses the global state.
    fn on_thread_exit(&self, local: Self::Local);

    fn has_retired_records(&self, local: &Self::Local) -> bool;

    unsafe fn reclaim_all_unprotected(&self, local: &mut Self::Local, protected: &[ProtectedPtr]);

    unsafe fn retire(&self, local: &mut Self::Local, retired: RawRetired);
}
