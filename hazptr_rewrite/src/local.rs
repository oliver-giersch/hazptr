use core::cell::UnsafeCell;
use core::convert::AsRef;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        use std::rc::Rc;
        use std::sync::Arc;
        use std::vec::Vec;
    } else {
        use alloc::rc::Rc;
        use alloc::sync::Arc;
        use alloc::vec::Vec;
    }
}

use arrayvec::ArrayVec;
use conquer_reclaim::{ReclaimerHandle, Retired};

use crate::global::{Global, GlobalHandle};
use crate::guard::Guard;
use crate::hazard::{Hazard, Protected};
use crate::policy::Policy;
use crate::HPHandle;

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct LocalHandle<'local, 'global, P: Policy> {
    inner: LocalRef<'local, 'global, P>,
}

/*********** impl AsRef ***************************************************************************/

impl<'local, 'global, P: Policy> AsRef<Local<'global, P>> for LocalHandle<'local, 'global, P> {
    #[inline]
    fn as_ref(&self) -> &Local<'global, P> {
        match &self.inner {
            LocalRef::Rc(local) => local.as_ref(),
            LocalRef::Ref(local) => local,
            LocalRef::Raw(local) => unsafe { &*local },
        }
    }
}

/*********** impl Clone ***************************************************************************/

impl<'local, 'global, P: Policy> Clone for LocalHandle<'local, 'global, P> {
    #[inline]
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

/********** impl inherent *************************************************************************/

impl<'global, P: Policy> LocalHandle<'static, 'global, P> {
    #[inline]
    pub fn owning(global: GlobalHandle<'global, P>) -> Self {
        Self { inner: LocalRef::Rc(Rc::new(Local::new(global))) }
    }

    #[inline]
    pub fn from_owned(local: Rc<Local<'global, P>>) -> Self {
        Self { inner: LocalRef::Rc(local) }
    }

    #[inline]
    pub unsafe fn from_raw(local: *const Local<'global, P>) -> Self {
        Self { inner: LocalRef::Raw(local) }
    }
}

impl<'local, 'global, P: Policy> LocalHandle<'local, 'global, P> {
    #[inline]
    pub fn from_ref(local: &'local Local<'global, P>) -> Self {
        Self { inner: LocalRef::Ref(local) }
    }
}

/********** impl ReclaimerHandle ******************************************************************/

unsafe impl<'local, 'global, P: Policy> ReclaimerHandle for LocalHandle<'local, 'global, P> {
    type Reclaimer = HPHandle<P>;
    type Guard = Guard<'local, 'global, P>;

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
    pub fn get_hazard(&self) -> &'global Hazard {
        unsafe {
            match (*self.inner.get()).hazard_cache.pop() {
                Some(hazard) => hazard,
                None => self.global.as_ref().get_hazard(),
            }
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalInner
////////////////////////////////////////////////////////////////////////////////////////////////////

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
