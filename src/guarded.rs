use core::sync::atomic::Ordering::{self, Relaxed, Release, SeqCst};

use reclaim::prelude::*;
use reclaim::typenum::Unsigned;
use reclaim::{MarkedNonNull, MarkedPtr, NotEqualError};

use crate::hazard::Hazard;
use crate::local::LocalAccess;
use crate::{Atomic, Shared, HP};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Guarded
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A guarded pointer that can be used to acquire hazard pointers.
#[derive(Debug)]
pub struct Guard<L: LocalAccess> {
    hazard: &'static Hazard,
    local_access: L,
}

unsafe impl<L: LocalAccess + Send> Send for Guard<L> {}

impl<L: LocalAccess> Clone for Guard<L> {
    #[inline]
    fn clone(&self) -> Self {
        let local_access = self.local_access;
        match self.hazard.protected(Relaxed) {
            Some(protect) => {
                Self { hazard: local_access.get_hazard(Some(protect.into_inner())), local_access }
            }
            None => Self { hazard: local_access.get_hazard(None), local_access },
        }
    }
}

// a small shorthand for a one-line return statement
macro_rules! release {
    ($self:ident, $tag:expr) => {{
        // (GUA:1) this `Release` store synchronizes-with ...
        $self.hazard.set_thread_reserved(Release);
        Null($tag)
    }};
}

unsafe impl<L: LocalAccess> Protect for Guard<L> {
    type Reclaimer = HP;

    #[inline]
    fn release(&mut self) {
        // (GUA:2) this `Release` store synchronizes-with ...
        self.hazard.set_thread_reserved(Release);
    }

    #[inline]
    fn protect<T, N: Unsigned>(
        &mut self,
        atomic: &Atomic<T, N>,
        order: Ordering,
    ) -> Marked<Shared<T, N>> {
        match MarkedNonNull::new(atomic.load_raw(Relaxed)) {
            Null(tag) => return release!(self, tag),
            Value(ptr) => {
                let mut protect = ptr.decompose_non_null();
                // (GUA:2 this `SeqCst` store synchronizes-with the `SeqCst` fence (LOC:2) and the
                // `SeqCst` CAS (LIS:3P).
                self.hazard.set_protected(protect.cast(), SeqCst);

                loop {
                    match MarkedNonNull::new(atomic.load_raw(order)) {
                        Null(tag) => return release!(self, tag),
                        Value(ptr) => {
                            let unmarked = ptr.decompose_non_null();
                            if protect == unmarked {
                                return Value(unsafe { Shared::from_marked_non_null(ptr) });
                            }

                            // (GUA:3) this `SeqCst` store synchronizes-with the `SeqCst` fence
                            // (LOC:2) and the SeqCst` CAS (LIS:3P).
                            self.hazard.set_protected(unmarked.cast(), SeqCst);
                            protect = unmarked;
                        }
                    }
                }
            }
        }
    }

    #[inline]
    fn protect_if_equal<T, N: Unsigned>(
        &mut self,
        atomic: &Atomic<T, N>,
        expected: MarkedPtr<T, N>,
        order: Ordering,
    ) -> Result<Marked<Shared<T, N>>, NotEqualError> {
        let raw = atomic.load_raw(Relaxed);
        if raw != expected {
            return Err(NotEqualError);
        }

        match MarkedNonNull::new(atomic.load_raw(order)) {
            Null(tag) => Ok(release!(self, tag)),
            Value(ptr) => {
                let unmarked = ptr.decompose_non_null();
                // (GUA:4) this `SeqCst` store synchronizes-with the `SeqCst` fence (LOC:2) and the
                // `SeqCst` CAS (LIS:3P).
                self.hazard.set_protected(unmarked.cast(), SeqCst);

                if atomic.load_raw(order) != ptr {
                    // (GUA:5) this `Release` store synchronizes-with ...
                    self.hazard.set_thread_reserved(Release);
                    Err(NotEqualError)
                } else {
                    Ok(unsafe { Marked::from_marked_non_null(ptr) })
                }
            }
        }
    }
}

impl<L: LocalAccess> Guard<L> {
    /// Creates a new [`Guard`] with the given means for `local_access`.
    #[inline]
    pub fn with_access(local_access: L) -> Self {
        Self { hazard: local_access.get_hazard(None), local_access }
    }
}

impl<L: LocalAccess> Drop for Guard<L> {
    #[inline]
    fn drop(&mut self) {
        if cfg!(feature = "count-release") {
            self.local_access.increase_ops_count();
        }

        if self.local_access.try_recycle_hazard(self.hazard).is_err() {
            self.hazard.set_free(Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering::Relaxed;

    use matches::assert_matches;

    use reclaim::prelude::*;
    use reclaim::typenum::U0;

    use crate::guarded::Guard;
    use crate::local::Local;
    use crate::Shared;

    type Atomic = crate::Atomic<i32, U0>;
    type Owned = crate::Owned<i32, U0>;
    type MarkedPtr = reclaim::MarkedPtr<i32, U0>;

    #[test]
    fn new() {
        let local = Local::new();
        let guard = Guard::with_access(&local);
        assert!(guard.hazard.protected(Relaxed).is_none());
    }

    #[test]
    fn protect() {
        let local = Local::new();
        let mut guard = Guard::with_access(&local);

        let null = Atomic::null();
        let marked = guard.protect(&null, Relaxed);
        assert_matches!(marked, Null(0));
        assert!(guard.hazard.protected(Relaxed).is_none());

        let atomic = Atomic::new(1);
        let shared = guard.protect(&atomic, Relaxed).unwrap_value();
        let reference = Shared::into_ref(shared);
        let addr = reference as *const _ as usize;
        assert_eq!(reference, &1);
        assert_eq!(guard.hazard.protected(Relaxed).unwrap().address(), addr);

        let _ = guard.protect(&null, Relaxed);
        assert!(guard.hazard.protected(Relaxed).is_none());
    }

    #[test]
    fn protect_if_equal() {
        let local = Local::new();
        let mut guard = Guard::with_access(&local);

        let null = Atomic::null();
        let null_ptr = MarkedPtr::null();

        let res = guard.protect_if_equal(&null, null_ptr, Relaxed);
        assert_matches!(res, Ok(Null(0)));
        assert!(guard.hazard.protected(Relaxed).is_none());

        let owned = Owned::new(1);
        let marked = Owned::as_marked_ptr(&owned);
        let atomic = Atomic::from(owned);

        let res = guard.protect_if_equal(&atomic, null_ptr, Relaxed);
        assert_matches!(res, Err(_));
        assert!(guard.hazard.protected(Relaxed).is_none());

        let res = guard.protect_if_equal(&atomic, marked, Relaxed);
        let shared = res.unwrap().unwrap_value();
        let reference = Shared::into_ref(shared);
        assert_eq!(reference, &1);
        assert_eq!(guard.hazard.protected(Relaxed).unwrap().address(), marked.into_usize());

        // a failed protection attempt must not alter the previous state
        let res = guard.protect_if_equal(&null, marked, Relaxed);
        assert!(res.is_err());
        assert_eq!(guard.hazard.protected(Relaxed).unwrap().address(), marked.into_usize());

        let res = guard.protect_if_equal(&null, null_ptr, Relaxed);
        assert_matches!(res, Ok(Null(0)));
        assert!(guard.hazard.protected(Relaxed).is_none());
    }
}
