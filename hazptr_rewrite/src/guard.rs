use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use conquer_reclaim::{NotEqualError, Protect, ReclaimHandle, Shared};

use crate::hazard::Hazard;
use crate::local::{Local, LocalHandle};
use crate::policy::Policy;
use crate::HP;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Guard
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct Guard<'local, 'global, P> {
    hazard: *const Hazard,
    local: LocalHandle<'local, 'global, P>,
}

impl<P: Policy> Guard<'_, '_, P> {}

unsafe impl<P: Policy> Protect for Guard<'_, '_, P> {
    type Reclaimer = HP<P>;

    #[inline]
    fn release(&mut self) {
        // unsafe { *(self.hazard).set_thread_reserved(Release) };
        unimplemented!()
    }

    fn protect<T, N: Unsigned>(
        &mut self,
        src: &Atomic<T, Self::Reclaimer, N>,
        order: Ordering,
    ) -> MarkedOption<Shared<T, Self::Reclaimer, N>> {
        unimplemented!()
    }

    fn protect_if_equal<T, N: Unsigned>(
        &mut self,
        src: &Atomic<T, Self::Reclaimer, N>,
        expected: MarkedPtr<T, N>,
        order: Ordering,
    ) -> Result<MarkedOption<Shared<T, Self::Reclaimer, N>>, NotEqualError> {
        unimplemented!()
    }
}
