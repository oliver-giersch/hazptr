//! Hazard pointer based concurrent memory reclamation.
//!
//! A difficult problem that has to be considered when implementing lock-free
//! collections or data structures is deciding, when a removed entry can be
//! safely deallocated.
//! It is usually not correct to deallocate removed entries right away, because
//! different threads might still hold references to such entries and could
//! consequently access already freed memory.
//!
//! Concurrent memory reclamation schemes solve that problem by extending the
//! lifetime of removed entries for a certain *grace period*.
//! After this period it must be impossible for other threads to have any
//! references to these entries anymore and they can be finally deallocated.
//! This is similar to the concept of *Garbage Collection* in languages like Go
//! and Java, but with a much more limited scope.
//!
//! The Hazard-pointer reclamation scheme was described by Maged M. Michael in
//! 2004 [[1]].
//! It requires every *read* of an entry from shared memory to be accompanied by
//! a global announcement marking the read entry as protected.
//! Threads must store removed (retired) entries in a local cache and regularly
//! attempt to reclaim all cached records in bulk.
//! A record is safe to be reclaimed, once there is no hazard pointer protecting
//! it anymore.
//!
//! # Reclamation Interface and Pointer Types
//!
//! The API of this library follows the abstract interface defined by the
//! [`reclaim`][reclaim] crate.
//! Hence, it uses the following types for atomically reading and writing from
//! and to shared memory:
//!
//! - [`Atomic`]
//! - [`Owned`]
//! - [`Shared`]
//! - [`Unlinked`]
//! - [`Unprotected`]
//!
//! The primary type exposed by this API is [`Atomic`], which is a
//! shared atomic pointer with similar semantics to `Option<Box<T>>`.
//! It provides all operations that are also supported by `AtomicPtr`, such as
//! `store`, `load` or `compare_exchange`.
//! All *load* operations on an [`Atomic`] return (optional) [`Shared`]
//! references.
//! [`Shared`] is a non-nullable pointer type that is protected by a hazard
//! pointer and has similar semantics to `&T`.
//! *Read-Modify-Write* operations (`swap`, `compare_exchange`,
//! `compare_exchange_weak`) return [`Unlinked`] values if they succeed.
//! Only values that are successfully unlinked in this manner can be retired,
//! which means they will be automatically reclaimed at some some point when it
//! is safe to do so.
//! [`Unprotected`] is useful for comparing and storing values, which do not
//! need to be de-referenced and hence don't need to be protected by hazard
//! pointers.
//!
//! # Compare-and-Swap
//!
//! The atomic [`compare_exchange`][reclaim::Atomic::compare_exchange] method of
//! the [`Atomic`] type is highly versatile and uses generics and (internal)
//! traits in order to achieve some degree of argument *overloading*.
//! The `current` and `new` arguments accept a wide variety of pointer types,
//! interchangeably.
//!
//! For instance, `current` accepts values of either types [`Shared`],
//! [`Option<Shared>`][Option], or [`Marked<Shared>`][Marked].
//! The same range of types and wrappers is also accepted for [`Unprotected`]
//! values.
//! A *compare-and-swap*  can only succeed if the `current` value is equal to
//! the value that is actually stored in the [`Atomic`].
//! Consequently, the return type of this method adapts to the input type:
//! When `current` is either a [`Shared`] or an [`Unprotected`], the return
//! type is [`Unlinked`], since all of these types are non-nullable.
//! However, when `current` is an `Option`, the return type is
//! `Option<Unlinked>`.
//!
//! The `new` argument accepts types like [`Owned`], [`Shared`], [`Unlinked`],
//! [`Unprotected`] or `Option` thereof.
//! Care has to be taken when inserting a [`Shared`] in this way, as it is
//! possible to insert the value twice at different positions of the same
//! collection, which violates the primary reclamation invariant (which is also
//! the reason why `retire` is unsafe):
//! It must be impossible for a thread to read a reference to a value that has
//! previously been retired.
//!
//! When a *compare-and-swap* fails, a [`struct`][reclaim::CompareExchangeFailure]
//! is returned that contains both the *actual* value and the value that was
//! attempted to be inserted.
//! This ensures that move-only types like [`Owned`] and [`Unlinked`] can be
//! retrieved again in the case of a failed *compare-and-swap*.
//! The actually loaded value is returned in the form a [`MarkedPtr`][reclaim::MarkedPtr].
//!
//! The other methods of [`Atomic`][Atomic] are similarly versatile in terms of
//! accepted argument types.
//!
//! # Pointer Tagging
//!
//! Many concurrent algorithms require the use of atomic pointers with
//! additional information stored in one or more of a pointer's lower bits.
//! For this purpose the [`reclaim`][reclaim] crate provides a type-based
//! generic solution for making pointer types markable.
//! The number of usable lower bits is part of the type signature of types like
//! [`Atomic`] or [`Owned`].
//! If the pointed-to type is not able to provide the required number of mark
//! bits (which depends on its alignment) this will lead to a compilation error.
//! Since the number of mark bits is part of the types themselves, using zero
//! mark bits also has zero runtime overhead.
//!
//! [1]: https://dl.acm.org/citation.cfm?id=987595
//! [reclaim]: https://github.com/oliver-giersch/reclaim

