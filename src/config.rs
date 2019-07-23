//! Types for global one-time configuration of the runtime parameters used by
//! the reclamation scheme.

const DEFAULT_INIT_CACHE: usize = 128;
const DEFAULT_MIN_REQUIRED_RECORDS: u32 = 0;
const DEFAULT_SCAN_THRESHOLD: u32 = 128;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Config
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Runtime configuration parameters.
#[derive(Copy, Clone, Debug)]
pub struct Config {
    init_cache: usize,
    min_required_records: u32,
    scan_threshold: u32,
}

/********** impl Default **************************************************************************/

impl Default for Config {
    #[inline]
    fn default() -> Self {
        ConfigBuilder::new().build()
    }
}

/********** impl inherent *************************************************************************/

impl Config {
    /// Creates a new [`Config`] with the given parameters
    ///
    /// # Panics
    ///
    /// This function panics, if `scan_threshold` is 0.
    #[inline]
    pub fn with_params(init_cache: usize, min_required_records: u32, scan_threshold: u32) -> Self {
        assert!(scan_threshold > 0, "scan threshold must be greater than 0");
        Self { init_cache, min_required_records, scan_threshold }
    }

    /// Returns the initial cache size for newly spawned threads.
    #[inline]
    pub fn init_cache(&self) -> usize {
        self.init_cache
    }

    /// Returns the minimum amount of retired records that is required, before
    /// an attempt at reclaiming records is initiated.
    #[inline]
    pub fn min_required_records(&self) -> u32 {
        self.min_required_records
    }

    /// Returns the scan threshold.
    ///
    /// Every retired record or dropped hazard `Guard` (depending on which
    /// feature is selected) counts towards this threshold.
    /// Once it is reached, an attempt is made to reclaim records.
    #[inline]
    pub fn scan_threshold(&self) -> u32 {
        self.scan_threshold
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// ConfigBuilder
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A builder type for gradually initializing a [`Config`].
///
/// This is mainly useful for keeping stability, in case the internal structure
/// of the [`Config`] type changes in the future, e.g. because further
/// parameters are added.
#[derive(Copy, Clone, Debug, Default)]
pub struct ConfigBuilder {
    init_cache: Option<usize>,
    min_required_records: Option<u32>,
    scan_threshold: Option<u32>,
}

impl ConfigBuilder {
    /// Creates a new [`ConfigBuilder`] with default values.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the initial size of the cache for retired records of each newly
    /// created thread.
    ///
    /// If this is set to e.g. 0, retiring the first record will require the
    /// allocation of memory by the internally used data structure.
    #[inline]
    pub fn init_cache(mut self, init_cache: usize) -> Self {
        self.init_cache = Some(init_cache);
        self
    }

    /// Sets the minimum amount of records that must have been retired by a
    /// thread, before the thread may attempt to reclaim any memory.
    #[inline]
    pub fn min_required_records(mut self, min_required_records: u32) -> Self {
        self.min_required_records = Some(min_required_records);
        self
    }

    /// Sets the scan threshold.
    #[inline]
    pub fn scan_threshold(mut self, scan_threshold: u32) -> Self {
        self.scan_threshold = Some(scan_threshold);
        self
    }

    /// Consumes the [`ConfigBuilder`] and returns a initialized [`Config`].
    ///
    /// Unspecified parameters are initialized with their default values.
    #[inline]
    pub fn build(self) -> Config {
        Config::with_params(
            self.init_cache.unwrap_or(DEFAULT_INIT_CACHE),
            self.min_required_records.unwrap_or(DEFAULT_MIN_REQUIRED_RECORDS),
            self.scan_threshold.unwrap_or(DEFAULT_SCAN_THRESHOLD),
        )
    }
}
