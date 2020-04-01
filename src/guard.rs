use core::sync::atomic::Ordering;

use conquer_reclaim::conquer_pointer::{MarkedNonNull, MarkedPtr, Null};
use conquer_reclaim::typenum::Unsigned;
use conquer_reclaim::{Atomic, NotEqual, Protect, Protected, Reclaim};

use crate::config::Operation;
use crate::hazard::{HazardPtr, ProtectStrategy};
use crate::local::LocalHandle;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Guard
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct Guard<'local, 'global, R> {
    /// Hazards are borrowed through the local handle from global state, so they
    /// act like `'global` references.
    hazard: *const HazardPtr,
    /// Each guard contains an e.g. reference-counted local handle which is
    /// accessed when a guard is cloned or dropped.
    local: LocalHandle<'local, 'global, R>,
}

/********** impl Clone ****************************************************************************/

impl<R> Clone for Guard<'_, '_, R> {
    #[inline]
    fn clone(&self) -> Self {
        let local = self.local.clone();
        let hazard = match unsafe { (*self.hazard).protected(Ordering::Relaxed).protected() } {
            Some(protected) => local.as_ref().get_hazard(ProtectStrategy::Protect(protected)),
            None => local.as_ref().get_hazard(ProtectStrategy::ReserveOnly),
        };

        Self { hazard, local }
    }

    #[inline]
    fn clone_from(&mut self, source: &Self) {
        unsafe {
            // TODO: is relaxed enough?
            if let Some(protected) = (*source.hazard).protected(Ordering::Relaxed).protected() {
                (*self.hazard).set_protected(protected.into_inner(), Ordering::SeqCst);
            }
        }
    }
}

/********** impl inherent *************************************************************************/

impl<'local, 'global, R> Guard<'local, 'global, R> {
    #[inline]
    pub fn with_handle(local: LocalHandle<'local, 'global, R>) -> Self {
        let hazard = local.as_ref().get_hazard(ProtectStrategy::ReserveOnly);
        Self { hazard, local }
    }

    #[inline]
    pub fn release(&mut self) {
        self.local.as_ref().try_increase_ops_count(Operation::Release);
        unsafe { (*self.hazard).set_thread_reserved(Ordering::Release) };
    }
}

/********** impl Drop *****************************************************************************/

impl<R> Drop for Guard<'_, '_, R> {
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
    ($self:ident, $ptr:expr) => {{
        $self.release();
        unsafe { Protected::from_marked_ptr($ptr) }
    }};
}

unsafe impl<R: Reclaim> Protect for Guard<'_, '_, R> {
    type Reclaimer = R;

    #[inline]
    fn protect<T, N: Unsigned>(
        &mut self,
        atomic: &Atomic<T, Self::Reclaimer, N>,
        order: Ordering,
    ) -> Protected<T, Self::Reclaimer, N> {
        let ptr = atomic.load_raw(Ordering::Relaxed);
        match MarkedNonNull::new(ptr) {
            Err(Null(tag)) => release!(self, ptr),
            Ok(ptr) => {
                let mut protect = ptr.decompose_non_null();
                unsafe { (*self.hazard).set_protected(protect.cast(), Ordering::SeqCst) };

                loop {
                    let ptr = atomic.load_raw(order);
                    match MarkedNonNull::new(ptr) {
                        Err(Null(tag)) => return release!(self, ptr),
                        Ok(ptr) => {
                            let compare = ptr.decompose_non_null();
                            if protect == compare {
                                return unsafe { Protected::from_marked_non_null(ptr) };
                            }

                            unsafe {
                                (*self.hazard).set_protected(compare.cast(), Ordering::SeqCst)
                            };
                            protect = compare;
                        }
                    }
                }
            }
        }
    }

    #[inline]
    fn protect_if_equal<T, N: Unsigned>(
        &mut self,
        atomic: &Atomic<T, Self::Reclaimer, N>,
        expected: MarkedPtr<T, N>,
        order: Ordering,
    ) -> Result<Protected<T, Self::Reclaimer, N>, NotEqual> {
        let ptr = atomic.load_raw(Ordering::Relaxed);
        if ptr != expected {
            return Err(NotEqual);
        }

        match MarkedNonNull::new(ptr) {
            Err(_) => Ok(release!(self, ptr)),
            Ok(ptr) => {
                let protect = ptr.decompose_non_null().cast();
                unsafe { (*self.hazard).set_protected(protect, Ordering::SeqCst) };

                if atomic.load_raw(order) == ptr.into_marked_ptr() {
                    Ok(unsafe { Protected::from_marked_non_null(ptr) })
                } else {
                    unsafe { (*self.hazard).set_thread_reserved(Ordering::Release) };
                    Err(NotEqual)
                }
            }
        }
    }
}
