pub(crate) mod global_retire;
pub(crate) mod local_retire;

use self::global_retire::RetiredQueue;
use self::local_retire::{AbandonedQueue, RetireNode};

////////////////////////////////////////////////////////////////////////////////////////////////////
// RetireStrategy (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A trait for selecting the strategy for retiring records.
pub trait RetireStrategy: Sized + 'static {}

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalRetire
////////////////////////////////////////////////////////////////////////////////////////////////////

/// The global retire strategy.
///
/// Using this strategy, whenever a record is retired it is stored in a
/// synchronized global queue that is used by all threads.
///
/// # Advantage
///
/// The global strategy allows any thread to reclaim records which were retired
/// by other threads.
/// This can be beneficial when there are imbalances between retirement and
/// reclamation.
/// For instance, only one thread may be responsible for retiring records but
/// may not be able to actually reclaim any memory due to hazard pointers held
/// by other threads.
/// By using the global retire strategy, these other threads can help with the
/// reclamation aspect.
///
/// # Disadvantage
///
/// Since the retirement of memory records requires synchronized access to a
/// global queue, this process is quite expensive.
/// Hence, it should preferably be used when memory records only infrequently
/// retired or when the outlined advantage clearly outweighs the higher cost
/// for accessing the global queue.
#[derive(Copy, Clone, Debug, Default, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct GlobalRetire;

/********** impl RetireStrategy *******************************************************************/

impl RetireStrategy for GlobalRetire {}

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalRetireState
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub(crate) enum GlobalRetireState {
    /// The [`GlobalStrategy`] requires a global queue for **all** retired
    /// records.
    GlobalStrategy(RetiredQueue),
    /// The [`LocalStrategy`] requires a global queue **only** for abandoned
    /// records, i.e., retired records which are stored globally when a thread
    /// exits.
    LocalStrategy(AbandonedQueue),
}

/********** impl inherent *************************************************************************/

impl GlobalRetireState {
    #[inline]
    pub(crate) const fn global_strategy() -> Self {
        GlobalRetireState::GlobalStrategy(RetiredQueue::new())
    }

    #[inline]
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

/// The thread-local state required by the selected retire strategy.
#[derive(Debug)]
pub(crate) enum LocalRetireState<'global> {
    /// The local state used by the global retire strategy.
    GlobalStrategy(&'global RetiredQueue),
    /// The local state used by the local retire strategy.
    LocalStrategy(Box<RetireNode>, &'global AbandonedQueue),
}

/********** impl From *****************************************************************************/

impl<'global> From<&'global GlobalRetireState> for LocalRetireState<'global> {
    #[inline]
    fn from(retire_state: &'global GlobalRetireState) -> Self {
        match retire_state {
            GlobalRetireState::GlobalStrategy(queue) => LocalRetireState::GlobalStrategy(queue),
            GlobalRetireState::LocalStrategy(abandoned) => {
                // check if there are any abandoned records that can be used by the new thread
                // instead of allocating a new local queue
                match abandoned.take_all_and_merge() {
                    Some(node) => LocalRetireState::LocalStrategy(node, abandoned),
                    None => {
                        LocalRetireState::LocalStrategy(Box::new(Default::default()), abandoned)
                    }
                }
            }
        }
    }
}
