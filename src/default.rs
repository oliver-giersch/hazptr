use std::ptr::NonNull;

use reclaim::typenum::Unsigned;
use reclaim::{LocalReclaim, Protect, Reclaim};

use crate::global::Global;
use crate::hazard::{Hazard, HazardPtr};
use crate::local::{Local, LocalAccess, RecycleErr};
use crate::{Guarded, Unlinked, HP};

/// The single static `Global` instance
static GLOBAL: Global = Global::new();

// Per-thread instances of `Local`
thread_local!(static LOCAL: Local = Local::new(&GLOBAL));

/// Creates a new (empty) guarded pointer that can be used to acquire hazard pointers.
#[inline]
pub fn guarded<T, N: Unsigned>() -> impl Protect<Item = T, MarkBits = N, Reclaimer = HP> {
    Guarded::new(DefaultAccess)
}

unsafe impl Reclaim for HP {
    #[inline]
    unsafe fn retire<T: 'static, N: Unsigned>(unlinked: Unlinked<T, N>) {
        LOCAL.with(move |local| Self::retire_local(local, unlinked))
    }

    #[inline]
    unsafe fn retire_unchecked<T, N: Unsigned>(unlinked: Unlinked<T, N>) {
        LOCAL.with(move |local| Self::retire_local_unchecked(local, unlinked))
    }
}

#[derive(Copy, Clone, Debug)]
pub struct DefaultAccess;

impl LocalAccess for DefaultAccess {
    #[inline]
    fn wrap_hazard(self, protect: NonNull<()>) -> HazardPtr<Self> {
        LOCAL.with(|local| HazardPtr::new(Local::get_hazard(local, protect), DefaultAccess))
    }

    #[inline]
    fn try_recycle_hazard(self, hazard: &'static Hazard) -> Result<(), RecycleErr> {
        LOCAL
            .try_with(|local| Local::try_recycle_hazard(local, hazard))
            .or(Err(RecycleErr::Access))
            .and(Ok(()))
    }

    #[inline]
    fn increase_ops_count(self) {
        LOCAL.with(Local::increase_ops_count);
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Testing related free functions
////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
#[inline]
pub(crate) fn global() -> &'static Global {
    &GLOBAL
}

#[cfg(test)]
#[inline]
pub(crate) fn cached_hazards_count() -> usize {
    LOCAL.with(Local::cached_hazards_count)
}
