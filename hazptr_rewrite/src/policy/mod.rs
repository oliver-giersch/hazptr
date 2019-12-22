mod global;
mod local;

use core::fmt::Debug;

use conquer_reclaim::RawRetired;

use crate::hazard::ProtectedPtr;

pub use self::{global::GlobalRetire, local::LocalRetire};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Policy (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An internal trait for abstracting over different retire policies.
pub trait Policy: Debug + Default + 'static {
    /// The memory record header is dependent on the retire policy.
    type Header: Default + Sync + Sized;

    type GlobalState: Debug + Default + Send + Sync;

    fn new(global: &Self::GlobalState) -> Option<Self>;
    fn drop(self);
    unsafe fn reclaim_all_unprotected(
        &mut self,
        global: &Self::GlobalState,
        protected: &[ProtectedPtr],
    );
    unsafe fn retire(&mut self, global: &Self::GlobalState, retired: RawRetired);
}
