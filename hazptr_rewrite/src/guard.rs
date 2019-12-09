use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use conquer_reclaim::{Atomic, NotEqualError, Protect, ReclaimerHandle, Shared};

use crate::hazard::Hazard;
use crate::local::{Local, LocalHandle};
use crate::policy::Policy;
use crate::HPHandle;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Guard
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct Guard<'local, 'global, P> {
    hazard: &'global Hazard,
    local: LocalHandle<'local, 'global, P>,
}

/********** impl inherent *************************************************************************/

impl<'local, 'global, P: Policy> Guard<'local, 'global, P> {
    #[inline]
    pub fn with_handle(local: LocalHandle<'local, 'global, P>) -> Self {
        let hazard = local.as_ref().get_hazard();
        Self { hazard, local }
    }
}

/********** impl Clone ****************************************************************************/

impl<'local, 'global, P: Policy> Clone for Guard<'local, 'global, P> {
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

impl<'local, 'global, P: Policy> Drop for Guard<'local, 'global, P> {
    #[inline]
    fn drop(&mut self) {
        unimplemented!()
    }
}

/********** impl Protect **************************************************************************/

unsafe impl<P: Policy> Protect for Guard<'_, '_, P> {
    type Reclaimer = HPHandle<P>;

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
    ) -> MarkedOption<Shared<T, Self::Reclaimer, N>> {
        unimplemented!()
    }

    #[inline]
    fn protect_if_equal<T, N: Unsigned>(
        &mut self,
        src: &Atomic<T, Self::Reclaimer, N>,
        expected: MarkedPtr<T, N>,
        order: Ordering,
    ) -> Result<MarkedOption<Shared<T, Self::Reclaimer, N>>, NotEqualError> {
        unimplemented!()
    }
}
