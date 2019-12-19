use core::cell::UnsafeCell;
use core::convert::AsRef;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ptr;
use core::sync::atomic::Ordering;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        use std::rc::Rc;
        use std::vec::Vec;
    } else {
        use alloc::rc::Rc;
        use alloc::vec::Vec;
    }
}

use arrayvec::{ArrayVec, CapacityError};
use conquer_reclaim::{RawRetired, Reclaimer, ReclaimerHandle, Retired};

use crate::global::GlobalHandle;
use crate::guard::Guard;
use crate::hazard::{Hazard, ProtectStrategy, Protected};
use crate::policy::Policy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct LocalHandle<'local, 'global, P: Policy, R: Reclaimer> {
    inner: LocalRef<'local, 'global, P>,
    _marker: PhantomData<R>,
}

/*********** impl Clone ***************************************************************************/

impl<'local, 'global, P: Policy, R: Reclaimer> Clone for LocalHandle<'local, 'global, P, R> {
    #[inline]
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), _marker: PhantomData }
    }
}

/********** impl inherent *************************************************************************/

impl<'global, P: Policy, R: Reclaimer> LocalHandle<'_, 'global, P, R> {
    #[inline]
    pub fn owning(global: GlobalHandle<'global, P>) -> Self {
        Self { inner: LocalRef::Rc(Rc::new(Local::new(global))), _marker: PhantomData }
    }

    #[inline]
    pub fn from_owned(local: Rc<Local<'global, P>>) -> Self {
        Self { inner: LocalRef::Rc(local), _marker: PhantomData }
    }

    #[inline]
    pub unsafe fn from_raw(local: *const Local<'global, P>) -> Self {
        Self { inner: LocalRef::Raw(local), _marker: PhantomData }
    }
}

impl<'local, 'global, P: Policy, R: Reclaimer> LocalHandle<'local, 'global, P, R> {
    #[inline]
    pub fn from_ref(local: &'local Local<'global, P>) -> Self {
        Self { inner: LocalRef::Ref(local), _marker: PhantomData }
    }
}

/*********** impl AsRef ***************************************************************************/

impl<'local, 'global, P: Policy, R: Reclaimer> AsRef<Local<'global, P>>
    for LocalHandle<'local, 'global, P, R>
{
    #[inline]
    fn as_ref(&self) -> &Local<'global, P> {
        match &self.inner {
            LocalRef::Rc(local) => local.as_ref(),
            LocalRef::Ref(local) => local,
            LocalRef::Raw(local) => unsafe { &**local },
        }
    }
}

/********** impl ReclaimerHandle ******************************************************************/

unsafe impl<'local, 'global, P, R> ReclaimerHandle for LocalHandle<'local, 'global, P, R>
where
    P: Policy,
    R: Reclaimer,
{
    type Reclaimer = R;
    type Guard = Guard<'local, 'global, P, R>;

    #[inline]
    fn guard(self) -> Self::Guard {
        Guard::with_handle(self)
    }

    #[inline]
    unsafe fn retire(self, record: Retired<Self::Reclaimer>) {
        self.as_ref().retire(record.into_raw())
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Local
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct Local<'global, P: Policy> {
    inner: UnsafeCell<LocalInner<'global, P>>,
}

/********** impl inherent *************************************************************************/

impl<'global, P: Policy> Local<'global, P> {
    #[inline]
    pub fn new(global: GlobalHandle<'global, P>) -> Self {
        Self { inner: UnsafeCell::new(LocalInner::new(global)) }
    }

    #[inline]
    pub(crate) fn retire(&self, retired: RawRetired) {
        unsafe { (*self.inner.get()).retire(retired) };
    }

    #[inline]
    pub(crate) fn get_hazard(&self, strategy: ProtectStrategy) -> &Hazard {
        unsafe { (*self.inner.get()).get_hazard(strategy) }
    }

    #[inline]
    pub(crate) fn try_recycle_hazard(&self, hazard: &'global Hazard) -> Result<(), RecycleError> {
        unsafe { (*self.inner.get()).try_recycle_hazard(hazard) }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalInner
////////////////////////////////////////////////////////////////////////////////////////////////////

const HAZARD_CACHE: usize = 16;

struct LocalInner<'global, P: Policy> {
    global: GlobalHandle<'global, P>,
    // config: ???, how to count, thresholds, etc
    state: ManuallyDrop<P::LocalState>,
    flush_count: u32,
    ops_count: u32,
    hazard_cache: ArrayVec<[&'global Hazard; HAZARD_CACHE]>,
    scan_cache: Vec<Protected>,
}

/********** impl inherent *************************************************************************/

impl<'global, P: Policy> LocalInner<'global, P> {
    #[inline]
    fn new(global: GlobalHandle<'global, P>) -> Self {
        Self {
            global,
            state: Default::default(),
            flush_count: Default::default(),
            ops_count: Default::default(),
            hazard_cache: Default::default(),
            scan_cache: Default::default(),
        }
    }

    #[inline]
    fn retire(&mut self, retired: RawRetired) {
        unsafe { P::retire(&mut *self.state, &self.global.as_ref().state) };
        // FIXME: increase op count conditionally
        unimplemented!()
    }

    #[inline]
    fn get_hazard(&mut self, strategy: ProtectStrategy) -> &Hazard {
        match self.hazard_cache.pop() {
            Some(hazard) => {
                if let ProtectStrategy::Protect(protected) = strategy {
                    hazard.set_protected(protected.into_inner(), Ordering::SeqCst);
                }

                hazard
            }
            None => self.global.as_ref().get_hazard(strategy),
        }
    }

    #[inline]
    fn try_recycle_hazard(&mut self, hazard: &'global Hazard) -> Result<(), RecycleError> {
        self.hazard_cache.try_push(hazard)?;
        hazard.set_thread_reserved(Ordering::Release);

        Ok(())
    }
}

/********** impl Drop *****************************************************************************/

impl<P: Policy> Drop for LocalInner<'_, P> {
    #[inline(never)]
    fn drop(&mut self) {
        let local_state = unsafe { ptr::read(&*self.state) };
        P::on_thread_exit(local_state, &self.global.as_ref().state);
        // P::on_thread_exit(local_state, ...);
        unimplemented!()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalRef
////////////////////////////////////////////////////////////////////////////////////////////////////

enum LocalRef<'local, 'global, P: Policy> {
    Rc(Rc<Local<'global, P>>),
    Ref(&'local Local<'global, P>),
    Raw(*const Local<'global, P>),
}

/********** impl Clone ****************************************************************************/

impl<'local, 'global, P: Policy> Clone for LocalRef<'local, 'global, P> {
    #[inline]
    fn clone(&self) -> Self {
        match self {
            LocalRef::Rc(local) => LocalRef::Rc(Rc::clone(local)),
            LocalRef::Ref(local) => LocalRef::Ref(*local),
            LocalRef::Raw(local) => LocalRef::Raw(*local),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// RecycleError
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Error type for thread local recycle operations.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RecycleError;

/********** impl From *****************************************************************************/

impl From<CapacityError<&'_ Hazard>> for RecycleError {
    #[inline]
    fn from(_: CapacityError<&Hazard>) -> Self {
        RecycleError
    }
}
