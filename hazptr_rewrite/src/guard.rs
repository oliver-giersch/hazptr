use core::sync::atomic::Ordering;

use conquer_reclaim::conquer_pointer::{MarkedPtr, MaybeNull};
use conquer_reclaim::typenum::Unsigned;
use conquer_reclaim::{Atomic, NotEqualError, Protect, Reclaimer, Shared};

use crate::hazard::Hazard;
use crate::local::LocalHandle;
use crate::policy::Policy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Guard
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct Guard<'local, 'global, P: Policy, R: Reclaimer> {
    hazard: *const Hazard,
    local: LocalHandle<'local, 'global, P, R>,
}

/********** impl inherent *************************************************************************/

impl<'local, 'global, P: Policy, R: Reclaimer> Guard<'local, 'global, P, R> {
    #[inline]
    pub fn with_handle(local: LocalHandle<'local, 'global, P, R>) -> Self {
        let hazard = local.as_ref().get_hazard();
        Self { hazard, local }
    }
}

/********** impl Clone ****************************************************************************/

impl<'local, 'global, P: Policy, R: Reclaimer> Clone for Guard<'local, 'global, P, R> {
    #[inline]
    fn clone(&self) -> Self {
        unimplemented!()
    }

    #[inline]
    fn clone_from(&mut self, source: &Self) {
        // let hazard protect source.hazard
        unimplemented!()
    }
}

/********** impl Drop *****************************************************************************/

impl<'local, 'global, P: Policy, R: Reclaimer> Drop for Guard<'local, 'global, P, R> {
    #[inline]
    fn drop(&mut self) {
        unimplemented!()
    }
}

/********** impl Protect **************************************************************************/

unsafe impl<P: Policy, R: Reclaimer> Protect for Guard<'_, '_, P, R> {
    type Reclaimer = R;

    #[inline]
    fn release(&mut self) {
        // unsafe { *(self.hazard).set_thread_reserved(Release) };
        unimplemented!()
    }

    #[inline]
    fn protect<T, N: Unsigned>(
        &mut self,
        src: &Atomic<T, Self::Reclaimer, N>,
        order: Ordering,
    ) -> MaybeNull<Shared<T, Self::Reclaimer, N>> {
        unimplemented!()
    }

    #[inline]
    fn protect_if_equal<T, N: Unsigned>(
        &mut self,
        src: &Atomic<T, Self::Reclaimer, N>,
        expected: MarkedPtr<T, N>,
        order: Ordering,
    ) -> Result<MaybeNull<Shared<T, Self::Reclaimer, N>>, NotEqualError> {
        unimplemented!()
    }
}
