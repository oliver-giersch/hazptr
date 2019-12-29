pub(crate) mod global_retire;
pub(crate) mod local_retire;

use self::global_retire::RetiredQueue;
use self::local_retire::{AbandonedQueue, RetireNode};

////////////////////////////////////////////////////////////////////////////////////////////////////
// RetireStrategy (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

pub trait RetireStrategy: Sized + 'static {}

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalRetire
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct GlobalRetire;

/********** impl RetireStrategy *******************************************************************/

impl RetireStrategy for GlobalRetire {}

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalRetireState
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub(crate) enum GlobalRetireState {
    GlobalStrategy(RetiredQueue),
    LocalStrategy(AbandonedQueue),
}

/********** impl inherent *************************************************************************/

impl GlobalRetireState {
    pub(crate) const fn global_strategy() -> Self {
        GlobalRetireState::GlobalStrategy(RetiredQueue::new())
    }

    pub(crate) const fn local_strategy() -> Self {
        GlobalRetireState::LocalStrategy(AbandonedQueue::new())
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalRetire
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct LocalRetire;

/********** impl RetireStrategy *******************************************************************/

impl RetireStrategy for LocalRetire {}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalRetireState
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub(crate) enum LocalRetireState {
    GlobalStrategy,
    LocalStrategy(Box<RetireNode>),
}

/********** impl From *****************************************************************************/

impl From<&GlobalRetireState> for LocalRetireState {
    #[inline]
    fn from(retire_state: &GlobalRetireState) -> Self {
        match retire_state {
            GlobalRetireState::GlobalStrategy(_) => LocalRetireState::GlobalStrategy,
            GlobalRetireState::LocalStrategy(abandoned) => {
                // check if there are any abandoned records that can be used by
                // the new thread instead of allocating a new local queue
                match abandoned.take_all_and_merge() {
                    Some(node) => LocalRetireState::LocalStrategy(node),
                    None => LocalRetireState::LocalStrategy(Box::new(Default::default())),
                }
            }
        }
    }
}
