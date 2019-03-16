#![feature(crate_visibility_modifier)]
#![feature(manually_drop_take)]

use std::mem;
use std::sync::atomic::Ordering;

use reclaim::{MarkedNonNull, MarkedPtr, NotEqual, Protected, Reclaim, Unsigned};

pub type Atomic<T, N> = reclaim::Atomic<T, N, HP>;
pub type Shared<'g, T, N> = reclaim::Shared<'g, T, N, HP>;
pub type Unlinked<T, N> = reclaim::Unlinked<T, N, HP>;

mod global;
mod hazard;
mod local;
mod retired;

use crate::hazard::{Hazard, HazardPair};
use crate::retired::Retired;
use std::ptr::NonNull;

////////////////////////////////////////////////////////////////////////////////////////////////////
/// HP
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct HP;

unsafe impl Reclaim for HP {
    type RecordHeader = ();

    #[inline]
    unsafe fn reclaim<T, N: Unsigned>(unlinked: Unlinked<T, N>)
    where
        T: 'static,
    {
        Self::reclaim_unchecked(unlinked)
    }

    #[inline]
    unsafe fn reclaim_unchecked<T, N: Unsigned>(unlinked: Unlinked<T, N>) {
        let unmarked = Unlinked::into_marked_non_null(unlinked).decompose_non_null();
        local::retire_record(Retired::new_unchecked(unmarked));
    }
}

pub fn guarded<T, N: Unsigned>() -> impl Protected<Item = T, MarkBits = N, Reclaimer = HP> {
    Guarded::new()
}

////////////////////////////////////////////////////////////////////////////////////////////////////
/// Guarded
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct Guarded<T, N: Unsigned> {
    hazard: Option<(&'static HazardPair, MarkedNonNull<T, N>)>,
}

impl<T, N: Unsigned> Guarded<T, N> {
    #[inline]
    fn take_hazard_and_protect(&mut self, protect: NonNull<()>) -> Option<Hazard> {
        self.hazard.take().map(|(hazard, _)| {
            // (1) this `Release` store synchronizes with any load on the same hazard pointer
            hazard.set_protected(protect, Ordering::Release);
            Hazard::from(hazard)
        })
    }
}

impl<T, N: Unsigned> Protected for Guarded<T, N> {
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
            .map(|(_, ptr)| unsafe { Shared::from_marked_non_null(ptr) })
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
                let handle = self
                    .take_hazard_and_protect(protect.cast())
                    .unwrap_or(acquire_hazard_for(protect.cast()));

                // the initially taken snapshot is now stored in the hazard pointer, but the value
                // stored in `atomic` may have changed
                // (2) this load synchronizes with any potential store to `atomic` and needs to have
                // at least `Acquire` ordering to ensure that none of the following reads or writes
                // are reordered before it
                while let Some(ptr) = MarkedNonNull::new(atomic.load_raw(order)) {
                    let unmarked = ptr.decompose_non_null();
                    if protect == unmarked {
                        self.hazard = Some((handle.into_inner(), ptr));

                        // this is safe because `ptr` is now stored in a hazard pointer and matches
                        // the current value of `atomic`
                        return Some(unsafe { Shared::from_marked_non_null(ptr) });
                    }

                    handle
                        .hazard_pair()
                        .set_protected(unmarked.cast(), Ordering::Release);
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
        compare: MarkedPtr<T, N>,
        order: Ordering,
    ) -> Result<Option<Shared<T, N>>, NotEqual> {
        match MarkedNonNull::new(atomic.load_raw(Ordering::Relaxed)) {
            // values of `atomic` and `compare` are non-null and equal
            Some(ptr) if ptr == compare => {
                let unmarked = ptr.decompose_non_null();
                let handle = self
                    .take_hazard_and_protect(unmarked.cast())
                    .unwrap_or(acquire_hazard_for(unmarked.cast()));

                // (3) this load operation ...
                if atomic.load_raw(order) != ptr {
                    return Err(NotEqual);
                }

                self.hazard = Some((handle.into_inner(), ptr));

                // this is safe because `ptr` is now stored in a hazard pointer and matches
                // the current value of `atomic`
                return Ok(Some(unsafe { Shared::from_marked_non_null(ptr) }));
            }
            // value `atomic` and `compare` are both null
            None if compare.is_null() => {
                self.release();
                return Ok(None);
            }
            _ => return Err(NotEqual),
        }
    }

    #[inline]
    fn release(&mut self) {
        if let Some((hazard, _)) = self.hazard {
            self.hazard = None;
            mem::drop(Hazard::from(hazard))
        }
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

#[inline]
fn acquire_hazard_for(ptr: NonNull<()>) -> Hazard {
    if let Some(handle) = local::acquire_hazard() {
        // (4) this `Release` store synchronizes with any load on the same hazard pointer
        handle.hazard_pair().set_protected(ptr, Ordering::Release);

        return handle;
    }

    global::acquire_hazard_for(ptr)
}
