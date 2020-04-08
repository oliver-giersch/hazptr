mod list;

use core::cmp;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicPtr, Ordering};

pub(crate) use self::list::HazardList;

/// State of a hazard pointer that is free and has not previously been acquired.
const NOT_YET_USED: *mut () = 0 as _;
/// State of a hazard pointer that is free and has previously been acquired.
const FREE: *mut () = 1 as _;
/// State of a hazard pointer that is reserved by a specific thread.
const THREAD_RESERVED: *mut () = 2 as _;

////////////////////////////////////////////////////////////////////////////////////////////////////
// HazardPtr
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A pointer that must visible to all threads that indicates that the currently
/// pointed-to value is in use by some thread and therefore protected from
/// reclamation, i.e. it must not be de-allocated.
pub(crate) struct HazardPtr {
    protected: AtomicPtr<()>,
}

/********** impl Hazard ***************************************************************************/

impl HazardPtr {
    /// Sets the [`HazardPtr`] free meaning it can be acquired by other threads
    /// and the previous value is no longer protected.
    #[inline]
    pub fn set_free(&self, order: Ordering) {
        self.protected.store(FREE, order);
    }

    /// Sets the [`HazardPtr`] as thread-reserved meaning  the previous value is
    /// no longer protected but the pointer is still logically owned by the
    /// calling thread.
    #[inline]
    pub fn set_thread_reserved(&self, order: Ordering) {
        self.protected.store(THREAD_RESERVED, order);
    }

    #[inline]
    pub fn protected(&self, order: Ordering) -> ProtectedResult {
        match self.protected.load(order) {
            NOT_YET_USED => ProtectedResult::AbortIteration,
            FREE | THREAD_RESERVED => ProtectedResult::Unprotected,
            ptr => unsafe {
                // safety: null is covered by `NOT_YET_USED`
                ProtectedResult::Protected(ProtectedPtr(NonNull::new_unchecked(ptr)))
            },
        }
    }

    #[inline]
    pub fn set_protected(&self, protected: NonNull<()>, order: Ordering) {
        debug_assert_eq!(order, Ordering::SeqCst, "this method requires sequential consistency");
        self.protected.store(protected.as_ptr(), Ordering::SeqCst);
    }

    /// Creates a new [`HazardPointer`].
    #[inline]
    const fn new() -> Self {
        Self { protected: AtomicPtr::new(NOT_YET_USED) }
    }

    /// Creates a new [`HazardPointer`] set to initially set to `protected`.
    #[inline]
    const fn with_protected(protected: *const ()) -> Self {
        Self { protected: AtomicPtr::new(protected as *mut _) }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// ProtectedResult
////////////////////////////////////////////////////////////////////////////////////////////////////

/// The result of a call to [`protected`][HazardPtr::protected].
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ProtectedResult {
    /// Indicates that the hazard pointer currently protects some value.
    Protected(ProtectedPtr),
    /// Indicates that the hazard pointer currently does not protect any value.
    Unprotected,
    /// Indicates that hazard pointer has never been used before.
    ///
    /// Since hazard pointers are acquired in order this means that any
    /// iteration of all hazard pointers can abort early, since no subsequent
    /// hazards pointers could be in use either.
    AbortIteration,
}

/********** impl inherent *************************************************************************/

impl ProtectedResult {
    #[inline]
    pub fn protected(self) -> Option<ProtectedPtr> {
        match self {
            ProtectedResult::Protected(protected) => Some(protected),
            _ => None,
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// ProtectedPtr
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An untyped pointer protected from reclamation, because it is stored within a hazard pair.
///
/// The type information is deliberately stripped as it is not needed in order to determine whether
/// a pointer is protected or not.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProtectedPtr(NonNull<()>);

/********** impl inherent *************************************************************************/

impl ProtectedPtr {
    /// Gets the internal non-nullable pointer.
    #[inline]
    pub fn into_inner(self) -> NonNull<()> {
        self.0
    }

    #[inline]
    pub fn compare_with(self, ptr: *const ()) -> cmp::Ordering {
        self.as_ptr().cmp(&ptr)
    }

    #[inline]
    fn as_ptr(self) -> *const () {
        self.0.as_ptr() as _
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// ProtectStrategy
////////////////////////////////////////////////////////////////////////////////////////////////////

pub(crate) enum ProtectStrategy {
    ReserveOnly,
    Protect(ProtectedPtr),
}

#[cfg(test)]
mod tests {
    use core::ptr::NonNull;
    use core::sync::atomic::Ordering;

    use super::{HazardPtr, ProtectedResult};

    #[test]
    fn hazard_ptr() {
        let hazard = HazardPtr::new();
        assert_eq!(hazard.protected(Ordering::Relaxed), ProtectedResult::AbortIteration);
        hazard.set_protected(NonNull::from(&mut 1).cast(), Ordering::Relaxed);
        assert!(hazard.protected(Ordering::Relaxed).protected().is_some());
        hazard.set_thread_reserved(Ordering::Relaxed);
        assert_eq!(hazard.protected(Ordering::Relaxed), ProtectedResult::Unprotected);
        hazard.set_free(Ordering::Relaxed);
        assert_eq!(hazard.protected(Ordering::Relaxed), ProtectedResult::Unprotected);
    }
}
