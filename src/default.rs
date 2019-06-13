use std::ptr::NonNull;

use reclaim::typenum::Unsigned;
use reclaim::{LocalReclaim, Protect, Reclaim};

use crate::hazard::Hazard;
use crate::local::{Local, LocalAccess, RecycleError};
use crate::{Guarded, Unlinked, HP};

// Per-thread instances of `Local`
thread_local!(static LOCAL: Local = Local::new());

/// Creates a new (empty) guarded pointer that can be used to acquire hazard pointers.
#[inline]
pub fn guarded<T, N: Unsigned>() -> impl Protect<Item = T, MarkBits = N, Reclaimer = HP> {
    Guarded::with_access(DefaultAccess)
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

////////////////////////////////////////////////////////////////////////////////////////////////////
// DefaultAccess
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default)]
pub struct DefaultAccess;

impl LocalAccess for DefaultAccess {
    #[inline]
    fn get_hazard(self, protect: NonNull<()>) -> &'static Hazard {
        LOCAL.with(|local| local.get_hazard(protect))
    }

    #[inline]
    fn try_recycle_hazard(self, hazard: &'static Hazard) -> Result<(), RecycleError> {
        LOCAL
            .try_with(|local| local.try_recycle_hazard(hazard))
            .unwrap_or(Err(RecycleError::Access))
    }

    #[inline]
    fn increase_ops_count(self) {
        LOCAL.with(|local| local.increase_ops_count());
    }
}

impl<T, N: Unsigned> Guarded<T, N> {
    #[inline]
    pub fn new() -> Self {
        Self::with_access(DefaultAccess)
    }
}

impl<T, N: Unsigned> Default for Guarded<T, N> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
