use std::rc::Rc;

use conquer_once::Lazy;
use conquer_reclaim::{GlobalReclaimer, OwningReclaimer, Reclaimer, ReclaimerHandle, Retired};

use crate::global::GlobalHandle;
use crate::guard::Guard;
use crate::local::LocalHandle;
use crate::policy::{LocalRetire, Policy};

type Local = crate::local::Local<'static, LocalRetire>;
type Global = crate::global::Global<LocalRetire>;

/********** global & thread-local *****************************************************************/

static GLOBAL: Lazy<Global> = Lazy::new(Global::default);
thread_local!(static LOCAL: Rc<Local> = Rc::new(Local::new(GlobalHandle::from_ref(&*GLOBAL))));

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalHP
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Default)]
pub struct GlobalHp;

/********** impl GlobalReclaimer ******************************************************************/

unsafe impl GlobalReclaimer for GlobalHp {
    #[inline]
    fn handle() -> Self::Handle {
        GlobalDefaultHandle
    }

    #[inline]
    fn guard() -> <Self::Handle as ReclaimerHandle>::Guard {
        GlobalDefaultHandle::guard(GlobalDefaultHandle)
    }

    #[inline]
    unsafe fn retire(record: Retired<Self>) {
        GlobalDefaultHandle::retire(GlobalDefaultHandle, record);
    }
}

/********** impl OwningReclaimer ******************************************************************/

unsafe impl OwningReclaimer for GlobalHp {
    type Handle = GlobalDefaultHandle;

    #[inline]
    fn owning_local_handle(&self) -> Self::Handle {
        GlobalDefaultHandle
    }
}

/********** impl Reclaimer ************************************************************************/

unsafe impl Reclaimer for GlobalHp {
    type Global = Global;
    type Header = <LocalRetire as Policy>::Header;

    #[inline]
    fn new() -> Self {
        Self::default()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default)]
pub struct GlobalDefaultHandle;

/********** impl ReclaimerHandle ******************************************************************/

unsafe impl ReclaimerHandle for GlobalDefaultHandle {
    type Reclaimer = GlobalHp;
    type Guard = Guard<'static, 'static, LocalRetire, GlobalHp>;

    #[inline]
    fn guard(self) -> Self::Guard {
        LOCAL.with(|local| Guard::with_handle(LocalHandle::from_owned(Rc::clone(local))))
    }

    #[inline]
    unsafe fn retire(self, record: Retired<Self::Reclaimer>) {
        LOCAL.with(move |local| {
            let handle = LocalHandle::from_ref(local);
            handle.retire(record);
        });
    }
}
