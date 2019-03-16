use std::mem;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicPtr, Ordering};

use reclaim::align::CachePadded;

use crate::local;

mod list;

pub use self::list::{HazardList, Iter};

const FREE: *mut () = 0 as *mut ();
const RESERVED: *mut () = 1 as *mut ();

////////////////////////////////////////////////////////////////////////////////////////////////////
/// Protected
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Protected(NonNull<()>);

impl Protected {
    #[inline]
    pub fn address(&self) -> usize {
        self.0.as_ptr() as usize
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
/// HazardPair
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct HazardPair {
    crate protected: CachePadded<AtomicPtr<()>>,
    crate next: CachePadded<AtomicPtr<HazardPair>>,
}

impl HazardPair {
    #[inline]
    fn new(protected: NonNull<()>) -> Self {
        Self {
            protected: CachePadded::new(AtomicPtr::new(protected.as_ptr())),
            next: CachePadded::new(AtomicPtr::default()),
        }
    }

    #[inline]
    pub fn set_free(&self, order: Ordering) {
        self.protected.store(FREE, order);
    }

    #[inline]
    pub fn set_reserved(&self, order: Ordering) {
        self.protected.store(RESERVED, order);
    }

    #[inline]
    pub fn set_protected(&self, ptr: NonNull<()>, order: Ordering) {
        self.protected.store(ptr.as_ptr(), order);
    }

    #[inline]
    pub fn protected(&self, order: Ordering) -> Option<Protected> {
        match self.protected.load(order) {
            FREE | RESERVED => None,
            ptr => Some(Protected(unsafe { NonNull::new_unchecked(ptr) }))
        }
    }
}

pub struct Hazard(&'static HazardPair);

impl Hazard {
    #[inline]
    pub fn hazard_pair(&self) -> &HazardPair {
        &self.0
    }

    #[inline]
    pub fn into_inner(self) -> &'static HazardPair {
        let hazard = self.0;
        mem::forget(self);
        hazard
    }
}

impl From<&'static HazardPair> for Hazard {
    #[inline]
    fn from(pair: &'static HazardPair) -> Self {
        Self(pair)
    }
}

impl Drop for Hazard {
    #[inline]
    fn drop(&mut self) {
        if let Err(hazard) = local::try_recycle_hazard(self.0) {
            hazard.set_free(Ordering::Release);
        }
    }
}
