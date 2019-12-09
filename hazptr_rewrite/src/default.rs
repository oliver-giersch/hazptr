use std::rc::Rc;

use conquer_once::Lazy;
use conquer_reclaim::{GenericReclaimer, GlobalReclaimer, Reclaimer, ReclaimerHandle, Retired};

use crate::global::GlobalHandle;
use crate::guard::Guard;
use crate::local::LocalHandle;
use crate::policy::{LocalRetire, Policy};

type Local = crate::local::Local<'static, LocalRetire>;
type Global = crate::global::Global<LocalRetire>;

/********** global & thread-local *****************************************************************/

static GLOBAL: Lazy<Global> = Lazy::new(Global::default);
thread_local!(static LOCAL: Rc<Local> = Local::new(GlobalHandle::from_ref(&*GLOBAL)));

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalHP
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct GlobalHP;

/********** impl GlobalReclaimer ******************************************************************/

unsafe impl GlobalReclaimer for GlobalHP {
    #[inline]
    fn guard() -> <Self::Handle as ReclaimerHandle>::Guard {
        GlobalDefaultHandle::guard(GlobalDefaultHandle)
    }

    #[inline]
    unsafe fn retire(record: Retired<Self>) {
        GlobalDefaultHandle::retire(GlobalDefaultHandle, record);
    }
}

/********** impl GenericReclaimer *****************************************************************/

unsafe impl GenericReclaimer for GlobalHP {
    type Handle = GlobalDefaultHandle;

    #[inline]
    fn create_local_handle(&self) -> Self::Handle {
        GlobalDefaultHandle
    }
}

/********** impl Reclaimer ************************************************************************/

unsafe impl Reclaimer for GlobalHP {
    type Global = Global;
    type Header = <LocalRetire as Policy>::Header;
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default)]
pub struct GlobalDefaultHandle;

/********** impl ReclaimerHandle ******************************************************************/

unsafe impl ReclaimerHandle for GlobalDefaultHandle {
    type Reclaimer = GlobalHP;
    type Guard = Guard<'static, 'static, LocalRetire>;

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
