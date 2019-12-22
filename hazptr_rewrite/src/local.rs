use core::cell::UnsafeCell;
use core::convert::AsRef;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ptr;
use core::sync::atomic::Ordering;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        use std::sync::Arc;
        use std::rc::Rc;
        use std::vec::Vec;
    } else {
        use alloc::sync::Arc;
        use alloc::rc::Rc;
        use alloc::vec::Vec;
    }
}

use arrayvec::{ArrayVec, CapacityError};
use conquer_reclaim::{RawRetired, Reclaim, ReclaimerLocalRef, Retired};

use crate::config::{Config, Operation};
use crate::global::GlobalHandle;
use crate::guard::Guard;
use crate::hazard::{HazardPtr, ProtectStrategy, ProtectedPtr};
use crate::policy::Policy;
use crate::{ArcHp, Hp};

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct LocalHandle<'local, 'global, P: Policy, R: Reclaim> {
    inner: LocalRef<'local, 'global, P>,
    _marker: PhantomData<R>,
}

/*********** impl Clone ***************************************************************************/

impl<'local, 'global, P: Policy, R: Reclaim> Clone for LocalHandle<'local, 'global, P, R> {
    #[inline]
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), _marker: PhantomData }
    }
}

/********** impl inherent *************************************************************************/

impl<'global, P: Policy, R: Reclaim> LocalHandle<'_, 'global, P, R> {
    #[inline]
    pub fn new(config: Config, global: GlobalHandle<'global, P>) -> Self {
        Self { inner: LocalRef::Rc(Rc::new(Local::new(config, global))), _marker: PhantomData }
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

impl<'local, 'global, P: Policy, R: Reclaim> LocalHandle<'local, 'global, P, R> {
    #[inline]
    pub fn from_ref(local: &'local Local<'global, P>) -> Self {
        Self { inner: LocalRef::Ref(local), _marker: PhantomData }
    }
}

/*********** impl AsRef ***************************************************************************/

impl<'local, 'global, P: Policy, R: Reclaim> AsRef<Local<'global, P>>
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

/********** impl ReclaimerRef *********************************************************************/

unsafe impl<'local, 'global, P: Policy> ReclaimerLocalRef
    for LocalHandle<'local, 'global, P, ArcHp<P>>
{
    type Guard = Guard<'local, 'global, P, Self::Reclaimer>;
    type Reclaimer = ArcHp<P>;

    #[inline]
    fn from_ref(global: &Self::Reclaimer) -> Self {
        Self::new(Default::default(), GlobalHandle::owned(Arc::clone(&global.handle)))
    }

    #[inline]
    unsafe fn from_raw(global: &Self::Reclaimer) -> Self {
        Self::new(Default::default(), GlobalHandle::from_raw())
    }

    #[inline]
    fn into_guard(self) -> Self::Guard {
        Guard::with_handle(self)
    }

    #[inline]
    unsafe fn retire(self, record: Retired<Self::Reclaimer>) {
        self.as_ref().retire(record.into_raw())
    }
}

unsafe impl<'local, 'global, P> ReclaimerLocalRef for LocalHandle<'local, 'global, P, Hp<P>>
where
    P: Policy,
{
    type Guard = Guard<'local, 'global, P, Self::Reclaimer>;
    type Reclaimer = Hp<P>;

    #[inline]
    fn from_ref(global: &Self::Reclaimer) -> Self {
        LocalHandle::new(Default::default(), GlobalHandle::from_ref(&global.state))
    }

    unsafe fn from_raw(global: *const Self::Reclaimer) -> Self {
        unimplemented!()
    }

    fn into_guard(self) -> Self::Guard {
        unimplemented!()
    }

    unsafe fn retire(self, retired: Retired<Self::Reclaimer>) {
        unimplemented!()
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
    pub fn new(config: Config, global: GlobalHandle<'global, P>) -> Self {
        Self { inner: UnsafeCell::new(LocalInner::new(config, global)) }
    }

    #[inline]
    pub(crate) fn config(&self) -> &Config {
        unsafe { &(*self.inner.get()).config }
    }

    #[inline]
    pub(crate) fn try_increase_ops_count(&self, op: Operation) {
        unsafe { (*self.inner.get()).try_increase_ops_count(op) }
    }

    #[inline]
    pub(crate) fn retire(&self, retired: RawRetired) {
        unsafe { (*self.inner.get()).retire(retired) };
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
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalInner
////////////////////////////////////////////////////////////////////////////////////////////////////

const HAZARD_CACHE: usize = 16;

struct LocalInner<'global, P: Policy> {
    config: Config,
    global: GlobalHandle<'global, P>,
    state: ManuallyDrop<P>,
    ops_count: u32,
    hazard_cache: ArrayVec<[&'global HazardPtr; HAZARD_CACHE]>,
    scan_cache: Vec<ProtectedPtr>,
}

/********** impl inherent *************************************************************************/

impl<'global, P: Policy> LocalInner<'global, P> {
    #[inline]
    fn new(config: Config, global: GlobalHandle<'global, P>) -> Self {
        Self {
            config,
            global,
            state: Default::default(),
            ops_count: Default::default(),
            hazard_cache: Default::default(),
            scan_cache: Default::default(),
        }
    }

    #[inline]
    fn try_increase_ops_count(&mut self, op: Operation) {
        if op == self.config.count_strategy {
            self.ops_count += 1;

            if self.ops_count == self.config.ops_count_threshold {
                self.ops_count = 0;
                unimplemented!() // fixme: scan records
            }
        }
    }

    #[inline]
    fn retire(&mut self, retired: RawRetired) {
        unsafe { self.state.retire(&self.global.as_ref().state, retired) };
        if self.config.is_count_retire() {
            self.ops_count += 1;
        }
    }

    #[inline]
    fn get_hazard(&mut self, strategy: ProtectStrategy) -> &HazardPtr {
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
    fn try_recycle_hazard(&mut self, hazard: &'global HazardPtr) -> Result<(), RecycleError> {
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

impl From<CapacityError<&'_ HazardPtr>> for RecycleError {
    #[inline]
    fn from(_: CapacityError<&HazardPtr>) -> Self {
        RecycleError
    }
}
