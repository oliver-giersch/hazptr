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

use conquer_reclaim::{BuildReclaimRef, RawRetired, Reclaim, ReclaimRef, Retired};

use crate::config::{Config, Operation};
use crate::global::GlobalRef;
use crate::guard::Guard;
use crate::hazard::{HazardPtr, ProtectStrategy};
use crate::retire::RetireStrategy;
use crate::Hp;

use self::inner::{LocalInner, RecycleError};

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

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
    #[inline]
    pub(crate) fn new(config: Config, global: GlobalRef<'global>) -> Self {
        Self { inner: Ref::Rc(Rc::new(Local::new(config, global))), _marker: PhantomData }
    }

    #[inline]
    pub fn from_owned(local: Rc<Local<'global>>) -> Self {
        Self { inner: Ref::Rc(local), _marker: PhantomData }
    }

    #[inline]
    pub unsafe fn from_raw(local: *const Local<'global>) -> Self {
        Self { inner: Ref::Raw(local), _marker: PhantomData }
    }
}

impl<'local, 'global, R> LocalHandle<'local, 'global, R> {
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

/********** impl BuildReclaimRef ******************************************************************/

impl<'global, S: RetireStrategy> BuildReclaimRef<'global> for LocalHandle<'_, 'global, Hp<S>>
where
    Self: 'global,
    Hp<S>: Reclaim,
{
    #[inline]
    fn from_ref(global: &'global Self::Reclaimer) -> Self {
        Self::new(Default::default(), GlobalRef::from_ref(&global.state))
    }
}

/********** impl ReclaimRef ***********************************************************************/

unsafe impl<'local, 'global, S: RetireStrategy> ReclaimRef for LocalHandle<'local, 'global, Hp<S>>
where
    Hp<S>: Reclaim,
{
    type Guard = Guard<'local, 'global, Self::Reclaimer>;
    type Reclaimer = Hp<S>;

    #[inline]
    unsafe fn from_raw(global: &Self::Reclaimer) -> Self {
        Self::new(Default::default(), GlobalRef::from_raw(&global.state))
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
// Ref
////////////////////////////////////////////////////////////////////////////////////////////////////

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