#![cfg_attr(not(any(test, feature = "std")), no_std)]
#![warn(missing_docs)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(any(test, feature = "std"))]
mod default;

mod global;
mod guard;
mod hazard;
mod local;
mod retired;

pub use reclaim;
pub use reclaim::typenum;

use cfg_if::cfg_if;
use reclaim::prelude::*;
use typenum::Unsigned;

/// A specialization of [`Atomic`][reclaim::Atomic] for the [`HP`] reclamation
/// scheme.
pub type Atomic<T, N> = reclaim::Atomic<T, HP, N>;
/// A specialization of [`Shared`][reclaim::Shared] for the [`HP`] reclamation
/// scheme.
pub type Shared<'g, T, N> = reclaim::Shared<'g, T, HP, N>;
/// A specialization of [`Owned`][reclaim::Owned] for the [`HP`] reclamation
/// scheme.
pub type Owned<T, N> = reclaim::Owned<T, HP, N>;
/// A specialization of [`Unlinked`][reclaim::Unlinked] for the [`HP`]
/// reclamation scheme.
pub type Unlinked<T, N> = reclaim::Unlinked<T, HP, N>;
/// A specialization of [`Unprotected`][reclaim::Unprotected] for the [`HP`]
/// reclamation scheme.
pub type Unprotected<T, N> = reclaim::Unprotected<T, HP, N>;

cfg_if! {
    if #[cfg(feature = "std")] {
        /// A guarded pointer that can be used to acquire hazard pointers.
        pub type Guard = crate::default::Guard;
    } else {
        pub use crate::local::{Local, RecycleError};
        /// A **thread local** guarded pointer that can be used to acquire
        /// hazard pointers.
        pub type LocalGuard<'a> = crate::guarded::Guard<&'a Local>;
    }
}

#[cfg(not(feature = "std"))]
use conquer_once::spin::OnceCell;
#[cfg(feature = "std")]
use conquer_once::OnceCell;

use crate::retired::Retired;

/// Global one-time configuration for runtime parameters used for memory
/// reclamation.
pub static CONFIG: OnceCell<Config> = OnceCell::new();

////////////////////////////////////////////////////////////////////////////////////////////////////
// HP
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Hazard Pointer based reclamation scheme.
#[derive(Debug, Default, Copy, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct HP;

/********** impl Reclaim **************************************************************************/

unsafe impl Reclaim for HP {
    type Local = crate::local::Local;
    type RecordHeader = (); // no extra header per allocated record is required

    #[inline]
    unsafe fn retire_local<T: 'static, N: Unsigned>(local: &Self::Local, unlinked: Unlinked<T, N>) {
        Self::retire_local_unchecked(local, unlinked)
    }

    #[inline]
    unsafe fn retire_local_unchecked<T, N: Unsigned>(
        local: &Self::Local,
        unlinked: Unlinked<T, N>,
    ) {
        let unmarked = Unlinked::into_marked_non_null(unlinked).decompose_non_null();
        local.retire_record(Retired::new_unchecked(unmarked));
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Config
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Runtime configuration parameters.
#[derive(Copy, Clone, Debug)]
pub struct Config {
    scan_threshold: u32,
}

/********** impl Default **************************************************************************/

impl Default for Config {
    #[inline]
    fn default() -> Self {
        Self { scan_threshold: 100 }
    }
}

/********** impl inherent *************************************************************************/

impl Config {
    /// Creates a new [`Config`] with the given parameters
    ///
    /// # Panics
    ///
    /// This function panics, if `scan_threshold` is 0.
    #[inline]
    pub fn with_params(scan_threshold: u32) -> Self {
        assert!(scan_threshold > 0, "scan threshold must be greater than 0");
        Self { scan_threshold }
    }

    /// Returns the scan threshold.
    #[inline]
    pub fn scan_threshold(&self) -> u32 {
        self.scan_threshold
    }
}

// The ThreadSanitizer can not correctly asses ordering restraints from explicit
// fences, so memory operations around such fences need stricter ordering than
// `Relaxed`, when instrumentation is chosen.

#[cfg(not(feature = "sanitize-threads"))]
mod sanitize {
    use core::sync::atomic::Ordering;

    pub const RELAXED_LOAD: Ordering = Ordering::Relaxed;
    pub const RELAXED_STORE: Ordering = Ordering::Relaxed;

    pub const RELEASE_SUCCESS: Ordering = Ordering::Release;
    pub const RELEASE_FAIL: Ordering = Ordering::Relaxed;
}

#[cfg(feature = "sanitize-threads")]
mod sanitize {
    use core::sync::atomic::Ordering;

    pub const RELAXED_LOAD: Ordering = Ordering::Acquire;
    pub const RELAXED_STORE: Ordering = Ordering::Release;

    pub const RELEASE_SUCCESS: Ordering = Ordering::AcqRel;
    pub const RELEASE_FAIL: Ordering = Ordering::Acquire;
}
