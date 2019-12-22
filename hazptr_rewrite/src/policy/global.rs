use conquer_reclaim::RawRetired;

use crate::hazard::ProtectedPtr;
use crate::policy::Policy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalRetire
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Default)]
pub struct GlobalRetire;

/********** impl Policy ***************************************************************************/

impl Policy for GlobalRetire {
    type Header = (); // AnyNodePtr
    type GlobalState = (); // Queue<Retired>, ...

    fn new(global: &Self::GlobalState) -> Option<Self> {
        unimplemented!()
    }

    fn drop(self) {
        unimplemented!()
    }

    unsafe fn reclaim_all_unprotected(
        &mut self,
        global: &Self::GlobalState,
        protected: &[ProtectedPtr],
    ) {
        unimplemented!()
    }

    unsafe fn retire(&mut self, global: &Self::GlobalState, retired: RawRetired) {
        unimplemented!()
    }
}
