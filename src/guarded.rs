use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use reclaim::prelude::*;
use reclaim::typenum::Unsigned;
use reclaim::{MarkedNonNull, MarkedPtr, NotEqual};

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
        let state = self.hazard.take();
        match MarkedNonNull::new(atomic.load_raw(Ordering::Relaxed)) {
            Value(ptr) => {
                let mut protect = ptr.decompose_non_null();
                let hazard = self.unwrap_and_protect(state, protect.cast());

                // the initially taken snapshot is now stored in the hazard pointer, but the value
                // stored in `atomic` may have changed already
                loop {
                    match MarkedNonNull::new(atomic.load_raw(order)) {
                        Value(ptr) => {
                            let unmarked = ptr.decompose_non_null();
                            if protect == unmarked {
                                self.hazard = Some(hazard);
                                // this is safe because `ptr` is now stored in a hazard pointer and
                                // matches the current value of `atomic`
                                return Value(unsafe { Shared::from_marked_non_null(ptr) });
                            }

                            // (GUA:2) this `SeqCst` store synchronizes-with the
                            // `SeqCst` fence (GLO:1)
                            hazard.set_protected(unmarked.cast(), Ordering::SeqCst);
                            protect = unmarked;
                        }
                        any => return self.handle_null(any, state),
                    }
                }
            }
            any => self.handle_null(any, state),
        }
    }

    #[inline]
    fn acquire_if_equal(
        &mut self,
        atomic: &Atomic<T, N>,
        expected: MarkedPtr<T, N>,
        order: Ordering,
    ) -> AcquireResult<T, N> {
        let raw = atomic.load_raw(Ordering::Relaxed);
        if raw != expected {
            return Err(NotEqual);
        }

        let state = self.hazard.take();
        match MarkedNonNull::new(raw) {
            Value(ptr) => {
                let unmarked = ptr.decompose_non_null();
                let hazard = self.unwrap_and_protect(state, unmarked.cast());

                if atomic.load_raw(order) != ptr {
                    hazard.set_scoped(Ordering::Release);
                    self.hazard = Some(hazard);
                    return Err(NotEqual);
                }

                self.hazard = Some(hazard);
                Ok(Value(unsafe { Shared::from_marked_non_null(ptr) }))
            }
            any => return Ok(self.handle_null(any, state)),
        }
    }

    #[inline]
    fn release(&mut self) {
        if let Some(hazard) = self.hazard {
            if cfg!(feature = "count-release") {
                LocalAccess::increase_ops_count(self.local_access);
            }

            // (GUA:y) this `Release` store synchronizes-with ...
            hazard.set_scoped(Ordering::Release);
        }

        self.marked = Null;
    }
}

impl<T, L: LocalAccess, N: Unsigned> Guarded<T, L, N> {
    /// Creates a new guarded
    #[inline]
    pub fn with_access(local_access: L) -> Self {
        Self { hazard: None, marked: Null, local_access }
    }

    #[inline]
    fn handle_null(
        &mut self,
        null: Marked<MarkedNonNull<T, N>>,
        hazard: Option<&'static Hazard>,
    ) -> Marked<Shared<T, N>> {
        self.marked = null;
        self.hazard = hazard;

        match null {
            OnlyTag(tag) => OnlyTag(tag),
            Null => Null,
            _ => unreachable!(),
        }
    }

    #[inline]
    fn unwrap_and_protect(
        &self,
        hazard: Option<&'static Hazard>,
        protect: NonNull<()>,
    ) -> &'static Hazard {
        hazard
            .map(|hazard| {
                // (GUA:4) this `SeqCst` store synchronizes-with the `SeqCst` fence (GLO:1)
                hazard.set_protected(protect, Ordering::SeqCst);
                hazard
            })
            .unwrap_or_else(|| self.local_access.get_hazard(protect))
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
                hazard.set_free(Ordering::Release);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;
    use std::sync::atomic::Ordering;

    use matches::assert_matches;

    use reclaim::typenum::U0;
    use reclaim::{MarkedPointer, Protect};

    use crate::global::Global;
    use crate::local::Local;

    use super::State;

    type Atomic = crate::Atomic<i32, U0>;
    type Owned = crate::Owned<i32, U0>;

    type MarkedPtr = reclaim::MarkedPtr<i32, U0>;

    type Guarded<'a> = super::Guarded<i32, &'a Local, U0>;

    static GLOBAL: Global = Global::new();

    #[test]
    fn empty() {
        let local = Local::new(&GLOBAL);
        let mut guarded = Guarded::with_access(&local);
        assert_matches!(guarded.state, State::None);
        assert!(guarded.shared().is_none());
        assert!(guarded.take_hazard_and_protect(NonNull::from(&())).is_none());
    }

    #[test]
    fn acquire() {
        let local = Local::new(&GLOBAL);
        let mut guarded = Guarded::with_access(&local);

        let null = Atomic::null();
        let _ = guarded.acquire(&null, Ordering::Relaxed);
        assert_matches!(guarded.state, State::None);
        assert!(guarded.shared().is_none());

        let atomic = Atomic::new(1);
        let _ = guarded.acquire(&atomic, Ordering::Relaxed);
        assert_matches!(guarded.state, State::Protected(..));
        assert_eq!(unsafe { guarded.shared().unwrap().deref() }, &1);

        let _ = guarded.acquire(&null, Ordering::Relaxed);
        assert_matches!(guarded.state, State::Scoped(_));
        assert!(guarded.shared().is_none());
    }

    #[test]
    fn acquire_if_equal() {
        let local = Local::new(&GLOBAL);
        let mut guarded = Guarded::with_access(&local);

        let empty = Atomic::null();
        let null = MarkedPtr::null();

        let res = guarded.acquire_if_equal(&empty, null, Ordering::Relaxed);
        assert_matches!(res, Ok(None));
        assert_matches!(guarded.state, State::None);
        assert!(guarded.shared().is_none());

        let owned = Owned::new(1);
        let marked = owned.as_marked();
        let atomic = Atomic::from(owned);

        let res = guarded.acquire_if_equal(&atomic, null, Ordering::Relaxed);
        assert_matches!(res, Err(_));
        assert_matches!(guarded.state, State::None);
        assert!(guarded.shared().is_none());

        let res = guarded.acquire_if_equal(&atomic, marked, Ordering::Relaxed);
        assert_matches!(res, Ok(Some(_)));
        assert_matches!(guarded.state, State::Protected(..));
        assert_eq!(unsafe { guarded.shared().unwrap().deref() }, &1);

        // a failed acquire must not alter the previous state
        let res = guarded.acquire_if_equal(&atomic, null, Ordering::Relaxed);
        assert_matches!(res, Err(_));
        assert_matches!(guarded.state, State::Protected(..));
        assert_eq!(unsafe { guarded.shared().unwrap().deref() }, &1);

        let res = guarded.acquire_if_equal(&empty, null, Ordering::Relaxed);
        assert_matches!(res, Ok(None));
        assert_matches!(guarded.state, State::Scoped(_));
        assert!(guarded.shared().is_none());
    }
}
