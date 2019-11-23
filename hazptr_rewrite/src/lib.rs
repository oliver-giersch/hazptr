#![cfg_attr(not(any(test, feature = "std")), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

mod default;
mod global;
mod guard;
mod hazard;
mod local;
mod policy;
mod queue;

use core::marker::PhantomData;
use core::sync::atomic::AtomicPtr;

#[cfg(not(feature = "std"))]
use alloc::sync::Arc;
#[cfg(feature = "std")]
use std::sync::Arc;

use conquer_reclaim::{Reclaim, ReclaimHandle, Record};

use crate::global::Global;
use crate::local::Local;
use crate::policy::Policy;

////////////////////////////////////////////////////////////////////////////////////////////////////
// HP
////////////////////////////////////////////////////////////////////////////////////////////////////

struct HP<P>(PhantomData<P>);

/********** impl Reclaim **************************************************************************/

unsafe impl<P: Policy> Reclaim for HP<P> {
    type DefaultHandle = Local<'static, P>;
    type Header = P::Header;
    type Global = Global<P>;
}
