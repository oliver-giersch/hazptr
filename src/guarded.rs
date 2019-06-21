use core::sync::atomic::Ordering::{self, Relaxed, Release, SeqCst};

use reclaim::prelude::*;
use reclaim::typenum::Unsigned;
use reclaim::{MarkedNonNull, MarkedPtr, NotEqualError};

use crate::hazard::Hazard;
use crate::local::LocalAccess;
use crate::{Atomic, Shared, HP};

type AcquireResult<'g, T, N> = reclaim::AcquireResult<'g, T, HP, N>;

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

macro_rules! release {
    ($self:ident, $tag:expr) => {{
        // (GUA:X) this `Release` store synchronizes-with...
        $self.hazard.set_reserved(Release);
        Null($tag)
    }};
}

unsafe impl<L: LocalAccess> Protect for Guard<L> {
    type Reclaimer = HP;

    #[inline]
    fn protect<T, N: Unsigned>(
        &mut self,
        atomic: &Atomic<T, N>,
        order: Ordering,
    ) -> Marked<Shared<T, N>> {
        match MarkedNonNull::new(atomic.load_raw(Relaxed)) {
            Null(tag) => release!(self, tag),
            Value(ptr) => {
                let mut protect = ptr.decompose_non_null();
                self.hazard.set_protected(protect.cast(), SeqCst);

                loop {
                    match MarkedNonNull::new(atomic.load_raw(order)) {
                        Null(tag) => release!(self, tag),
                        Value(ptr) => {
                            let unmarked = ptr.decompose_non_null();
                            if protect == unmarked {
                                return Value(unsafe { Shared::from_marked_non_null(ptr) });
                            }

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
                self.hazard.set_protected(unmarked.cast(), SeqCst);

                if atomic.load_raw(order) != ptr {
                    self.hazard.set_reserved(Release);
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
    use std::sync::atomic::Ordering;

    use matches::assert_matches;

    use reclaim::prelude::*;
    use reclaim::typenum::U0;

    use crate::local::Local;
    use crate::Shared;

    type Atomic = crate::Atomic<i32, U0>;
    type Owned = crate::Owned<i32, U0>;

    type MarkedPtr = reclaim::MarkedPtr<i32, U0>;

    type Guarded<'a> = super::Guard<i32, &'a Local, U0>;

    #[test]
    fn empty() {
        let local = Local::new();
        let guarded = Guarded::with_access(&local);
        assert!(guarded.hazard.is_none());
        assert!(guarded.marked.is_null());
        assert!(guarded.marked().is_null());
        assert!(guarded.shared().is_none());
    }

    #[test]
    fn acquire() {
        let local = Local::new();
        let mut guarded = Guarded::with_access(&local);

        let null = Atomic::null();
        let _ = guarded.acquire(&null, Ordering::Relaxed);
        assert!(guarded.hazard.is_none());
        assert!(guarded.marked.is_null());
        assert!(guarded.marked().is_null());
        assert!(guarded.shared().is_none());

        let atomic = Atomic::new(1);
        let _ = guarded.acquire(&atomic, Ordering::Relaxed);
        assert!(guarded.hazard.is_some());
        assert!(guarded.marked.is_value());
        assert_eq!(guarded.marked().unwrap_value().as_ref(), &1);
        assert_eq!(guarded.shared().unwrap().as_ref(), &1);

        let _ = guarded.acquire(&null, Ordering::Relaxed);
        assert!(guarded.hazard.is_some());
        assert!(guarded.hazard.unwrap().protected(Ordering::Relaxed).is_none());
        assert!(guarded.marked.is_null());
    }

    #[test]
    fn acquire_if_equal() {
        let local = Local::new();
        let mut guarded = Guarded::with_access(&local);

        let empty = Atomic::null();
        let null = MarkedPtr::null();

        let res = guarded.acquire_if_equal(&empty, null, Ordering::Relaxed);
        assert_matches!(res, Ok(Null(0)));
        assert!(guarded.hazard.is_none());
        assert!(guarded.shared().is_none());

        let owned = Owned::new(1);
        let marked = Owned::as_marked_ptr(&owned);
        let atomic = Atomic::from(owned);

        let res = guarded.acquire_if_equal(&atomic, null, Ordering::Relaxed);
        assert_matches!(res, Err(_));
        assert!(guarded.hazard.is_none());
        assert!(guarded.shared().is_none());

        let res = guarded.acquire_if_equal(&atomic, marked, Ordering::Relaxed);
        assert_matches!(res, Ok(Value(_)));
        assert!(guarded.hazard.is_some());
        let shared = guarded.shared().unwrap();
        assert_eq!(shared.as_ref(), &1);
        assert_eq!(
            Shared::into_marked_ptr(shared).into_usize(),
            guarded.hazard.unwrap().protected(Ordering::Relaxed).unwrap().address()
        );

        // a failed acquire attempt must not alter the previous state
        let res = guarded.acquire_if_equal(&atomic, null, Ordering::Relaxed);
        assert_matches!(res, Err(_));
        assert!(guarded.hazard.is_some());
        assert_eq!(guarded.shared().unwrap().as_ref(), &1);

        let res = guarded.acquire_if_equal(&empty, null, Ordering::Relaxed);
        assert_matches!(res, Ok(Null(0)));
        assert!(guarded.hazard.is_some());
        assert!(guarded.hazard.unwrap().protected(Ordering::Relaxed).is_none());
        assert!(guarded.shared().is_none());
    }
}
