use core::cell::UnsafeCell;
use core::convert::AsRef;
use core::marker::PhantomData;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        use std::rc::Rc;
        use std::vec::Vec;
    } else {
        use alloc::rc::Rc;
        use alloc::vec::Vec;
    }
}

use arrayvec::ArrayVec;
use conquer_reclaim::{Reclaimer, ReclaimerHandle, Retired};

use crate::global::GlobalHandle;
use crate::guard::Guard;
use crate::hazard::{Hazard, Protected};
use crate::policy::Policy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct LocalHandle<'local, 'global, P: Policy, R: Reclaimer> {
    inner: LocalRef<'local, 'global, P>,
    _marker: PhantomData<R>,
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

/********** impl ReclaimerHandle ******************************************************************/

unsafe impl<'local, 'global, P: Policy, R: Reclaimer> ReclaimerHandle
    for LocalHandle<'local, 'global, P, R>
{
    type Reclaimer = R;
    type Guard = Guard<'local, 'global, P, R>;

    #[inline]
    fn guard(self) -> Self::Guard {
        Guard::with_handle(self)
    }

    #[inline]
    unsafe fn retire(self, record: Retired<Self::Reclaimer>) {
        unimplemented!()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Local
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct Local<'global, P: Policy> {
    inner: UnsafeCell<LocalInner<'global, P>>,
    global: GlobalHandle<'global, P>,
}

/********** impl inherent *************************************************************************/

impl<'global, P: Policy> Local<'global, P> {
    #[inline]
    pub fn new(global: GlobalHandle<'global, P>) -> Self {
        Self { inner: UnsafeCell::new(unimplemented!()), global }
    }

    #[inline]
    pub fn get_hazard(&self) -> &Hazard {
        unsafe {
            match (*self.inner.get()).hazard_cache.pop() {
                Some(hazard) => hazard,
                None => {
                    self.global.as_ref().get_hazard(crate::hazard::ProtectStrategy::ReserveOnly)
                } // FIXME: import
            }
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalInner
////////////////////////////////////////////////////////////////////////////////////////////////////

const HAZARD_CACHE: usize = 16;

struct LocalInner<'global, P: Policy> {
    state: P::LocalState,
    // config: ???, how to count, thresholds, etc
    ops_count: u32,
    flush_count: u32,
    hazard_cache: ArrayVec<[&'global Hazard; HAZARD_CACHE]>,
    scan_cache: Vec<Protected>,
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
