use std::rc::Rc;
use std::sync::RwLock;

use conquer_once::Lazy;
use conquer_reclaim::{BuildReclaimRef, GlobalReclaim, Reclaim, ReclaimRef, Retired};

use crate::config::Config;
use crate::global::GlobalRef;
use crate::guard::Guard;
use crate::local::LocalHandle;
use crate::retire::LocalRetire;

type Local = crate::local::Local<'static>;
type Hp = crate::Hp<LocalRetire>;

/********** globals & thread-locals ***************************************************************/

/// The global hazard pointer configuration.
pub static CONFIG: Lazy<RwLock<Config>> = Lazy::new(Default::default);

/// The global hazard pointer state.
static HP: Lazy<Hp> = Lazy::new(Default::default);

thread_local!(static LOCAL: Rc<Local> = {
    let config = *CONFIG.read().unwrap();
    Rc::new(Local::new(config, GlobalRef::from_ref(&HP.state)))
});

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalHP
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A handle to the global hazard pointer state.
#[derive(Debug, Default)]
pub struct GlobalHp;

/********** impl GlobalReclaimer ******************************************************************/

impl GlobalReclaim for GlobalHp {
    #[inline]
    fn build_global_ref() -> Self::Ref {
        GlobalHpRef
    }
}

/********** impl Reclaimer ************************************************************************/

unsafe impl Reclaim for GlobalHp {
    type Header = <Hp as Reclaim>::Header;
    type Ref = GlobalHpRef;

    #[inline]
    fn new() -> Self {
        Self::default()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalHpRef
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default)]
pub struct GlobalHpRef;

/********** impl BuildReclaimRef ******************************************************************/

impl<'a> BuildReclaimRef<'a> for GlobalHpRef {
    #[inline]
    fn from_ref(_: &'a Self::Reclaimer) -> Self {
        Self
    }
}

/********** impl ReclaimRef ***********************************************************************/

unsafe impl ReclaimRef for GlobalHpRef {
    type Guard = Guard<'static, 'static, Self::Reclaimer>;
    type Reclaimer = GlobalHp;

    #[inline]
    unsafe fn from_raw(_: &Self::Reclaimer) -> Self {
        Self
    }

    #[inline]
    fn into_guard(self) -> Self::Guard {
        LOCAL.with(|local| Guard::with_handle(LocalHandle::from_owned(Rc::clone(local))))
    }

    #[inline]
    unsafe fn retire(self, record: Retired<Self::Reclaimer>) {
        LOCAL.with(move |local| {
            local.retire(record.into_raw());
        });
    }
}
