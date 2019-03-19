use std::mem;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::local;

mod list;

pub use self::list::{HazardList, Iter};

const FREE: *mut () = 0 as *mut ();
const RESERVED: *mut () = 1 as *mut ();

////////////////////////////////////////////////////////////////////////////////////////////////////
/// Hazard
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A pointer visible to all threads that is protected from reclamation.
pub struct Hazard {
    protected: AtomicPtr<()>,
}

impl Hazard {
    /// Marks the hazard as unused (available for acquisition by any thread).
    #[inline]
    pub fn set_free(&self, order: Ordering) {
        self.protected.store(FREE, order);
    }

    /// Marks the hazard as unused but reserved by some specific thread for quick acquisition.
    #[inline]
    pub fn set_reserved(&self, order: Ordering) {
        self.protected.store(RESERVED, order);
    }

    /// Marks the hazard as actively protecting the given pointer (`protect`).
    #[inline]
    pub fn set_protected(&self, protect: NonNull<()>, order: Ordering) {
        self.protected.store(protect.as_ptr(), order);
    }

    /// Gets the protected pointer if there is one.
    #[inline]
    pub fn protected(&self, order: Ordering) -> Option<Protected> {
        match self.protected.load(order) {
            FREE | RESERVED => None,
            ptr => Some(Protected(unsafe { NonNull::new_unchecked(ptr) })),
        }
    }

    /// Creates new hazard for insertion in the global hazards list protecting the given pointer.
    #[inline]
    fn new(protect: NonNull<()>) -> Self {
        Self {
            protected: AtomicPtr::new(protect.as_ptr()),
        }
    }
}

/// An RAII wrapper for a global reference to a hazard pair.
pub struct HazardPtr(&'static Hazard);

impl HazardPtr {
    /// Gets a (lifetime restricted) reference to the hazard.
    #[inline]
    pub fn hazard(&self) -> &Hazard {
        &self.0
    }

    /// Consumes self and returns the raw static `Hazard` reference.
    #[inline]
    pub fn into_inner(self) -> &'static Hazard {
        let hazard = self.0;
        mem::forget(self);
        hazard
    }
}

impl From<&'static Hazard> for HazardPtr {
    #[inline]
    fn from(pair: &'static Hazard) -> Self {
        Self(pair)
    }
}

impl Drop for HazardPtr {
    #[inline]
    fn drop(&mut self) {
        // try returning the hazard pair to the thread local cache or mark it (globally) as
        // available for all threads if the cache is at maximum capacity
        if let Err(hazard) = local::try_recycle_hazard(self.0) {
            hazard.set_free(Ordering::Release);
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
/// Protected
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An untyped pointer protected from reclamation, because it is stored within a hazard pair.
///
/// The type information is deliberately stripped as it is not needed in order to determine whether
/// a pointer is protected or not.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Protected(NonNull<()>);

impl Protected {
    /// Gets the memory address of the protected pointer.
    #[inline]
    pub fn address(&self) -> usize {
        self.0.as_ptr() as usize
    }
}
