use core::marker::PhantomData;
use core::mem;

// TODO: if .. else
use alloc::rc::Rc;
use alloc::sync::Arc;

use crate::global::{Global, GlobalHandle};
use crate::policy::Policy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Local
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct Local<'global, P: Policy> {
    state: P::LocalState,
    global: GlobalHandle<'global, P>,
}

impl<P: Policy> Local<'static, P> {
    #[inline]
    pub fn from_owned(global: Arc<Global<P>>) -> Self {
        Self { state: unimplemented!(), global: GlobalHandle::Arc(global) }
    }

    #[inline]
    pub unsafe fn from_raw(global: *const Global<P>) -> Self {
        Self { state: unimplemented!(), global: GlobalHandle::Raw(global) }
    }
}

impl<'global, P: Policy> Local<'global, P> {
    #[inline]
    pub fn from_ref(global: &'global Global<P>) -> Self {
        Self { state: unimplemented!(), global: GlobalHandle::Ref(global) }
    }
}

impl<P: Policy> Drop for Local<'_, P> {
    #[inline]
    fn drop(&mut self) {
        match self.global {
            GlobalHandle::Arc(_) => {
                let global = unsafe { Arc::from_raw(self.global) };
                // P::drop_local(&*global);
                mem::drop(global);
            }
            GlobalHandle::Ref(_) => unimplemented!(),
            GlobalHandle::Raw(_) => unimplemented!(),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

pub enum LocalHandle<'local, 'global, P: Policy> {
    Rc(Rc<Local<'global, P>>),
    Ref(&'local Local<'global, P>),
    Raw(*const Local<'global, P>),
}
