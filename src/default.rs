use std::rc::Rc;
use std::sync::RwLock;

use conquer_once::Lazy;
use conquer_reclaim::{GlobalReclaim, LocalState, Reclaim, Retired};

use crate::config::Config;
use crate::global::GlobalRef;
use crate::local::LocalRef;
use crate::Hp;

type Guard = crate::guard::Guard<'static, 'static, GlobalHp>;
type Local = crate::local::Local<'static, GlobalHp>;

/********** globals & thread-locals ***************************************************************/

/// The global hazard pointer configuration.
pub static CONFIG: Lazy<RwLock<Config>> = Lazy::new(RwLock::default);

/// The global hazard pointer state.
static HP: Hp = Hp::local_retire(Config::with_defaults());

thread_local!(static LOCAL: Rc<Local> = {
    let config = *CONFIG.read().unwrap();
    Rc::new(Local::new(config, GlobalRef::from_ref(&HP.state)))
});

/********** public functions **********************************************************************/

#[inline]
pub fn build_guard() -> <GlobalHpRef as LocalState>::Guard {
    GlobalHpRef.build_guard()
}

#[inline]
pub unsafe fn retire_record(record: Retired<GlobalHp>) {
    GlobalHpRef.retire_record(record);
}

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
    type Guard = Guard;
    type Reclaimer = GlobalHp;

    #[inline]
    fn build_guard(&self) -> Self::Guard {
        LOCAL.with(|local| Guard::with_handle(LocalRef::from_owned(Rc::clone(local))))
    }

    #[inline]
    unsafe fn retire_record(&self, retired: Retired<Self::Reclaimer>) {
        LOCAL.with(move |local| local.retire_record(retired.into_raw()))
    }
}

/********** (extra) impl for Guard ****************************************************************/

impl Guard {
    /// Creates a new guard.
    #[inline]
    pub fn new() -> Self {
        GlobalHpRef.build_guard()
    }
}

impl Default for Guard {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
