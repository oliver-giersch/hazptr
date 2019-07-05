use std::ptr::NonNull;

use reclaim::typenum::Unsigned;
use reclaim::{GlobalReclaim, Reclaim};

use crate::hazard::Hazard;
use crate::local::{Local, LocalAccess, RecycleError};
use crate::{Unlinked, HP};

pub type Guard = crate::guard::Guard<DefaultAccess>;

// Per-thread instances of `Local`
thread_local!(static LOCAL: Local = Local::new());

unsafe impl GlobalReclaim for HP {
    type Guard = Guard;

    #[inline]
    fn try_flush() {
        LOCAL.with(Local::try_flush);
    }
    
    #[inline]
    unsafe fn retire<T: 'static, N: Unsigned>(unlinked: Unlinked<T, N>) {
        LOCAL.with(move |local| Self::retire_local(local, unlinked))
    }

    #[inline]
    unsafe fn retire_unchecked<T, N: Unsigned>(unlinked: Unlinked<T, N>) {
        LOCAL.with(move |local| Self::retire_local_unchecked(local, unlinked))
    }
}

impl Guard {
    #[inline]
    pub fn new() -> Self {
        Self::with_access(DefaultAccess)
    }
}

impl Default for Guard {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// DefaultAccess
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default)]
pub struct DefaultAccess;

impl LocalAccess for DefaultAccess {
    #[inline]
    fn get_hazard(self, protect: Option<NonNull<()>>) -> &'static Hazard {
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
