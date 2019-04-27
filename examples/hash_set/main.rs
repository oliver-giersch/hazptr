use std::borrow::Borrow;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};
use std::mem;
use std::slice;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

mod ordered;
mod original;

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

////////////////////////////////////////////////////////////////////////////////////////////////////
// Example
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Default)]
#[repr(align(64))]
struct ThreadCount(AtomicUsize);

#[derive(Debug)]
struct DropI8<'a>(i8, &'a ThreadCount);

impl Borrow<i8> for DropI8<'_> {
    #[inline]
    fn borrow(&self) -> &i8 {
        &self.0
    }
}

impl Hash for DropI8<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl Drop for DropI8<'_> {
    #[inline]
    fn drop(&mut self) {
        (self.1).0.fetch_add(1, Ordering::Relaxed);
    }
}

impl PartialEq for DropI8<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl PartialOrd for DropI8<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

impl Eq for DropI8<'_> {}

impl Ord for DropI8<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

fn test_insert_remove() {
    let set = HashSet::with_buckets(1);
    let mut guards = Guards::new();

    // insert
    assert!(set.insert(0, &mut guards));
    assert!(set.insert(1, &mut guards));
    assert!(set.insert(-10, &mut guards));
    assert!(set.insert(10, &mut guards));
    assert!(set.insert(5, &mut guards));
    assert!(set.insert(-5, &mut guards));
    assert!(set.insert(7, &mut guards));
    assert!(set.insert(-2, &mut guards));

    // remove
    assert!(set.remove(&-10, &mut guards));
    assert!(set.remove(&-5, &mut guards));
    assert!(set.remove(&-2, &mut guards));
    assert!(set.remove(&0, &mut guards));
    assert!(set.remove(&5, &mut guards));
    assert!(set.remove(&7, &mut guards));
    assert!(set.remove(&10, &mut guards));

    assert!(!set.contains(&-10, &mut guards));
    assert!(!set.contains(&-5, &mut guards));
    assert!(!set.contains(&-2, &mut guards));
    assert!(!set.contains(&0, &mut guards));
    assert!(!set.contains(&5, &mut guards));
    assert!(!set.contains(&7, &mut guards));
    assert!(!set.contains(&10, &mut guards));

    println!("test_insert_remove: success");
}

fn test_random() {
    use rand::prelude::*;

    let set = HashSet::with_buckets(1);
    let mut guards = Guards::new();

    let mut conflicts = 0;
    for _ in 0..10_000 {
        let value: i8 = rand::thread_rng().gen();
        if set.contains(&value, &mut guards) {
            conflicts += 1;
            set.remove(&value, &mut guards);
        } else {
            set.insert(value, &mut guards);
        }
    }

    println!("test_random: success, detected {} insertion conflicts", conflicts);
}

fn main() {
    use std::sync::Arc;

    use rand::prelude::*;

    const THREADS: usize = 8;
    const OPS_COUNT: usize = 10_000_000;

    static THREAD_COUNTS: [ThreadCount; THREADS] = [
        ThreadCount(AtomicUsize::new(0)),
        ThreadCount(AtomicUsize::new(0)),
        ThreadCount(AtomicUsize::new(0)),
        ThreadCount(AtomicUsize::new(0)),
        ThreadCount(AtomicUsize::new(0)),
        ThreadCount(AtomicUsize::new(0)),
        ThreadCount(AtomicUsize::new(0)),
        ThreadCount(AtomicUsize::new(0)),
    ];

    //test_insert_remove();
    //test_random();

    // the single bucket ensures maximum contention
    let set = Arc::new(HashSet::with_buckets(1));
    let handles: Vec<_> = (0..THREADS)
        .map(|id| {
            let set = Arc::clone(&set);
            thread::spawn(move || {
                let mut guards = Guards::new();
                let mut alloc_count = 0u32;

                for ops in 0..OPS_COUNT {
                    if ops > 0 && ops % (OPS_COUNT / 10) == 0 {
                        println!("thread {}: {} out of {} ops", id, ops, OPS_COUNT);
                    }

                    let value: i8 = rand::thread_rng().gen();
                    if set.contains(&value, &mut guards) {
                        set.remove(&value, &mut guards);
                    } else {
                        set.insert(DropI8(value, &THREAD_COUNTS[id]), &mut guards);
                        alloc_count += 1;
                    }
                }

                println!("thread {}: done", id);
                alloc_count
            })
        })
        .collect();

    let total_alloc: u32 = handles.into_iter().map(|handle| handle.join().unwrap()).sum();
    mem::drop(set);
    let total_drop: usize = THREAD_COUNTS.iter().map(|count| count.0.load(Ordering::Relaxed)).sum();
    assert_eq!(total_alloc as usize, total_drop);
    println!(
        "main: {} threads reclaimed {} out of {} allocated records",
        THREADS, total_drop, total_alloc
    );
    println!("success, no leaks detected.");
}
