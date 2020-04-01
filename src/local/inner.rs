use core::mem::ManuallyDrop;
use core::ptr;
use core::sync::atomic::Ordering;

use arrayvec::{ArrayVec, CapacityError};
use conquer_reclaim::RetiredPtr;

use crate::config::{Config, Operation};
use crate::global::GlobalRef;
use crate::hazard::{HazardPtr, ProtectStrategy, ProtectedPtr};
use crate::strategy::{GlobalRetireState, LocalRetireState};

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

#[derive(Debug)]
pub(super) struct LocalInner<'global> {
    config: Config,
    global: GlobalRef<'global>,
    state: ManuallyDrop<LocalRetireState>,
    ops_count: u32,
    hazard_cache: ArrayVec<[&'global HazardPtr; HAZARD_CACHE]>,
    scan_cache: Vec<ProtectedPtr>,
}

/********** impl inherent *************************************************************************/

impl<'global> LocalInner<'global> {
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

    #[inline]
    pub fn try_increase_ops_count(&mut self, op: Operation) {
        if op == self.config.count_strategy {
            self.ops_count += 1;

            if self.ops_count == self.config.ops_count_threshold {
                self.ops_count = 0;
                self.try_reclaim();
            }
        }
    }

    #[inline]
    pub fn get_hazard(&mut self, strategy: ProtectStrategy) -> &HazardPtr {
        match self.hazard_cache.pop() {
            Some(hazard) => {
                if let ProtectStrategy::Protect(protected) = strategy {
                    hazard.set_protected(protected.into_inner(), Ordering::SeqCst);
                }

                hazard
            }
            None => self.global.as_ref().get_hazard(strategy),
        }
    }

    #[inline]
    pub fn try_recycle_hazard(&mut self, hazard: &'global HazardPtr) -> Result<(), RecycleError> {
        // todo: use small vec, incorporate config?
        self.hazard_cache.try_push(hazard)?;
        hazard.set_thread_reserved(Ordering::Release);

        Ok(())
    }

    #[inline]
    pub unsafe fn retire_record(&mut self, retired: RetiredPtr) {
        unsafe { self.retire_inner(retired.into_raw()) };

        if self.config.is_count_retire() {
            self.ops_count += 1;
        }
    }

    #[inline]
    fn try_reclaim(&mut self) {
        if !self.has_retired_records() {
            return;
        }

        // collect into scan_cache
        self.global.as_ref().collect_protected_hazards(&mut self.scan_cache, Ordering::SeqCst);

        unsafe { self.reclaim_all_unprotected() };
    }

    #[inline]
    fn has_retired_records(&self) -> bool {
        match &*self.state {
            LocalRetireState::GlobalStrategy => match &self.global.as_ref().retire_state {
                GlobalRetireState::GlobalStrategy(queue) => !queue.is_empty(),
                _ => unreachable!(),
            },
            LocalRetireState::LocalStrategy(node) => !node.is_empty(),
        }
    }

    #[inline]
    unsafe fn retire_inner(&mut self, retired: RetiredPtr) {
        match &mut *self.state {
            LocalRetireState::GlobalStrategy => match &self.global.as_ref().retire_state {
                GlobalRetireState::GlobalStrategy(queue) => queue.retire(retired),
                _ => unreachable!(),
            },
            LocalRetireState::LocalStrategy(node) => node.retire(retired),
        }
    }

    #[inline]
    unsafe fn reclaim_all_unprotected(&mut self) {
        match &mut *self.state {
            LocalRetireState::GlobalStrategy => match &self.global.as_ref().retire_state {
                GlobalRetireState::GlobalStrategy(queue) => {
                    queue.reclaim_all_unprotected(&self.scan_cache)
                }
                _ => unreachable!(),
            },
            LocalRetireState::LocalStrategy(local) => match &self.global.as_ref().retire_state {
                GlobalRetireState::LocalStrategy(queue) => {
                    if let Some(node) = queue.take_all_and_merge() {
                        local.merge(node.into_inner())
                    }

                    self.scan_cache.sort_unstable();
                    local.reclaim_all_unprotected(&self.scan_cache)
                }
                _ => unreachable!(),
            },
        }
    }
}

/********** impl Drop *****************************************************************************/

impl Drop for LocalInner<'_> {
    #[inline(never)]
    fn drop(&mut self) {
        // set all thread-reserved hazard pointers free
        for hazard in self.hazard_cache.iter() {
            hazard.set_free(Ordering::Relaxed);
        }

        // execute a final reclamation attempt
        self.try_reclaim();

        // with the local retire strategy, any remaining retired records must
        // be abandoned, i.e. stored globally so that other threads can adopt
        // and eventually reclaim them
        let state = unsafe { ptr::read(&*self.state) };
        if let LocalRetireState::LocalStrategy(node) = state {
            // if there are no remaining records the node can be de-allocated right away
            if node.is_empty() {
                return;
            }

            // otherwise, it must be pushed to the global queue of retired records
            match &self.global.as_ref().retire_state {
                GlobalRetireState::LocalStrategy(queue) => queue.push(node),
                _ => unreachable!(),
            }
        }
    }
}
