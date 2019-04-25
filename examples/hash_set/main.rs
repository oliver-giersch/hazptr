use std::borrow::Borrow;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};
use std::mem;
use std::slice;

mod ordered;

use crate::ordered::{Guards, OrderedSet};

const DEFAULT_BUCKETS: usize = 256;

/// A concurrent hash set.
pub struct HashSet<T, S = RandomState> {
    size: usize,
    buckets: Box<[OrderedSet<T>]>,
    hash_builder: S,
}

impl<T: Ord + Hash> Default for HashSet<T, RandomState> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Ord + Hash> HashSet<T, RandomState> {
    /// Creates a new hash set.
    #[inline]
    pub fn new() -> Self {
        Self::with_hasher(RandomState::new())
    }

    /// Creates a new hash set with the specified number of buckets.
    ///
    /// # Panics
    ///
    /// This function will panic, if `buckets` is 0.
    #[inline]
    pub fn with_buckets(buckets: usize) -> Self {
        Self::with_hasher_and_buckets(RandomState::new(), buckets)
    }
}

impl<T, S> HashSet<T, S>
where
    T: Hash + Ord,
    S: BuildHasher,
{
    /// Creates a new hash set with the default number of buckets and the given `hash_builder`.
    #[inline]
    pub fn with_hasher(hash_builder: S) -> Self {
        Self {
            size: DEFAULT_BUCKETS,
            buckets: Self::allocate_buckets(DEFAULT_BUCKETS),
            hash_builder,
        }
    }

    /// Creates a new hash set with the specified number of buckets and the given `hash_builder`.
    ///
    /// # Panics
    ///
    /// This function will panic, if `buckets` is 0.
    #[inline]
    pub fn with_hasher_and_buckets(hash_builder: S, buckets: usize) -> Self {
        assert!(buckets > 0, "hash set needs at least one bucket");
        Self { size: buckets, buckets: Self::allocate_buckets(buckets), hash_builder }
    }

    /// Returns the number of buckets in this hash set.
    #[inline]
    pub fn buckets(&self) -> usize {
        self.size
    }

    /// Returns a reference to the set's `BuildHasher`.
    #[inline]
    pub fn hasher(&self) -> &S {
        &self.hash_builder
    }

    /// Allocates a boxed slice of ordered sets.
    #[inline]
    fn allocate_buckets(buckets: usize) -> Box<[OrderedSet<T>]> {
        assert_eq!(mem::size_of::<OrderedSet<T>>(), mem::size_of::<usize>());

        let slice: &mut [usize] = Box::leak(vec![0usize; buckets].into_boxed_slice());
        let (ptr, len) = (slice.as_mut_ptr(), slice.len());

        // this is safe because `Atomic::null()` and `0usize` have the same in-memory representation
        unsafe {
            let slice: &mut [OrderedSet<T>] = slice::from_raw_parts_mut(ptr as *mut _, len);
            Box::from_raw(slice)
        }
    }

    /// Generates a hash for `value` and transforms it into a slice index for the given number of
    /// buckets.
    #[inline]
    fn make_hash<Q>(builder: &S, value: &Q, buckets: usize) -> usize
    where
        T: Borrow<Q>,
        Q: Hash + Ord,
    {
        let mut state = builder.build_hasher();
        value.hash(&mut state);
        (state.finish() % buckets as u64) as usize
    }
}

impl<T, S> HashSet<T, S>
where
    T: Hash + Ord + 'static,
    S: BuildHasher,
{
    /// Returns `true` if the set contains the given `value`.
    #[inline]
    pub fn contains<Q>(&self, value: &Q, guards: &mut Guards<T>) -> bool
    where
        T: Borrow<Q>,
        Q: Hash + Ord,
    {
        let set = &self.buckets[Self::make_hash(&self.hash_builder, value, self.size)];
        set.get(value, guards).is_some()
    }

    /// TODO: Doc...
    #[inline]
    pub fn get<'g, Q>(&self, value: &Q, guards: &'g mut Guards<T>) -> Option<&'g T>
    where
        T: Borrow<Q>,
        Q: Hash + Ord,
    {
        let set = &self.buckets[Self::make_hash(&self.hash_builder, value, self.size)];
        set.get(value, guards)
    }

    /// TODO: Doc...
    #[inline]
    pub fn insert(&self, value: T, guards: &mut Guards<T>) -> bool {
        let set = &self.buckets[Self::make_hash(&self.hash_builder, &value, self.size)];
        set.insert_node(value, guards)
    }

    /// TODO: Doc...
    #[inline]
    pub fn remove<Q>(&self, value: &Q, guards: &mut Guards<T>) -> bool
    where
        T: Borrow<Q>,
        Q: Ord + Hash,
    {
        let set = &self.buckets[Self::make_hash(&self.hash_builder, value, self.size)];
        set.remove_node(value, guards)
    }
}

fn main() {
    assert!(true);   
}
