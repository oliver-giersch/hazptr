//! Hazard pointer based concurrent memory reclamation.
//!
//! A difficult problem that has to be considered when implementing lock-free collections or data
//! structures is deciding, when a removed entry can be safely deallocated.
//! It is usually not correct to deallocate removed entries right away, because different threads
//! might still hold references to such entries and could consequently access already freed memory.
//!
//! Concurrent memory reclamation schemes solve that problem by extending the lifetime of removed
//! entries for a certain *grace period*.
//! After this period it must be impossible for other threads to have any references to these
//! entries anymore and they can be finally deallocated.
//! This is similar to the concept of *Garbage Collection* in languages like Go and Java, but with
//! a much more limited scope.
//!
//! The Hazard-pointer reclamation scheme was described by Maged M. Michael in 2004 [[1]].
//! It requires every *read* of an entry from shared memory to be accompanied by a global
//! announcement marking the read entry as protected.
//! Threads must store removed (retired) entries in a local cache and regularly attempt to reclaim
//! all cached records in bulk.
//! A record is safe to be reclaimed, once there is no hazard pointer protecting it anymore.
//!
//! # Reclamation Interface and Pointer Types
//!
//! The API of this library follows the abstract interface defined by the [`reclaim`][reclaim]
//! crate.
//! Hence, it uses the following types for atomically reading and writing from and to shared memory:
//!
//! - [`Atomic`][Atomic]
//! - [`Owned`][Owned]
//! - [`Shared`][Shared]
//! - [`Unlinked`][Unlinked]
//! - [`Unprotected`][Unprotected]
//!
//! The primary type exposed by this API is [`Atomic`][Atomic], which is a shared atomic pointer
//! with similar semantics to `Option<Box<T>>`.
//! It provides all operations that are also supported by `AtomicPtr`, such as `store`, `load` or
//! `compare_exchange`.
//! All *load* operations on an [`Atomic`][Atomic] return (optional) [`Shared`][Shared] references.
//! [`Shared`][Shared] is a non-nullable pointer type that is protected by a hazard pointer and has
//! similar semantics to `&T`.
//! *Read-Modify-Write* operations (`swap`, `compare_exchange`, `compare_exchange_weak`) return
//! [`Unlinked`][Unlinked] values if they succeed.
//! Only values that are successfully unlinked in this manner can be retired, which means they will
//! be automatically reclaimed at some some point when it is safe to do so.
//! [`Unprotected`][Unprotected] is useful for comparing and storing values, which do not need to be
//! de-referenced and hence don't need to be protected by hazard pointers.
//!
//! # Compare-and-Swap
//!
//! The atomic [`compare_exchange`][compare_exchange] method of the [`Atomic`][Atomic] type is
//! highly versatile and uses generics and (internal) traits in order to achieve some degree of
//! argument *overloading*.
//! The `current` and `new` arguments accept a wide variety of pointer types interchangeably.
//!
//! For instance, `current` accepts values of both types [`Shared`][Shared] and `Option<Shared>`.
//! It also accepts [`Unprotected`][Unprotected] or `Option<Unprotected>` values.
//! A *compare-and-swap*  can only succeed if the `current` value is equal to the value that is
//! actually stored in the [`Atomic`][Atomic].
//! Consequently, the return type of this method adapts to the input type:
//! When `current` is either a [`Shared`][Shared] or an [`Unprotected`][Unprotected], the return
//! type is [`Unlinked`][Unlinked], since all of these types are non-nullable.
//! However, when `current` is an `Option`, the return type is `Option<Unlinked>`.
//!
//! The `new` argument accepts types like [`Owned`][Owned], [`Shared`][Shared], [`Unlinked`][Unlinked],
//! [`Unprotected`][Unprotected] or `Option` thereof.
//! Care has to be taken when inserting a [`Shared`][Shared] in this way, as it is possible to
//! insert the value twice at different positions of the same collection, which violates the primary
//! reclamation invariant (which is also the reason why `retire` is unsafe):
//! It must be impossible for a thread to read a reference to a value that has previously been
//! retired.
//!
//! When a *compare-and-swap* fails, a `struct` is returned that contains both the *actual* value
//! and the value that was attempted to be inserted.
//! This ensures that move-only types like [`Owned`][Owned] and [`Unlinked`][Unlinked] can be
//! retrieved again in the case of a failed *compare-and-swap*.
//!
//! The other methods of [`Atomic`][Atomic] are similarly versatile in terms of accepted argument
//! types.
//!
//! # Pointer Tagging
//!
//! Many concurrent algorithms require the use of atomic pointers with additional information stored
//! in one or more of a pointer's lower bits.
//! For this purpose the [`reclaim`][reclaim] crate provides a type-based generic solution for
//! making pointer types markable.
//! The number of usable lower bits is part of the type signature of types like [`Atomic`][Atomic]
//! or [`Owned`][Owned].
//! If the pointed-to type is not able to provide the required number of mark bits (which depends on
//! its alignment) this will lead to a compilation error.
//! Since the number of mark bits is part of the types themselves, using zero mark bits also has
//! zero runtime overhead.
//!
//! [1]: https://dl.acm.org/citation.cfm?id=987595
//! [reclaim]: https://github.com/oliver-giersch/reclaim
//! [Atomic]: Atomic
//! [Shared]: Shared
//! [Unlinked]: Unlinked
//! [Unprotected]: Unprotected
//! [Owned]: Owned
//! [compare_exchange]: reclaim::Atomic::compare_exchange

