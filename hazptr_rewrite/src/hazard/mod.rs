mod list;

use core::ptr::NonNull;
use core::sync::atomic::{AtomicPtr, Ordering};

pub(crate) use self::list::HazardList;

const FREE: *mut () = 0 as *mut ();
const THREAD_RESERVED: *mut () = 1 as *mut ();
const NOT_YET_USED: *mut () = 2 as *mut ();

////////////////////////////////////////////////////////////////////////////////////////////////////
// HazardPtr
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A pointer visible to all threads that is protected from reclamation.
#[derive(Debug)]
pub(crate) struct HazardPtr {
    protected: AtomicPtr<()>,
}

/********** impl Hazard ***************************************************************************/

impl HazardPtr {
    #[inline]
    pub const fn new() -> Self {
        Self { protected: AtomicPtr::new(FREE) }
    }

    #[inline]
    pub const fn with_protected(protected: *const ()) -> Self {
        Self { protected: AtomicPtr::new(protected as *mut _) }
    }

    #[inline]
    pub fn set_free(&self, order: Ordering) {
        self.protected.store(FREE, order);
    }

    #[inline]
    pub fn set_thread_reserved(&self, order: Ordering) {
        self.protected.store(THREAD_RESERVED, order);
    }

    #[inline]
    pub fn protected(&self, order: Ordering) -> ProtectedResult {
        match self.protected.load(order) {
            FREE | THREAD_RESERVED => ProtectedResult::Unprotected,
            NOT_YET_USED => ProtectedResult::Abort,
            ptr => ProtectedResult::Protected(ProtectedPtr(NonNull::new(ptr).unwrap())),
        }
    }

    #[inline]
    pub fn set_protected(&self, protected: NonNull<()>, order: Ordering) {
        assert_eq!(order, Ordering::SeqCst, "this method must be sequentially consistent");
        self.protected.store(protected.as_ptr(), order);
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// ProtectedResult
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ProtectedResult {
    Protected(ProtectedPtr),
    Unprotected,
    Abort,
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
    #[inline]
    pub fn as_const_ptr(self) -> *const () {
        self.0.as_ptr() as *const _
    }

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

////////////////////////////////////////////////////////////////////////////////////////////////////
// ProtectStrategy
////////////////////////////////////////////////////////////////////////////////////////////////////

pub(crate) enum ProtectStrategy {
    ReserveOnly,
    Protect(ProtectedPtr),
}
