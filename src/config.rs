use crate::strategy::local_retire::RetireNode;

const DEFAULT_SCAN_CACHE_SIZE: usize = 128;
const DEFAULT_RETIRE_CACHE_SIZE: usize = RetireNode::DEFAULT_INITIAL_CAPACITY;
const DEFAULT_OPS_COUNT_THRESHOLD: u32 = 128;
const DEFAULT_COUNT_STRATEGY: CountStrategy = CountStrategy::Retire;

////////////////////////////////////////////////////////////////////////////////////////////////////
// ConfigBuilder
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct ConfigBuilder {
    initial_scan_cache_size: Option<usize>,
    initial_retire_cache_size: Option<usize>,
    ops_count_threshold: Option<u32>,
    count_strategy: Option<CountStrategy>,
}

/********** impl inherent *************************************************************************/

impl ConfigBuilder {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn set_initial_scan_cache_size(mut self, val: usize) -> Self {
        self.initial_scan_cache_size = Some(val);
        self
    }

    #[inline]
    pub fn set_initial_retire_cache_size(mut self, val: usize) -> Self {
        self.initial_retire_cache_size = Some(val);
        self
    }

    #[inline]
    pub fn set_ops_count_threshold(mut self, val: u32) -> Self {
        self.ops_count_threshold = Some(val);
        self
    }

    pub fn set_count_strategy(mut self, val: CountStrategy) -> Self {
        self.count_strategy = Some(val);
        self
    }

    #[inline]
    pub fn build(self) -> Config {
        Config {
            initial_scan_cache_size: self
                .initial_scan_cache_size
                .unwrap_or(DEFAULT_SCAN_CACHE_SIZE),
            initial_retire_cache_size: self
                .initial_retire_cache_size
                .unwrap_or(DEFAULT_RETIRE_CACHE_SIZE),
            ops_count_threshold: self.ops_count_threshold.unwrap_or(DEFAULT_OPS_COUNT_THRESHOLD),
            count_strategy: self.count_strategy.unwrap_or(DEFAULT_COUNT_STRATEGY),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Config
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Hash, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct Config {
    /// The initial size of the scan cache, choosing a suitably large size can
    /// prevent re-allocations at runtime
    pub initial_scan_cache_size: usize,
    pub initial_retire_cache_size: usize,
    pub ops_count_threshold: u32,
    pub count_strategy: CountStrategy,
}

/********** impl Default **************************************************************************/

impl Default for Config {
    #[inline]
    fn default() -> Self {
        Self {
            initial_scan_cache_size: DEFAULT_SCAN_CACHE_SIZE,
            initial_retire_cache_size: DEFAULT_RETIRE_CACHE_SIZE,
            ops_count_threshold: DEFAULT_OPS_COUNT_THRESHOLD,
            count_strategy: Default::default(),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// CountStrategy
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Hash, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum CountStrategy {
    Release,
    Retire,
}

/********** impl Default **************************************************************************/

impl Default for CountStrategy {
    #[inline]
    fn default() -> Self {
        DEFAULT_COUNT_STRATEGY
    }
}
