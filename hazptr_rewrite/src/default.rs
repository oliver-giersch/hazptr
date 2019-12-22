use std::rc::Rc;
use std::sync::RwLock;

use conquer_once::Lazy;
use conquer_reclaim::{GlobalReclaim, Reclaim, ReclaimerLocalRef, Retired};

use crate::config::Config;
use crate::global::GlobalHandle;
use crate::guard::Guard;
use crate::local::LocalHandle;
use crate::policy::{LocalRetire, Policy};

type Local = crate::local::Local<'static, LocalRetire>;
type Hp = crate::Hp<LocalRetire>;

/********** global & thread-local *****************************************************************/

/// Global hazard pointer configuration.
pub static CONFIG: Lazy<RwLock<Config>> = Lazy::new(Default::default);
static HP: Lazy<Hp> = Lazy::new(Default::default);

thread_local!(static LOCAL: Rc<Local> = {
    let config = *CONFIG.read().unwrap();
    Rc::new(Local::new(config, GlobalHandle::from_ref(&HP.state)))
});

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalHP
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Default)]
pub struct GlobalHp;

/********** impl GlobalReclaimer ******************************************************************/

impl GlobalReclaim for GlobalHp {
    #[inline]
    fn build_local_ref() -> Self::Ref {
        DefaultHandle
    }
}

/********** impl Reclaimer ************************************************************************/

unsafe impl Reclaim for GlobalHp {
    type Header = <LocalRetire as Policy>::Header;
    type Ref = DefaultHandle;

    #[inline]
    fn new() -> Self {
        Self::default()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// DefaultHandle
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default)]
pub struct DefaultHandle;

/********** impl ReclaimerHandle ******************************************************************/

unsafe impl ReclaimerLocalRef for DefaultHandle {
    type Guard = Guard<'static, 'static, LocalRetire, Self::Reclaimer>;
    type Reclaimer = GlobalHp;

    #[inline]
    fn from_ref(_: &Self::Reclaimer) -> Self {
        Self
    }

    #[inline]
    unsafe fn from_raw(global: *const Self::Reclaimer) -> Self {
        Self
    }

    #[inline]
    fn into_guard(self) -> Self::Guard {
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
