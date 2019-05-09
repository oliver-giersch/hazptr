//! Data structures and functionality for temporarily protecting specific pointers acquired by
//! specific threads from concurrent reclamation.
//!
//! # Global List
//!
//! All hazard pointers are stored in a global linked list. This list can never remove and
//! deallocate any of its entries, since this would require some scheme for concurrent memory
//! reclamation on its own. Consequently, this linked list can only grow during the entire program
//! runtime and is never actually dropped. However, its individual entries can be reused arbitrarily
//! often.
//!
//! # Hazard Pointers
//!
//! Whenever a thread reads a value in a data structure from shared memory it has to acquire a
//! hazard pointer for it before the loaded reference to the value can be safely dereferenced. These
//! pointers are stored in the global list of hazard pointers. Any time a thread wants to reclaim a
//! retired record, it has to ensure that no hazard pointer in this list still protects the retired
//! value.

use core::ptr::NonNull;
use core::sync::atomic::{AtomicPtr, Ordering};

mod list;

pub use self::list::{HazardList, Iter};

const FREE: *mut () = 0 as *mut ();
const SCOPED: *mut () = 1 as *mut ();
const RESERVED: *mut () = 2 as *mut ();

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hazard
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A pointer visible to all threads that is protected from reclamation.
#[derive(Debug)]
pub struct Hazard {
    protected: AtomicPtr<()>,
}

impl Hazard {
    /// Marks the hazard as unused (available for acquisition by any thread).
    #[inline]
    pub fn set_free(&self, order: Ordering) {
        self.protected.store(FREE, order);
    }

    /// Marks the hazard as unused but scoped by a specific `Guarded` for fastest acquisition.
    #[inline]
    pub fn set_scoped(&self, order: Ordering) {
        self.protected.store(SCOPED, order);
    }

    /// Marks the hazard as unused but reserved by a specific thread for quick acquisition.
    #[inline]
    pub fn set_reserved(&self, order: Ordering) {
        self.protected.store(RESERVED, order);
    }

    /// Gets the protected pointer, if there is one.
    #[inline]
    pub fn protected(&self, order: Ordering) -> Option<Protected> {
        match self.protected.load(order) {
            FREE | SCOPED | RESERVED => None,
            ptr => Some(Protected(unsafe { NonNull::new_unchecked(ptr) })),
        }
    }

    /// Marks the hazard as actively protecting the given pointer `protect`.
    ///
    /// # Panics
    ///
    /// In a `debug` build, this operation panics if `ordering` is not `SeqCst`.
    #[inline]
    pub fn set_protected(&self, protect: NonNull<()>, order: Ordering) {
        assert_eq!(order, Ordering::SeqCst, "must only be called with `SeqCst`");
        self.protected.store(protect.as_ptr(), order);
    }

    /// Creates new hazard for insertion in the global hazards list protecting the given pointer.
    #[inline]
    fn new(protect: NonNull<()>) -> Self {
        Self { protected: AtomicPtr::new(protect.as_ptr()) }
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

    /// Gets the internal non-nullable pointer.
    #[inline]
    pub fn into_inner(self) -> NonNull<()> {
        self.0
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
        assert_eq!(ptr.as_ptr() as usize, hazard.protected(Ordering::Relaxed).unwrap().address());

        hazard.set_free(Ordering::Relaxed);
        assert_eq!(None, hazard.protected(Ordering::Relaxed));
        assert_eq!(FREE, hazard.protected.load(Ordering::Relaxed));

        hazard.set_reserved(Ordering::Relaxed);
        assert_eq!(None, hazard.protected(Ordering::Relaxed));
        assert_eq!(RESERVED, hazard.protected.load(Ordering::Relaxed));

        hazard.set_protected(ptr.cast(), Ordering::SeqCst);
        assert_eq!(ptr.as_ptr() as usize, hazard.protected(Ordering::Relaxed).unwrap().address());
    }
}