#![cfg_attr(feature = "no-std", feature(alloc))]
#![cfg_attr(feature = "no-std", no_std)]

#[cfg(feature = "no-std")]
extern crate alloc;

pub use reclaim;
pub use reclaim::typenum;

use reclaim::LocalReclaim;
use typenum::Unsigned;

/// Atomic pointer that must be either `null` or valid. Loads of non-null values must acquire hazard
/// pointers and are hence protected from reclamation.
pub type Atomic<T, N> = reclaim::Atomic<T, HP, N>;
/// Shared reference to a value loaded from shared memory that is protected by a hazard pointer.
pub type Shared<'g, T, N> = reclaim::Shared<'g, T, HP, N>;
/// A pointer type for heap allocation like [`Box`](std::boxed::Box) that can be marked.
pub type Owned<T, N> = reclaim::Owned<T, HP, N>;
/// Unique reference to a value that has been successfully unlinked and can be retired.
pub type Unlinked<T, N> = reclaim::Unlinked<T, HP, N>;
/// Shared reference to a value loaded from shared memory that is **not** safe to dereference and
/// could be reclaimed at any point.
pub type Unprotected<T, N> = reclaim::Unprotected<T, HP, N>;

#[cfg(not(feature = "no-std"))]
mod default;

mod global;
mod guarded;
mod hazard;
mod local;
mod retired;
#[cfg(test)]
mod tests;

#[cfg(not(feature = "no-std"))]
pub use crate::default::guarded;
#[cfg(not(feature = "no-std"))]
pub type Guarded<T, N> = crate::guarded::Guarded<T, crate::default::DefaultAccess, N>;

#[cfg(feature = "no-std")]
pub use crate::{
    global::Global,
    hazard::{Hazard, HazardPtr},
    local::{Local, LocalAccess, RecycleErr},
};
#[cfg(feature = "no-std")]
pub type Guarded<'a, T, N> = crate::guarded::Guarded<T, &'a Local, N>;

use crate::retired::Retired;

////////////////////////////////////////////////////////////////////////////////////////////////////
// HP
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Hazard Pointer based reclamation scheme.
#[derive(Debug, Default, Copy, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct HP;

unsafe impl LocalReclaim for HP {
    type Local = crate::local::Local;
    // hazard pointers do not require any extra information per each allocated record
    type RecordHeader = ();

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
