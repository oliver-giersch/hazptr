use std::rc::Rc;
use std::sync::RwLock;

use conquer_once::Lazy;
use conquer_reclaim::{GlobalReclaim, LocalState, Reclaim, Retired};

use crate::config::Config;
use crate::global::GlobalRef;
use crate::guard::Guard;
use crate::local::LocalHandle;
use crate::strategy::LocalRetire;

type Local = crate::local::Local<'static>;
type Hp = crate::Hp<LocalRetire>;

/********** globals & thread-locals ***************************************************************/

/// The global hazard pointer configuration.
pub static CONFIG: Lazy<RwLock<Config>> = Lazy::new(RwLock::default);

/// The global hazard pointer state.
static HP: Lazy<Hp> = Lazy::new(Hp::default);

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

unsafe impl GlobalReclaim for GlobalHp {
    fn build_local_state() -> Self::LocalState {
        GlobalHpRef
    }
}

/********** impl Reclaimer ************************************************************************/

impl Reclaim for GlobalHp {
    type Header = ();
    type LocalState = GlobalHpRef;

    #[inline]
    unsafe fn build_local_state(&self) -> Self::LocalState {
        GlobalHpRef
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalHpRef
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default)]
pub struct GlobalHpRef;

/********** impl LocalState ***********************************************************************/

unsafe impl LocalState for GlobalHpRef {
    type Guard = Guard<'static, 'static, Self::Reclaimer>;
    type Reclaimer = GlobalHp;

    #[inline]
    fn build_guard(&self) -> Self::Guard {
        LOCAL.with(|local| Guard::with_handle(LocalHandle::from_owned(Rc::clone(local))))
    }

    #[inline]
    unsafe fn retire_record(&self, retired: Retired<Self::Reclaimer>) {
        LOCAL.with(move |local| local.retire_record(retired.into_raw()))
    }
}
