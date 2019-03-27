//! Hazard Pointer based concurrent memory reclamation adhering to the reclamation interface defined
//! by the `reclaim` crate.

use std::ptr::NonNull;
use std::sync::atomic::Ordering;

use reclaim::{MarkedNonNull, MarkedPtr, NotEqual, Protect, Reclaim, Unsigned};

pub type Atomic<T, N> = reclaim::Atomic<T, N, HP>;
pub type Shared<'g, T, N> = reclaim::Shared<'g, T, N, HP>;
pub type Owned<T, N> = reclaim::Owned<T, N, HP>;
pub type Unlinked<T, N> = reclaim::Unlinked<T, N, HP>;

mod global;
mod hazard;
mod local;
mod retired;
#[cfg(test)]
mod tests;

use crate::hazard::HazardPtr;
use crate::retired::Retired;

////////////////////////////////////////////////////////////////////////////////////////////////////
// HP
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Hazard Pointer based reclamation scheme.
#[derive(Debug, Default, Copy, Clone)]
pub struct HP;

unsafe impl Reclaim for HP {
    // hazard pointers do not require any extra information per allocated record
    type RecordHeader = ();

    #[inline]
    unsafe fn retire<T, N: Unsigned>(unlinked: Unlinked<T, N>)
    where
        T: 'static,
    {
        Self::retire_unchecked(unlinked)
    }

    #[inline]
    unsafe fn retire_unchecked<T, N: Unsigned>(unlinked: Unlinked<T, N>) {
        let unmarked = Unlinked::into_marked_non_null(unlinked).decompose_non_null();
        local::retire_record(Retired::new_unchecked(unmarked));
    }
}

/// Creates a new (empty) guarded pointer that can be used to acquire hazard pointers.
#[inline]
pub fn guarded<T, N: Unsigned>() -> impl Protect<Item = T, MarkBits = N, Reclaimer = HP> {
    Guarded::new()
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Guarded
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A guarded pointer that can be used to acquire hazard pointers.
pub struct Guarded<T, N: Unsigned> {
    hazard: Option<(HazardPtr, MarkedNonNull<T, N>)>,
}

impl<T, N: Unsigned> Guarded<T, N> {
    /// Takes the internally stored hazard pointer, sets it to protect the given pointer (`protect`)
    /// and wraps it in a `HazardPtr`.
    #[inline]
    fn take_hazard_and_protect(&mut self, protect: NonNull<()>) -> Option<HazardPtr> {
        self.hazard.take().map(|(handle, _)| {
            handle.set_protected(protect);
            handle
        })
    }
}

impl<T, N: Unsigned> Protect for Guarded<T, N> {
    type Item = T;
    type MarkBits = N;
    type Reclaimer = HP;

    #[inline]
    fn new() -> Self {
        Self::default()
    }

    #[inline]
    fn shared(&self) -> Option<Shared<T, N>> {
        self.hazard
            .as_ref()
            .map(|(_, ptr)| unsafe { Shared::from_marked_non_null(*ptr) })
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
                    .unwrap_or_else(|| acquire_hazard_for(protect.cast()));

                // the initially taken snapshot is now stored in the hazard pointer, but the value
                // stored in `atomic` may have changed already
                // (LIB:2) this load has to synchronize with any potential store to `atomic`
                while let Some(ptr) = MarkedNonNull::new(atomic.load_raw(order)) {
                    let unmarked = ptr.decompose_non_null();
                    if protect == unmarked {
                        self.hazard = Some((hazard, ptr));

                        // this is safe because `ptr` is now stored in a hazard pointer and matches
                        // the current value of `atomic`
                        return Some(unsafe { Shared::from_marked_non_null(ptr) });
                    }

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
    ) -> Result<Option<Shared<T, N>>, NotEqual> {
        match MarkedNonNull::new(atomic.load_raw(Ordering::Relaxed)) {
            // values of `atomic` and `compare` are non-null and equal
            Some(ptr) if ptr == expected => {
                let unmarked = ptr.decompose_non_null();
                let hazard = self
                    .take_hazard_and_protect(unmarked.cast())
                    .unwrap_or_else(|| acquire_hazard_for(unmarked.cast()));

                // (LIB:2) this load operation should synchronize-with any store operation to the
                // same `atomic`
                if atomic.load_raw(order) != ptr {
                    return Err(NotEqual);
                }

                self.hazard = Some((hazard, ptr));

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
        if cfg!(feature = "count-release") && self.hazard.is_some() {
            local::increase_ops_count();
        }

        // if `hazard` is Some(_) the contained `HazardPtr` is dropped
        self.hazard = None;
    }
}

impl<T, N: Unsigned> Default for Guarded<T, N> {
    #[inline]
    fn default() -> Self {
        Self { hazard: None }
    }
}

impl<T, N: Unsigned> Drop for Guarded<T, N> {
    #[inline]
    fn drop(&mut self) {
        self.release();
    }
}

/// Attempts to take a reserved hazard from the thread-local cache or infallibly acquires one from
/// the global list.
#[inline]
fn acquire_hazard_for(protect: NonNull<()>) -> HazardPtr {
    if let Some(handle) = local::acquire_hazard() {
        handle.set_protected(protect);

        return handle;
    }

    global::acquire_hazard_for(protect)
}
