const DEFAULT_SCAN_CACHE_SIZE: usize = 128;
const DEFAULT_MAX_RESERVED_HAZARD_POINTERS: u32 = 16;
const DEFAULT_OPS_COUNT_THRESHOLD: u32 = 128;
const DEFAULT_COUNT_STRATEGY: Operation = Operation::Retire;

////////////////////////////////////////////////////////////////////////////////////////////////////
// ConfigBuilder
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Default, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub struct ConfigBuilder {
    initial_scan_cache_size: Option<usize>,
    max_reserved_hazard_pointers: Option<u32>,
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
            max_reserved_hazard_pointers: self
                .max_reserved_hazard_pointers
                .unwrap_or(DEFAULT_MAX_RESERVED_HAZARD_POINTERS),
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
    pub initial_scan_cache_size: usize,
    pub max_reserved_hazard_pointers: u32,
    pub ops_count_threshold: u32,
    pub count_strategy: Operation,
}

/********* impl inherent **************************************************************************/

impl Config {
    #[inline]
    pub fn is_count_release(&self) -> bool {
        self.count_strategy == Operation::Release
    }

    #[inline]
    pub fn is_count_retire(&self) -> bool {
        self.count_strategy == Operation::Retire
    }
}

/********** impl Default **************************************************************************/

impl Default for Config {
    #[inline]
    fn default() -> Self {
        Self {
            initial_scan_cache_size: DEFAULT_SCAN_CACHE_SIZE,
            max_reserved_hazard_pointers: DEFAULT_MAX_RESERVED_HAZARD_POINTERS,
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
