use core::sync::atomic::Ordering;

use conquer_reclaim::conquer_pointer::{
    MarkedPtr,
    MaybeNull::{self, NotNull, Null},
};
use conquer_reclaim::typenum::Unsigned;
use conquer_reclaim::{Atomic, NotEqualError, Protect, Reclaim, Shared};

use crate::config::Operation;
use crate::hazard::{HazardPtr, ProtectStrategy};
use crate::local::LocalHandle;
use crate::retire::RetireStrategy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Guard
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct Guard<'local, 'global, S: RetireStrategy, R: Reclaim> {
    /// Hazards are borrowed through the local handle from global state, so they
    /// act like `'global` references.
    hazard: *const HazardPtr,
    /// Each guard contains an e.g. reference-counted local handle which is
    /// accessed when a guard is cloned or dropped.
    local: LocalHandle<'local, 'global, S, R>,
}

/********** impl Clone ****************************************************************************/

impl<'local, 'global, S: RetireStrategy, R: Reclaim> Clone for Guard<'local, 'global, S, R> {
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

impl<'local, 'global, S: RetireStrategy, R: Reclaim> Guard<'local, 'global, S, R> {
    #[inline]
    pub fn with_handle(local: LocalHandle<'local, 'global, S, R>) -> Self {
        let hazard = local.as_ref().get_hazard(ProtectStrategy::ReserveOnly);
        Self { hazard, local }
    }
}

/********** impl Drop *****************************************************************************/

impl<'local, 'global, S: RetireStrategy, R: Reclaim> Drop for Guard<'local, 'global, S, R> {
    #[inline]
    fn drop(&mut self) {
        let local = self.local.as_ref();
        local.try_increase_ops_count(Operation::Release);
        let hazard = unsafe { &*self.hazard };
        if local.try_recycle_hazard(hazard).is_err() {
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

unsafe impl<S: RetireStrategy, R: Reclaim> Protect for Guard<'_, '_, S, R> {
    type Reclaimer = R;

    #[inline]
    fn release(&mut self) {
        self.local.as_ref().try_increase_ops_count(Operation::Release);
        unsafe { (*self.hazard).set_thread_reserved(Ordering::Release) };
    }

    #[inline]
    fn protect<T, N: Unsigned + 'static>(
        &mut self,
        src: &Atomic<T, Self::Reclaimer, N>,
        order: Ordering,
    ) -> MaybeNull<Shared<T, Self::Reclaimer, N>> {
        match MaybeNull::from(src.load_raw(Ordering::Relaxed)) {
            Null(tag) => release!(self, tag),
            NotNull(ptr) => {
                let mut protect = ptr.decompose_non_null();
                unsafe { (*self.hazard).set_protected(protect.cast(), Ordering::SeqCst) };

                loop {
                    match MaybeNull::from(src.load_raw(order)) {
                        Null(tag) => return release!(self, tag),
                        NotNull(ptr) => {
                            let temp = ptr.decompose_non_null();
                            if protect == temp {
                                return NotNull(unsafe { Shared::from_marked_non_null(ptr) });
                            }

                            unsafe { (*self.hazard).set_protected(temp.cast(), Ordering::SeqCst) };
                            protect = temp;
                        }
                    }
                }
            }
        }
    }

    #[inline]
    fn protect_if_equal<T, N: Unsigned + 'static>(
        &mut self,
        src: &Atomic<T, Self::Reclaimer, N>,
        expected: MarkedPtr<T, N>,
        order: Ordering,
    ) -> Result<MaybeNull<Shared<T, Self::Reclaimer, N>>, NotEqualError> {
        let raw = src.load_raw(order);
        if raw != expected {
            return Err(NotEqualError);
        }

        match MaybeNull::from(raw) {
            Null(tag) => Ok(release!(self, tag)),
            NotNull(ptr) => {
                let protect = ptr.decompose_non_null().cast();
                unsafe { (*self.hazard).set_protected(protect, Ordering::SeqCst) };

                if src.load_raw(order) == ptr.into_marked_ptr() {
                    Ok(NotNull(unsafe { Shared::from_marked_non_null(ptr) }))
                } else {
                    unsafe { (*self.hazard).set_thread_reserved(Ordering::Release) };
                    Err(NotEqualError)
                }
            }
        }
    }
}
