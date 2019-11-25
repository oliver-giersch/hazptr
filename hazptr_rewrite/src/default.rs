use std::rc::Rc;

use conquer_once::Lazy;
use conquer_reclaim::{GlobalReclaimer, Reclaimer, ReclaimerHandle, Retired};

use crate::global::GlobalHandle;
use crate::guard::Guard;
use crate::local::LocalHandle;
use crate::policy::{LocalRetire, Policy};

type Local = crate::local::Local<'static, LocalRetire>;
type Global = crate::global::Global<LocalRetire>;

/********** global & thread-local *****************************************************************/

static GLOBAL: Lazy<Global> = Lazy::new(Global::default);
thread_local!(static LOCAL: Rc<Local<LocalRetire>> = Local::new(GlobalHandle::from_ref(&*GLOBAL)));

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalHP
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct GlobalHP;

/********** impl GlobalReclaimer ******************************************************************/

unsafe impl GlobalReclaimer for GlobalHP {
    #[inline]
    fn guard() -> <Self::Handle as ReclaimerHandle>::Guard {
        let handle = GlobalHandle;
        handle.guard()
    }

    #[inline]
    unsafe fn retire(record: Retired<Self>) {
        let handle = GlobalHandle;
        handle.retire(record);
    }
}

/********** impl Reclaimer ************************************************************************/

unsafe impl Reclaimer for GlobalHP {
    type Global = Global;
    type Header = LocalRetire::Header;
    type Handle = GlobalHandle;

    #[inline]
    fn create_local_handle(&self) -> Self::Handle {
        GlobalHandle
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default)]
pub struct GlobalHandle;

/********** impl ReclaimerHandle ******************************************************************/

impl ReclaimerHandle for GlobalHandle {
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
