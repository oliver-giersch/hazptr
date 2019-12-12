use core::sync::atomic::Ordering;

use conquer_reclaim::conquer_pointer::{
    MarkedPtr,
    MaybeNull::{self, NotNull, Null},
};
use conquer_reclaim::typenum::Unsigned;
use conquer_reclaim::{Atomic, NotEqualError, Protect, Reclaimer, Shared};

use crate::hazard::{Hazard, ProtectStrategy};
use crate::local::LocalHandle;
use crate::policy::Policy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Guard
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct Guard<'local, 'global, P: Policy, R: Reclaimer> {
    hazard: *const Hazard,
    local: LocalHandle<'local, 'global, P, R>,
}

/********** impl Clone ****************************************************************************/

impl<'local, 'global, P: Policy, R: Reclaimer> Clone for Guard<'local, 'global, P, R> {
    #[inline]
    fn clone(&self) -> Self {
        let local = self.local.clone();
        match unsafe { (*self.hazard).protected(Ordering::Relaxed) } {
            Some(protected) => Self {
                hazard: local.as_ref().get_hazard(ProtectStrategy::Protect(protected)),
                local,
            },
            None => Self { hazard: local.as_ref().get_hazard(ProtectStrategy::ReserveOnly), local },
        }
    }

    #[inline]
    fn clone_from(&mut self, source: &Self) {
        unsafe {
            // TODO: is relaxed enough?
            if let Some(protected) = (*source.hazard).protected(Ordering::Relaxed) {
                (*self.hazard).set_protected(protected.into_inner(), Ordering::SeqCst);
            }
        }
    }
}

/********** impl inherent *************************************************************************/

impl<'local, 'global, P: Policy, R: Reclaimer> Guard<'local, 'global, P, R> {
    #[inline]
    pub fn with_handle(local: LocalHandle<'local, 'global, P, R>) -> Self {
        let hazard = local.as_ref().get_hazard(ProtectStrategy::ReserveOnly);
        Self { hazard, local }
    }
}

/********** impl Drop *****************************************************************************/

impl<'local, 'global, P: Policy, R: Reclaimer> Drop for Guard<'local, 'global, P, R> {
    #[inline]
    fn drop(&mut self) {
        // TODO: increase count, perhaps based on self.local.as_ref().config ?
        let hazard = unsafe { &*self.hazard };
        if self.local.as_ref().try_recycle_hazard(hazard).is_err() {
            hazard.set_free(Ordering::Release);
        }
    }
}

/********** impl Protect **************************************************************************/

macro_rules! release {
    ($self:ident, $tag:expr) => {{
        $self.release();
        Null($tag)
    }};
}

unsafe impl<P: Policy, R: Reclaimer> Protect for Guard<'_, '_, P, R> {
    type Reclaimer = R;

    #[inline]
    fn release(&mut self) {
        unsafe { (*self.hazard) }.set_thread_reserved(Ordering::Release);
    }

    #[inline]
    fn protect<T, N: Unsigned>(
        &mut self,
        src: &Atomic<T, Self::Reclaimer, N>,
        order: Ordering,
    ) -> MaybeNull<Shared<T, Self::Reclaimer, N>> {
        match MaybeNull::from(src.load_raw(Ordering::Relaxed)) {
            Null(tag) => return release!(self, tag),
            NotNull(ptr) => {
                let mut protect = ptr.decompose_non_null();
                unsafe { *self.hazard }.set_protected(protect.cast(), Ordering::SeqCst);

                loop {
                    match MaybeNull::from(src.load_raw(order)) {
                        Null(tag) => return release!(self, tag),
                        NotNull(ptr) => {
                            let temp = ptr.decompose_non_null();
                            if protect == temp {
                                return NotNull(unsafe { Shared::from_marked_non_null(ptr) });
                            }

                            unsafe { *self.hazard }.set_protected(temp.cast(), Ordering::SeqCst);
                            protect = temp;
                        }
                    }
                }
            }
        }
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
