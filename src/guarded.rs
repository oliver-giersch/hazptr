use core::mem;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use reclaim::typenum::Unsigned;
use reclaim::{AcquireResult, MarkedNonNull, MarkedPointer, MarkedPtr, NotEqual, Protect};

use crate::hazard::HazardPtr;
use crate::local::LocalAccess;
use crate::{Atomic, Shared, HP};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Guarded
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A guarded pointer that can be used to acquire hazard pointers.
#[derive(Debug)]
pub struct Guarded<T, L: LocalAccess, N: Unsigned> {
    state: State<T, L, N>,
    local_access: L,
}

unsafe impl<T, L: LocalAccess, N: Unsigned> Send for Guarded<T, L, N> {}

impl<T, L: LocalAccess, N: Unsigned> Clone for Guarded<T, L, N> {
    #[inline]
    fn clone(&self) -> Self {
        if let State::Protected(hazard, ptr) = &self.state {
            Self {
                state: State::Protected(
                    L::acquire_hazard_for(
                        self.local_access,
                        hazard.protected(Ordering::Acquire).unwrap().into_inner(),
                    ),
                    *ptr,
                ),
                local_access: self.local_access,
            }
        } else {
            Self {
                state: State::None,
                local_access: self.local_access,
            }
        }
    }
}

unsafe impl<T, L: LocalAccess, N: Unsigned> Protect for Guarded<T, L, N> {
    type Item = T;
    type Reclaimer = HP;
    type MarkBits = N;

    #[inline]
    fn shared(&self) -> Option<Shared<T, N>> {
        match self.state {
            State::Protected(_, ptr) => Some(unsafe { Shared::from_marked_non_null(ptr) }),
            _ => None,
        }
    }

    #[inline]
    fn acquire(&mut self, atomic: &Atomic<T, N>, order: Ordering) -> Option<Shared<T, N>> {
        match MarkedNonNull::new(atomic.load_raw(Ordering::Relaxed)) {
            None => {
                self.release();
                None
            }
            Some(ptr) => {
                let mut protect = ptr.decompose_non_null();
                let hazard = self
                    .take_hazard_and_protect(protect.cast())
                    .unwrap_or_else(|| L::acquire_hazard_for(self.local_access, protect.cast()));

                // the initially taken snapshot is now stored in the hazard pointer, but the value
                // stored in `atomic` may have changed already
                // (LIB:2) this load has to synchronize with any potential store to `atomic`
                while let Some(ptr) = MarkedNonNull::new(atomic.load_raw(order)) {
                    let unmarked = ptr.decompose_non_null();
                    if protect == unmarked {
                        self.state = State::Protected(hazard, ptr);

                        // this is safe because `ptr` is now stored in a hazard pointer and matches
                        // the current value of `atomic`
                        return Some(unsafe { Shared::from_marked_non_null(ptr) });
                    }

                    // this operation issues a full `SeqCst` memory fence
                    hazard.set_protected(unmarked.cast());
                    protect = unmarked;
                }

                None
            }
        }
    }

    #[inline]
    fn acquire_if_equal(
        &mut self,
        atomic: &Atomic<T, N>,
        expected: MarkedPtr<T, N>,
        order: Ordering,
    ) -> AcquireResult<T, Self::Reclaimer, N> {
        match MarkedNonNull::new(atomic.load_raw(Ordering::Relaxed)) {
            // values of `atomic` and `compare` are non-null and equal
            Some(ptr) if ptr == expected => {
                let unmarked = ptr.decompose_non_null();
                let hazard = self
                    .take_hazard_and_protect(unmarked.cast())
                    .unwrap_or_else(|| L::acquire_hazard_for(self.local_access, unmarked.cast()));

                // (LIB:2) this load operation should synchronize-with any store operation to the
                // same `atomic`
                if atomic.load_raw(order) != ptr {
                    return Err(NotEqual);
                }

                self.state = State::Protected(hazard, ptr);

                // this is safe because `ptr` is now stored in a hazard pointer and matches
                // the current value of `atomic`
                Ok(Some(unsafe { Shared::from_marked_non_null(ptr) }))
            }
            // values of `atomic` and `compare` are both null
            None if expected.is_null() => {
                self.release();
                Ok(None)
            }
            _ => Err(NotEqual),
        }
    }

    #[inline]
    fn release(&mut self) {
        if let State::Protected(hazard, _) | State::Scoped(hazard) = self.state.take() {
            if cfg!(feature = "count-release") {
                LocalAccess::increase_ops_count(self.local_access);
            }

            // (LIB:3) this `Release` store synchronizes-with any `Acquire` load on the `protected`
            // field of the same hazard pointer
            hazard.set_scoped(Ordering::Release);
            self.state = State::Scoped(hazard)
        }
    }
}

impl<T, L: LocalAccess, N: Unsigned> Guarded<T, L, N> {
    /// Creates a new guarded
    #[inline]
    pub fn new(local_access: L) -> Self {
        Self {
            state: State::None,
            local_access,
        }
    }

    /// Takes the internally stored hazard pointer, sets it to protect the given pointer (`protect`)
    /// and wraps it in a [`HazardPtr`](HazardPtr).
    #[inline]
    fn take_hazard_and_protect(&mut self, protect: NonNull<()>) -> Option<HazardPtr<L>> {
        match self.state.take() {
            State::Protected(hazard, _) | State::Scoped(hazard) => {
                // this operation issues a full `SeqCst` memory fence
                hazard.set_protected(protect);
                Some(hazard)
            }
            _ => None,
        }
    }
}

impl<T, L: LocalAccess, N: Unsigned> Drop for Guarded<T, L, N> {
    #[inline]
    fn drop(&mut self) {
        if cfg!(feature = "count-release") {
            LocalAccess::increase_ops_count(self.local_access);
        }
    }
}

#[derive(Debug)]
enum State<T, L: LocalAccess, N: Unsigned> {
    Protected(HazardPtr<L>, MarkedNonNull<T, N>),
    Scoped(HazardPtr<L>),
    None,
}

impl<T, L: LocalAccess, N: Unsigned> State<T, L, N> {
    #[inline]
    fn take(&mut self) -> Self {
        mem::replace(self, State::None)
    }
}
