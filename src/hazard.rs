//! Data structures and functionality for temporarily protecting specific pointers (i.e. hazard
//! pointers) acquired by specific threads from concurrent reclamation.
//!
//! # Global List
//!
//! All hazard pointers are stored in a global linked list. This list can never remove and
//! deallocate any of its entries, since this would require some scheme for concurrent memory
//! reclamation on its own.
//! Consequently, this linked list can only grow during the entire program runtime and is never
//! actually dropped. However, its individual entries can be reused arbitrarily often.
//!
//! # Hazard Pointers
//!
//! Whenever a thread reads a pointer to a data structure from shared memory it has to acquire a
//! hazard pointer for it before this pointer can be safely dereferenced. These pointers are a

use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::local;

mod list;

pub use self::list::{HazardList, Iter};

const FREE: *mut () = 0 as *mut ();
const RESERVED: *mut () = 1 as *mut ();

////////////////////////////////////////////////////////////////////////////////////////////////////
// HazardPtr
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An RAII wrapper for a global reference to a hazard pair.
pub struct HazardPtr(&'static Hazard);

impl Deref for HazardPtr {
    type Target = Hazard;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
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
        // try to store the hazard in the thread local cache or mark is as globally available
        if local::try_recycle_hazard(self.0).is_err() {
            // (HAZ:1) this `Release` store synchronizes-with ...
            self.0.set_free(Ordering::Release);
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hazard
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

    /// Gets the protected pointer if there is one.
    #[inline]
    pub fn protected(&self, order: Ordering) -> Option<Protected> {
        match self.protected.load(order) {
            FREE | RESERVED => None,
            ptr => Some(Protected(unsafe { NonNull::new_unchecked(ptr) })),
        }
    }

    /// Marks the hazard as actively protecting the given pointer.
    #[inline]
    pub fn set_protected(&self, protect: NonNull<()>) {
        // (HAZ:2) this `SeqCst` store synchronizes-with the `SeqCst` fence (GLO:1) and establishes
        // a total order of all stores
        self.protected.store(protect.as_ptr(), Ordering::SeqCst);
    }

    /// Creates new hazard for insertion in the global hazards list protecting the given pointer.
    #[inline]
    fn new(protect: NonNull<()>) -> Self {
        Self {
            protected: AtomicPtr::new(protect.as_ptr()),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Protected
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
    pub fn address(self) -> usize {
        self.0.as_ptr() as usize
    }
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;
    use std::sync::atomic::Ordering;

    use super::*;

    #[test]
    fn protect_hazard() {
        let ptr = NonNull::from(&1);

        let hazard = Hazard::new(ptr.cast());
        assert_eq!(
            ptr.as_ptr() as usize,
            hazard.protected(Ordering::Relaxed).unwrap().address()
        );

        hazard.set_free(Ordering::Relaxed);
        assert_eq!(None, hazard.protected(Ordering::Relaxed));
        assert_eq!(FREE, hazard.protected.load(Ordering::Relaxed));

        hazard.set_reserved(Ordering::Relaxed);
        assert_eq!(None, hazard.protected(Ordering::Relaxed));
        assert_eq!(RESERVED, hazard.protected.load(Ordering::Relaxed));

        hazard.set_protected(ptr.cast());
        assert_eq!(
            ptr.as_ptr() as usize,
            hazard.protected(Ordering::Relaxed).unwrap().address()
        );
    }
}
