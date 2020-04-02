const DEFAULT_SCAN_CACHE_SIZE: usize = 128;
const DEFAULT_OPS_COUNT_THRESHOLD: u32 = 128;
const DEFAULT_COUNT_STRATEGY: Operation = Operation::Retire;

////////////////////////////////////////////////////////////////////////////////////////////////////
// ConfigBuilder
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct ConfigBuilder {
    initial_scan_cache_size: Option<usize>,
    ops_count_threshold: Option<u32>,
    count_strategy: Option<Operation>,
}

/********** impl inherent *************************************************************************/

impl ConfigBuilder {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn initial_scan_cache_size(mut self, val: usize) -> Self {
        self.initial_scan_cache_size = Some(val);
        self
    }

    #[inline]
    pub fn build(self) -> Config {
        Config {
            initial_scan_cache_size: self
                .initial_scan_cache_size
                .unwrap_or(DEFAULT_SCAN_CACHE_SIZE),
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
    pub ops_count_threshold: u32,
    pub count_strategy: Operation,
}

/********** impl Default **************************************************************************/

impl Default for Config {
    #[inline]
    fn default() -> Self {
        Self {
            initial_scan_cache_size: DEFAULT_SCAN_CACHE_SIZE,
            ops_count_threshold: DEFAULT_OPS_COUNT_THRESHOLD,
            count_strategy: Default::default(),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Operation
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Hash, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Operation {
    Release,
    Retire,
}

/********** impl Default **************************************************************************/

impl Default for Operation {
    #[inline]
    fn default() -> Self {
        DEFAULT_COUNT_STRATEGY
    }
}
