use core::mem::ManuallyDrop;
use core::sync::atomic::Ordering;

use arrayvec::{ArrayVec, CapacityError};
use conquer_reclaim::RetiredPtr;

use crate::config::{Config, CountStrategy};
use crate::global::GlobalRef;
use crate::hazard::{HazardPtr, ProtectStrategy, ProtectedPtr};
use crate::strategy::LocalRetireState;

////////////////////////////////////////////////////////////////////////////////////////////////////
// RecycleError
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Error type for thread local recycle operations.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct RecycleError;

/********** impl From *****************************************************************************/

impl From<CapacityError<&'_ HazardPtr>> for RecycleError {
    #[inline]
    fn from(_: CapacityError<&HazardPtr>) -> Self {
        RecycleError
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalInner
////////////////////////////////////////////////////////////////////////////////////////////////////

const HAZARD_CACHE: usize = 16;

/// The thread-local state for using and managing hazard pointers.

pub(super) struct LocalInner<'global> {
    /// The configuration used by the thread.
    config: Config,
    /// A reference to the global state containing, e.g., all hazard pointers.
    global: GlobalRef<'global>,
    /// The local retire state depending on the employed retire strategy.
    state: ManuallyDrop<LocalRetireState<'global>>,
    /// The current count of relevant operations counting towards the reclaim
    /// threshold (which ops are counted depends on the configuration).
    ops_count: u32,
    /// The bounded local cache of previously acquired hazard pointers.
    hazard_cache: ArrayVec<[&'global HazardPtr; HAZARD_CACHE]>,
    /// The cache for storing a list of all protected pointers during
    /// reclamation attempts (may re-allocate at runtime).
    scan_cache: Vec<ProtectedPtr>,
}

/********** impl inherent *************************************************************************/

impl<'global> LocalInner<'global> {
    /// Creates a new `LocalInner` from a `config` and a `global`.
    #[inline]
    pub fn new(config: Config, global: GlobalRef<'global>) -> Self {
        let state = ManuallyDrop::new(LocalRetireState::from(&global.as_ref().retire_state));
        Self {
            config,
            global,
            state,
            ops_count: Default::default(),
            hazard_cache: Default::default(),
            scan_cache: Default::default(),
        }
    }

    /// Increases the ops count if the `CountStrategy` is to count on release.
    #[inline(always)]
    pub fn increase_ops_count_if_count_release(&mut self) {
        if let CountStrategy::Release = self.config.count_strategy {
            self.increase_ops_count();
        }
    }

    /// Acquires a hazard pointer, either from the local cache or, if this is
    /// empty, from the global state.
    ///
    /// Depending on the `strategy` argument, the acquired hazard pointer is
    /// either immediately set to protect some pointer or is only marked as
    /// reserved.
    #[inline]
    pub fn get_hazard(&mut self, strategy: ProtectStrategy) -> &HazardPtr {
        // check the local hazard cache for fast-path acquisition
        match self.hazard_cache.pop() {
            Some(hazard) => {
                if let ProtectStrategy::Protect(protected) = strategy {
                    hazard.set_protected(protected.into_inner(), Ordering::SeqCst);
                }

                hazard
            }
            // ...otherwise acquire a hazard pointer globally
            None => self.global.as_ref().get_hazard(strategy),
        }
    }

    /// Attempts to recycle `hazard` in the local hazard pointer cache.
    ///
    /// # Errors
    ///
    /// Fails if the local cache is full.
    #[inline]
    pub fn try_recycle_hazard(&mut self, hazard: &'global HazardPtr) -> Result<(), RecycleError> {
        self.hazard_cache.try_push(hazard)?;
        hazard.set_thread_reserved(Ordering::Release);

        Ok(())
    }

    /// Retires the given `retired` according to the defined retire strategy.
    ///
    /// # Safety
    ///
    /// The usual invariants for record retirement apply.
    /// Additionally, `retired` must be derived from a `Retired<Hp<_>>` for the
    /// correct retire strategy.
    #[inline]
    pub unsafe fn retire_record(&mut self, retired: RetiredPtr) {
        // retire the record according to the specified retire strategy
        self.retire_inner(retired);

        // if the chosen config specifies retire operations to be counted, increase the ops count
        if let CountStrategy::Retire = self.config.count_strategy {
            self.increase_ops_count();
        }
    }

    /// Increases the ops count and initiates a reclamation attempt if the
    /// threshold is passed.
    #[inline]
    fn increase_ops_count(&mut self) {
        self.ops_count += 1;

        if self.ops_count == self.config.ops_count_threshold {
            self.ops_count = 0;
            self.try_reclaim();
        }
    }

    #[cold]
    fn try_reclaim(&mut self) {
        // return early if no records have been retired
        if !self.has_retired_records() {
            return;
        }

        // globally collect all protected pointers into scan cache (this issues a full memory fence)
        self.global.as_ref().collect_protected_hazards(&mut self.scan_cache, Ordering::SeqCst);
        // reclaim all retired records that are not protected
        unsafe { self.reclaim_all_unprotected() };
    }

    #[inline]
    fn has_retired_records(&self) -> bool {
        match &*self.state {
            LocalRetireState::GlobalStrategy(queue) => !queue.is_empty(),
            LocalRetireState::LocalStrategy(node, _) => !node.is_empty(),
        }
    }

    #[inline]
    unsafe fn retire_inner(&mut self, retired: RetiredPtr) {
        match &mut *self.state {
            LocalRetireState::GlobalStrategy(ref queue) => queue.retire_record(retired),
            LocalRetireState::LocalStrategy(node, _) => node.retire_record(retired),
        }
    }

    #[inline]
    unsafe fn reclaim_all_unprotected(&mut self) {
        // scan cache must be sorted for binary search
        self.scan_cache.sort_unstable();
        match &mut *self.state {
            LocalRetireState::GlobalStrategy(ref queue) => {
                queue.reclaim_all_unprotected(&self.scan_cache)
            }
            LocalRetireState::LocalStrategy(local, ref queue) => {
                // check if there are any abandoned records and adopt them into the local cache.
                if let Some(node) = queue.take_all_and_merge() {
                    local.merge(node.into_inner())
                }

                local.reclaim_all_unprotected(&self.scan_cache)
            }
        };
    }
}

/********** impl Drop *****************************************************************************/

impl Drop for LocalInner<'_> {
    #[inline(never)]
    fn drop(&mut self) {
        // set all thread-reserved hazard pointers free again
        for hazard in self.hazard_cache.iter() {
            hazard.set_free(Ordering::Relaxed);
        }

        // execute a final reclamation attempt
        self.try_reclaim();

        let state = unsafe { ManuallyDrop::take(&mut self.state) };
        // if a local retire strategy is used, any remaining retired records must be made
        // reclaimable by other threads and are pushed to a global queue.
        if let LocalRetireState::LocalStrategy(node, queue) = state {
            // if there are no remaining records the node can be de-allocated right away
            if node.is_empty() {
                return;
            }

            // ... otherwise, it is pushed to the global queue of abandoned retired records
            queue.push(node);
        }
    }
}
