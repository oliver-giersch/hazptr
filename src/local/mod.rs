mod inner;

use core::cell::UnsafeCell;
use core::convert::AsRef;
use core::marker::PhantomData;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        use std::rc::Rc;
    } else {
        use alloc::rc::Rc;
        use alloc::vec::Vec;
    }
}

use conquer_reclaim::{LocalState, Reclaim, Retired, RetiredPtr};

use crate::config::{Config, Operation};
use crate::global::GlobalRef;
use crate::guard::Guard;
use crate::hazard::{HazardPtr, ProtectStrategy};

use self::inner::{LocalInner, RecycleError};

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A handle to the thread-local ([`Local`]) state.
///
/// This type abstracts over the ownership of the local state, which may either
/// be owned through a shared pointer or borrowed through a reference or raw
/// pointer (unsafely).
#[derive(Debug)]
pub struct LocalHandle<'local, 'global, R> {
    inner: Ref<'local, 'global>,
    _marker: PhantomData<R>,
}

/*********** impl Clone ***************************************************************************/

impl<R> Clone for LocalHandle<'_, '_, R> {
    #[inline]
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), _marker: PhantomData }
    }
}

/********** impl inherent *************************************************************************/

impl<'global, R> LocalHandle<'_, 'global, R> {
    /// Creates a new (owned) `Local` state instance from the supplied
    /// arguments and returns an owning `LocalHandle` for it.
    #[inline]
    pub(crate) fn new(config: Config, global: GlobalRef<'global>) -> Self {
        Self { inner: Ref::Rc(Rc::new(Local::new(config, global))), _marker: PhantomData }
    }

    /// Creates a new owning `LocalHandle` from an existing [`Rc`] (shared
    /// pointer).
    #[inline]
    pub fn from_owned(local: Rc<Local<'global>>) -> Self {
        Self { inner: Ref::Rc(local), _marker: PhantomData }
    }

    /// Creates a new borrowing `LocalHandle` from a raw pointer.
    ///
    /// # Safety
    ///
    /// The caller has to ensure the `LocalHandle` handle does not outlive the
    /// pointed to `Local`.
    #[inline]
    pub unsafe fn from_raw(local: *const Local<'global>) -> Self {
        Self { inner: Ref::Raw(local), _marker: PhantomData }
    }
}

impl<'local, 'global, R> LocalHandle<'local, 'global, R> {
    /// Creates a new borrowing `LocalHandle` from a shared reference.
    #[inline]
    pub fn from_ref(local: &'local Local<'global>) -> Self {
        Self { inner: Ref::Ref(local), _marker: PhantomData }
    }
}

/*********** impl AsRef ***************************************************************************/

impl<'global, R> AsRef<Local<'global>> for LocalHandle<'_, 'global, R> {
    #[inline]
    fn as_ref(&self) -> &Local<'global> {
        match &self.inner {
            Ref::Rc(local) => local.as_ref(),
            Ref::Ref(local) => local,
            Ref::Raw(local) => unsafe { &**local },
        }
    }
}

/********** impl LocalState ***********************************************************************/

unsafe impl<'local, 'global, R: Reclaim> LocalState for LocalHandle<'local, 'global, R> {
    type Guard = Guard<'local, 'global, Self::Reclaimer>;
    type Reclaimer = R;

    #[inline]
    fn build_guard(&self) -> Self::Guard {
        Guard::with_handle(self.clone())
    }

    #[inline]
    unsafe fn retire_record(&self, retired: Retired<Self::Reclaimer>) {
        self.inner.as_ref().retire_record(retired.into_raw())
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Local
////////////////////////////////////////////////////////////////////////////////////////////////////

/// The local state of a thread using hazard pointers.
#[derive(Debug)]
pub struct Local<'global> {
    inner: UnsafeCell<LocalInner<'global>>,
}

/********** impl inherent *************************************************************************/

impl<'global> Local<'global> {
    #[inline]
    pub(crate) fn new(config: Config, global: GlobalRef<'global>) -> Self {
        Self { inner: UnsafeCell::new(LocalInner::new(config, global)) }
    }

    #[inline]
    pub(crate) fn count_strategy(&self) -> Operation {
        unsafe { (*self.inner.get()).count_strategy() }
    }

    #[inline]
    pub(crate) fn increase_ops_count(&self) {
        unsafe { (*self.inner.get()).increase_ops_count() }
    }

    #[inline]
    pub(crate) fn get_hazard(&self, strategy: ProtectStrategy) -> &HazardPtr {
        unsafe { (*self.inner.get()).get_hazard(strategy) }
    }

    #[inline]
    pub(crate) fn try_recycle_hazard(
        &self,
        hazard: &'global HazardPtr,
    ) -> Result<(), RecycleError> {
        unsafe { (*self.inner.get()).try_recycle_hazard(hazard) }
    }

    #[inline]
    pub(crate) unsafe fn retire_record(&self, retired: RetiredPtr) {
        (*self.inner.get()).retire_record(retired);
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Ref
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An abstraction for an owning or borrowing reference to a `Local` instance.
#[derive(Debug)]
enum Ref<'local, 'global> {
    Rc(Rc<Local<'global>>),
    Ref(&'local Local<'global>),
    Raw(*const Local<'global>),
}

/********** impl AsRef ****************************************************************************/

impl<'global> AsRef<Local<'global>> for Ref<'_, 'global> {
    #[inline]
    fn as_ref(&self) -> &Local<'global> {
        match self {
            Ref::Rc(local) => &**local,
            Ref::Ref(local) => *local,
            Ref::Raw(local) => unsafe { &**local },
        }
    }
}

/********** impl Clone ****************************************************************************/

impl<'local, 'global> Clone for Ref<'local, 'global> {
    #[inline]
    fn clone(&self) -> Self {
        match self {
            Ref::Rc(local) => Ref::Rc(Rc::clone(local)),
            Ref::Ref(local) => Ref::Ref(*local),
            Ref::Raw(local) => Ref::Raw(*local),
        }
    }
}
