mod list;

use core::ptr::NonNull;
use core::sync::atomic::AtomicPtr;

// use conquer_util::align::CacheAligned

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

////////////////////////////////////////////////////////////////////////////////////////////////////
// Protected
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An untyped pointer protected from reclamation, because it is stored within a hazard pair.
///
/// The type information is deliberately stripped as it is not needed in order to determine whether
/// a pointer is protected or not.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Protected(NonNull<()>);
