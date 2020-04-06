use core::sync::atomic::Ordering;

use conquer_reclaim::conquer_pointer::{MarkedNonNull, MarkedPtr};
use conquer_reclaim::typenum::Unsigned;
use conquer_reclaim::{Atomic, NotEqual, Protect, Protected, Reclaim};

use crate::hazard::{HazardPtr, ProtectStrategy};
use crate::local::LocalRef;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Guard
////////////////////////////////////////////////////////////////////////////////////////////////////

/// The [`Protect`] type representing an acquired hazard pointer.
pub struct Guard<'local, 'global, R> {
    /// The pointer to the acquired `HazardPtr`, the lifetime is implicitly
    /// bound to `'global`.
    hazard: *const HazardPtr,
    /// Each guard contains an e.g. reference-counted local handle which is
    /// accessed when a guard is cloned or dropped.
    local: LocalRef<'local, 'global, R>,
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
            if let Some(protected) = (*source.hazard).protected(Ordering::Relaxed).protected() {
                (*self.hazard).set_protected(protected.into_inner(), Ordering::SeqCst);
            }
        }
    }
}

/********** impl inherent *************************************************************************/

impl<'local, 'global, R> Guard<'local, 'global, R> {
    /// Creates a new guard from a `local` reference.
    #[inline]
    pub fn with_handle(local: LocalRef<'local, 'global, R>) -> Self {
        let hazard = local.as_ref().get_hazard(ProtectStrategy::ReserveOnly);
        Self { hazard, local }
    }

    /// Releases the currently protected hazard pointer.
    ///
    /// A call to `release` increases the threads ops count, if the reclaimer is
    /// configured with the [`Release`][crate::config::CountStrategy::Release]
    /// strategy.
    #[inline]
    pub fn release(&mut self) {
        self.local.as_ref().increase_ops_count_if_count_release();
        unsafe { (*self.hazard).set_thread_reserved(Ordering::Release) };
    }
}

/********** impl Drop *****************************************************************************/

impl<R> Drop for Guard<'_, '_, R> {
    #[inline]
    fn drop(&mut self) {
        let (hazard, local) = (unsafe { &*self.hazard }, self.local.as_ref());
        if local.try_recycle_hazard(hazard).is_err() {
            hazard.set_free(Ordering::Release);
        }

        local.increase_ops_count_if_count_release();
    }
}

/********** impl Protect **************************************************************************/

/// A small short-hand for releasing the hazard pointer and returning a null
/// pointer.
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
            Err(_) => release!(self, ptr),
            Ok(ptr) => {
                let mut protect = ptr.decompose_non_null();
                loop {
                    unsafe { (*self.hazard).set_protected(protect.cast(), Ordering::SeqCst) };
                    let ptr = atomic.load_raw(order);
                    match MarkedNonNull::new(ptr) {
                        Err(_) => return release!(self, ptr),
                        Ok(ptr) => {
                            let cmp = ptr.decompose_non_null();
                            if protect == cmp {
                                // safety: `ptr` is now guaranteed to be protected by the memory
                                // reclamation scheme
                                return unsafe { Protected::from_marked_non_null(ptr) };
                            }

                            protect = cmp;
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
