use core::ptr::NonNull;
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
pub struct Guarded<T, L: LocalAccess, N: Unsigned> {
    hazard: Option<&'static Hazard>,
    marked: Marked<MarkedNonNull<T, N>>,
    local_access: L,
}

unsafe impl<T, L: LocalAccess + Send, N: Unsigned> Send for Guarded<T, L, N> {}

impl<T, L: LocalAccess, N: Unsigned> Clone for Guarded<T, L, N> {
    #[inline]
    fn clone(&self) -> Self {
        if let Value(ptr) = self.marked {
            let protect = ptr.decompose_non_null();
            let hazard = Some(self.local_access.get_hazard(protect.cast()));

            return Self { hazard, marked: Value(ptr), local_access: self.local_access };
        }

        Self { hazard: None, marked: self.marked, local_access: self.local_access }
    }
}

unsafe impl<T, L: LocalAccess, N: Unsigned> Protect for Guarded<T, L, N> {
    type Item = T;
    type Reclaimer = HP;
    type MarkBits = N;

    #[inline]
    fn marked(&self) -> Marked<Shared<T, N>> {
        self.marked.map(|ptr| unsafe { Shared::from_marked_non_null(ptr) })
    }

    #[inline]
    fn acquire(&mut self, atomic: &Atomic<T, N>, order: Ordering) -> Marked<Shared<T, N>> {
        match MarkedNonNull::new(atomic.load_raw(Relaxed)) {
            Null(tag) => self.release_with_tag(tag),
            Value(ptr) => {
                let mut protect = ptr.decompose_non_null();
                let hazard = self.unwrap_hazard_and_protect(protect.cast());

                // the initially taken snapshot is now stored in the hazard pointer, but the value
                // stored in `atomic` may have changed already
                loop {
                    match MarkedNonNull::new(atomic.load_raw(order)) {
                        Null(tag) => return self.release_with_tag(tag),
                        Value(ptr) => {
                            let unmarked = ptr.decompose_non_null();
                            if protect == unmarked {
                                self.marked = Value(ptr);
                                // this is safe because `ptr` is now stored in a hazard pointer and
                                // matches the current value of `atomic`
                                return Value(unsafe { Shared::from_marked_non_null(ptr) });
                            }

                            // (GUA:2) this `SeqCst` store synchronizes-with the
                            // `SeqCst` fence (GLO:1)
                            hazard.set_protected(unmarked.cast(), SeqCst);
                            protect = unmarked;
                        }
                    }
                }
            }
        }
    }

    #[inline]
    fn acquire_if_equal(
        &mut self,
        atomic: &Atomic<T, N>,
        expected: MarkedPtr<T, N>,
        order: Ordering,
    ) -> AcquireResult<T, N> {
        let raw = atomic.load_raw(Relaxed);
        if raw != expected {
            return Err(NotEqualError);
        }

        match MarkedNonNull::new(raw) {
            Null(tag) => Ok(self.release_with_tag(tag)),
            Value(ptr) => {
                let unmarked = ptr.decompose_non_null();
                let hazard = self.unwrap_hazard_and_protect(unmarked.cast());

                if atomic.load_raw(order) != ptr {
                    hazard.set_scoped(Release);
                    return Err(NotEqualError);
                }

                self.marked = Value(ptr);
                Ok(Value(unsafe { Shared::from_marked_non_null(ptr) }))
            }
        }
    }

    #[inline]
    fn release(&mut self) {
        let _ = self.release_with_tag(0);
    }
}

impl<T, L: LocalAccess, N: Unsigned> Guarded<T, L, N> {
    /// Creates a new guarded
    #[inline]
    pub fn with_access(local_access: L) -> Self {
        Self { hazard: None, marked: Marked::default(), local_access }
    }

    #[inline]
    fn release_with_tag(&mut self, tag: usize) -> Marked<Shared<T, N>> {
        if cfg!(feature = "count-release") {
            LocalAccess::increase_ops_count(self.local_access);
        }

        if let Some(hazard) = self.hazard {
            // (GUA:3) this `Release` store synchronizes-with ...
            hazard.set_scoped(Release);
        }

        self.marked = Null(tag);
        Null(tag)
    }

    #[inline]
    fn unwrap_hazard_and_protect(&mut self, protect: NonNull<()>) -> &'static Hazard {
        match self.hazard.take() {
            Some(hazard) => {
                hazard.set_protected(protect.cast(), SeqCst);
                self.hazard = Some(hazard);
                hazard
            }
            None => {
                let hazard = self.local_access.get_hazard(protect.cast());
                self.hazard = Some(hazard);
                hazard
            }
        }
    }
}

impl<T, L: LocalAccess, N: Unsigned> Drop for Guarded<T, L, N> {
    #[inline]
    fn drop(&mut self) {
        if let Some(hazard) = self.hazard {
            if cfg!(feature = "count-release") {
                LocalAccess::increase_ops_count(self.local_access);
            }

            if self.local_access.try_recycle_hazard(hazard).is_err() {
                hazard.set_free(Release);
            }
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

    type Guarded<'a> = super::Guarded<i32, &'a Local, U0>;

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
