mod list;

use core::ptr::NonNull;
use core::sync::atomic::AtomicPtr;

pub(crate) use self::list::{HazardList, Iter};

const FREE: *mut () = 0 as *mut ();
const THREAD_RESERVED: *mut () = 1 as *mut ();

////////////////////////////////////////////////////////////////////////////////////////////////////
// Hazard
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A pointer visible to all threads that is protected from reclamation.
#[derive(Debug)]
pub struct Hazard {
    protected: AtomicPtr<()>,
}

/********** impl Hazard ***************************************************************************/

impl Hazard {
    #[inline]
    pub const fn new() -> Self {
        Self { protected: AtomicPtr::new(FREE) }
    }

    #[inline]
    pub const fn with_protected(protected: *const ()) -> Self {
        Self { protected: AtomicPtr::new(protected as *mut _) }
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

/********** impl inherent *************************************************************************/

impl Protected {
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
    Protect(Protected),
}
