use core::cmp;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        use std::boxed::Box;
        use std::vec::Vec;
    } else {
        use alloc::boxed::Box;
        use alloc::vec::Vec;
    }
}

use conquer_reclaim::RawRetired;

use crate::hazard::ProtectedPtr;
use crate::policy::Policy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalRetire
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Default)]
pub struct LocalRetire(Box<Vec<ReclaimOnDrop>>);

/********** impl Policy ***************************************************************************/

impl Policy for LocalRetire {
    type Header = ();
    type GlobalState = (); // AbandonedBags, ...

    fn new(global: &Self::GlobalState) -> Option<Self> {
        // try adopt abandoned records and use as own
        unimplemented!()
    }

    fn drop(self) {
        // add own records to abandoned ones
        unimplemented!()
    }

    unsafe fn reclaim_all_unprotected(
        &mut self,
        global: &Self::GlobalState,
        protected: &[ProtectedPtr],
    ) {
        self.0.retain(|retired| {
            // retain (i.e. DON'T drop) all records found within the scan cache of protected hazards
            protected.binary_search_by(|&protected| retired.compare_with(protected)).is_ok()
        });
    }

    unsafe fn retire(&mut self, global: &Self::GlobalState, retired: RawRetired) {
        self.0.push(unsafe { ReclaimOnDrop::new(retired) });
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// ReclaimOnDrop
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
struct ReclaimOnDrop(RawRetired);

/********** impl inherent *************************************************************************/

impl ReclaimOnDrop {
    #[inline]
    unsafe fn new(retired: RawRetired) -> Self {
        Self(retired)
    }

    #[inline]
    fn compare_with(&self, protected: ProtectedPtr) -> cmp::Ordering {
        protected.address().cmp(&self.0.address())
    }
}

/********** impl Drop *****************************************************************************/

impl Drop for ReclaimOnDrop {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe { self.0.reclaim() };
    }
}
