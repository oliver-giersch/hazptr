use core::cell::UnsafeCell;
use core::convert::AsRef;
use core::mem::ManuallyDrop;
use core::ptr;
use core::sync::atomic::Ordering;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        use std::rc::Rc;
    } else {
        use alloc::rc::Rc;
        use alloc::vec::Vec;
    }
}

use arrayvec::{ArrayVec, CapacityError};
use conquer_reclaim::{BuildReclaimRef, RawRetired, Reclaim, ReclaimRef, Retired};

use crate::config::{Config, Operation};
use crate::global::GlobalHandle;
use crate::guard::Guard;
use crate::hazard::{HazardPtr, ProtectStrategy, ProtectedPtr};
use crate::retire::RetireStrategy;
use crate::Hp;

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct LocalHandle<'local, 'global, S: RetireStrategy> {
    inner: LocalRef<'local, 'global, S>,
}

/*********** impl Clone ***************************************************************************/

impl<'local, 'global, S: RetireStrategy> Clone for LocalHandle<'local, 'global, S> {
    #[inline]
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

/********** impl inherent *************************************************************************/

impl<'global, S: RetireStrategy> LocalHandle<'_, 'global, S> {
    #[inline]
    pub(crate) fn new(config: Config, global: GlobalHandle<'global, S>) -> Self {
        Self { inner: LocalRef::Rc(Rc::new(Local::new(config, global))) }
    }

    #[inline]
    pub fn from_owned(local: Rc<Local<'global, S>>) -> Self {
        Self { inner: LocalRef::Rc(local) }
    }

    #[inline]
    pub unsafe fn from_raw(local: *const Local<'global, S>) -> Self {
        Self { inner: LocalRef::Raw(local) }
    }
}

impl<'local, 'global, S: RetireStrategy> LocalHandle<'local, 'global, S> {
    #[inline]
    pub fn from_ref(local: &'local Local<'global, S>) -> Self {
        Self { inner: LocalRef::Ref(local) }
    }
}

/*********** impl AsRef ***************************************************************************/

impl<'local, 'global, S> AsRef<Local<'global, S>> for LocalHandle<'local, 'global, S>
where
    S: RetireStrategy,
{
    #[inline]
    fn as_ref(&self) -> &Local<'global, S> {
        match &self.inner {
            LocalRef::Rc(local) => local.as_ref(),
            LocalRef::Ref(local) => local,
            LocalRef::Raw(local) => unsafe { &**local },
        }
    }
}

/********** impl BuildReclaimRef ******************************************************************/

impl<'local, 'global, S> BuildReclaimRef<'global> for LocalHandle<'local, 'global, S>
where
    Self: 'global,
    S: RetireStrategy,
{
    #[inline]
    fn from_ref(global: &'global Self::Reclaimer) -> Self {
        Self::new(Default::default(), GlobalHandle::from_ref(&global.state))
    }
}

/********** impl ReclaimRef ***********************************************************************/

unsafe impl<'local, 'global, S> ReclaimRef for LocalHandle<'local, 'global, S>
where
    S: RetireStrategy,
{
    type Guard = Guard<'local, 'global, S, Self::Reclaimer>;
    type Reclaimer = Hp<S>;

    #[inline]
    unsafe fn from_raw(global: &Self::Reclaimer) -> Self {
        Self::new(Default::default(), GlobalHandle::from_raw(&global.state))
    }

    #[inline]
    fn into_guard(self) -> Self::Guard {
        Guard::with_handle(self)
    }

    #[inline]
    unsafe fn retire(self, retired: Retired<Self::Reclaimer>) {
        self.inner.as_ref().retire(retired.into_raw())
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Local
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct Local<'global, S: RetireStrategy> {
    inner: UnsafeCell<LocalInner<'global, S>>,
}

/********** impl inherent *************************************************************************/

impl<'global, S: RetireStrategy> Local<'global, S> {
    #[inline]
    pub(crate) fn new(config: Config, global: GlobalHandle<'global, S>) -> Self {
        Self { inner: UnsafeCell::new(LocalInner::new(config, global)) }
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

#[derive(Debug)]
struct LocalInner<'global, S: RetireStrategy> {
    config: Config,
    global: GlobalHandle<'global, S>,
    state: ManuallyDrop<S>,
    ops_count: u32,
    hazard_cache: ArrayVec<[&'global HazardPtr; HAZARD_CACHE]>,
    scan_cache: Vec<ProtectedPtr>,
}

/********** impl inherent *************************************************************************/

impl<'global, S: RetireStrategy> LocalInner<'global, S> {
    #[inline]
    fn new(config: Config, global: GlobalHandle<'global, S>) -> Self {
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
                self.reclaim_all_unprotected();
            }
        }
    }

    #[inline]
    fn retire(&mut self, retired: RawRetired) {
        unsafe { self.state.retire(self.global.as_ref(), retired) };
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
        // TODO: use small vec, incorporate config
        self.hazard_cache.try_push(hazard)?;
        hazard.set_thread_reserved(Ordering::Release);

        Ok(())
    }

    #[inline]
    fn reclaim_all_unprotected(&mut self) {
        let global = self.global.as_ref();
        if self.state.no_retired_records(global) {
            return;
        }

        // collect into scan_cache
        self.global.as_ref().collect_protected_hazards(&mut self.scan_cache, Ordering::SeqCst);

        self.scan_cache.sort_unstable();
        unsafe { self.state.reclaim_all_unprotected(global, &self.scan_cache) };
    }
}

/********** impl Drop *****************************************************************************/

impl<S: RetireStrategy> Drop for LocalInner<'_, S> {
    #[inline(never)]
    fn drop(&mut self) {
        for hazard in self.hazard_cache.iter() {
            hazard.set_free(Ordering::Relaxed);
        }

        // do a final reclaim attempt

        let local_state = unsafe { ptr::read(&*self.state) };
        local_state.drop(&self.global.as_ref());
        unimplemented!()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalRef
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
enum LocalRef<'local, 'global, S: RetireStrategy> {
    Rc(Rc<Local<'global, S>>),
    Ref(&'local Local<'global, S>),
    Raw(*const Local<'global, S>),
}

/********** impl AsRef ****************************************************************************/

impl<'global, S: RetireStrategy> AsRef<Local<'global, S>> for LocalRef<'_, 'global, S> {
    #[inline]
    fn as_ref(&self) -> &Local<'global, S> {
        match self {
            LocalRef::Rc(local) => &**local,
            LocalRef::Ref(local) => *local,
            LocalRef::Raw(local) => unsafe { &**local },
        }
    }
}

/********** impl Clone ****************************************************************************/

impl<'local, 'global, S: RetireStrategy> Clone for LocalRef<'local, 'global, S> {
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
