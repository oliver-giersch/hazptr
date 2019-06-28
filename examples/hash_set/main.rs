// implementation is currently defunct

mod ordered;

use std::borrow::Borrow;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};
use std::mem;
use std::slice;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use hazptr::Guard;
use reclaim::prelude::*;

use crate::ordered::OrderedSet;

const DEFAULT_BUCKETS: usize = 64;

////////////////////////////////////////////////////////////////////////////////////////////////////
// HashSet
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct HashSet<T, S = RandomState> {
    inner: Arc<RawHashSet<T, S>>,
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
            inner: Arc::new(RawHashSet {
                size: DEFAULT_BUCKETS,
                buckets: Self::allocate_buckets(DEFAULT_BUCKETS),
                hash_builder,
            }),
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
        Self {
            inner: Arc::new(RawHashSet {
                size: buckets,
                buckets: Self::allocate_buckets(buckets),
                hash_builder,
            }),
        }
    }

    /// Returns the number of buckets in this hash set.
    #[inline]
    pub fn buckets(&self) -> usize {
        self.inner.size
    }

    /// Returns a reference to the set's `BuildHasher`.
    #[inline]
    pub fn hasher(&self) -> &S {
        &self.inner.hash_builder
    }

    /// Returns a new handle to the [`HashSet`].
    #[inline]
    pub fn handle(&self) -> Handle<T, S> {
        Handle { inner: Arc::clone(&self.inner), guards: Guards::new() }
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
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Handle
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct Handle<T, S = RandomState> {
    inner: Arc<RawHashSet<T, S>>,
    guards: Guards,
}

impl<T, S> Handle<T, S>
where
    T: Hash + Ord + 'static,
    S: BuildHasher,
{
    /// Returns `true` if the set contains the given `value`.
    /// 
    /// This method requires a mutable `self` reference, because the internally use hazard pointers
    /// must be adapted during iteration of the set.
    #[inline]
    pub fn contains<Q>(&mut self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Hash + Ord,
    {
        self.inner.contains(value, &mut self.guards)
    }

    /// Returns a reference to the value in the set, if any, that is equal to the given value.
    ///
    /// The value may be any borrowed form of the set's value type, but [`Hash`][Hash] and
    /// [`Eq`][Eq] on the borrowed form *must* match those for the value type.
    /// 
    /// This method requires a mutable `self` reference, because the internally use hazard pointers
    /// must be adapted during iteration of the set.
    /// The returned reference is likewise protected by one of these hazard pointers, so it can not
    /// be used after calling another method that mutates these.
    ///
    /// [Hash]: std::hash::Hash
    /// [Eq]: std::cmp::Eq
    #[inline]
    pub fn get<Q>(&mut self, value: &Q) -> Option<&T>
    where
        T: Borrow<Q>,
        Q: Hash + Ord,
    {
        self.inner.get(value, &mut self.guards)
    }

    /// Adds a value to the set.
    ///
    /// If the set did not have this value present, `true` is returned.
    /// If the set did have this value present, `false` is returned.
    #[inline]
    pub fn insert(&mut self, value: T) -> bool {
        self.inner.insert(value, &mut self.guards)
    }

    /// Removes a value from the set. Returns whether the value was
    /// present in the set.
    ///
    /// The value may be any borrowed form of the set's value type, but
    /// [`Hash`][Hash] and [`Eq`][Eq] on the borrowed form *must* match those for
    /// the value type.
    ///
    /// [Hash]: std::hash::Hash
    /// [Eq]: std::cmp::Eq
    #[inline]
    pub fn remove<Q>(&mut self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Ord + Hash,
    {
        self.inner.remove(value, &mut self.guards)
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Guards
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A container for the three hazard pointers required to safely traverse a hash
/// set.
#[derive(Debug, Default)]
struct Guards {
    prev: Guard,
    curr: Guard,
    next: Guard,
}

impl Guards {
    /// Creates a new set of [`Guards`].
    #[inline]
    fn new() -> Self {
        Self { prev: Guard::new(), curr: Guard::new(), next: Guard::new() }
    }

    /// Releases all contained guards.
    #[inline]
    fn release_all(&mut self) {
        self.prev.release();
        self.curr.release();
        self.next.release();
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// RawHashSet
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A concurrent hash set.
struct RawHashSet<T, S = RandomState> {
    size: usize,
    buckets: Box<[OrderedSet<T>]>,
    hash_builder: S,
}

impl<T, S> RawHashSet<T, S>
where
    T: Hash + Ord + 'static,
    S: BuildHasher,
{
    /// Returns `true` if the set contains the given `value`.
    #[inline]
    pub fn contains<Q>(&self, value: &Q, guards: &mut Guards) -> bool
    where
        T: Borrow<Q>,
        Q: Hash + Ord,
    {
        let set = &self.buckets[Self::make_hash(&self.hash_builder, value, self.size)];
        let res = set.get(value, guards).is_some();
        guards.release_all();

        res
    }

    /// Returns a reference to the value in the set, if any, that is equal to the given value.
    ///
    /// The value may be any borrowed form of the set's value type, but [`Hash`][Hash] and
    /// [`Eq`][Eq] on the borrowed form *must* match those for the value type.
    ///
    /// [Hash]: std::hash::Hash
    /// [Eq]: std::cmp::Eq
    #[inline]
    pub fn get<'g, Q>(&self, value: &Q, guards: &'g mut Guards) -> Option<&'g T>
    where
        T: Borrow<Q>,
        Q: Hash + Ord,
    {
        let set = &self.buckets[Self::make_hash(&self.hash_builder, value, self.size)];
        set.get(value, guards)
    }

    /// Adds a value to the set.
    ///
    /// If the set did not have this value present, `true` is returned.
    /// If the set did have this value present, `false` is returned.
    #[inline]
    pub fn insert(&self, value: T, guards: &mut Guards) -> bool {
        let set = &self.buckets[Self::make_hash(&self.hash_builder, &value, self.size)];
        set.insert_node(value, guards)
    }

    /// Removes a value from the set. Returns whether the value was
    /// present in the set.
    ///
    /// The value may be any borrowed form of the set's value type, but
    /// [`Hash`][Hash] and [`Eq`][Eq] on the borrowed form *must* match those for
    /// the value type.
    ///
    /// [Hash]: std::hash::Hash
    /// [Eq]: std::cmp::Eq
    #[inline]
    pub fn remove<Q>(&self, value: &Q, guards: &mut Guards) -> bool
    where
        T: Borrow<Q>,
        Q: Ord + Hash,
    {
        let set = &self.buckets[Self::make_hash(&self.hash_builder, value, self.size)];
        set.remove_node(value, guards)
    }
}

impl<T, S> RawHashSet<T, S>
    where
        T: Hash + Ord,
        S: BuildHasher,
{
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
    let mut handle = set.handle();

    // insert
    assert!(handle.insert(0));
    assert!(handle.insert(1));
    assert!(handle.insert(-10));
    assert!(handle.insert(10));
    assert!(handle.insert(5));
    assert!(handle.insert(-5));
    assert!(handle.insert(7));
    assert!(handle.insert(-2));

    // remove
    assert!(handle.remove(&-10));
    assert!(handle.remove(&-5));
    assert!(handle.remove(&-2));
    assert!(handle.remove(&0));
    assert!(handle.remove(&5));
    assert!(handle.remove(&7));
    assert!(handle.remove(&10));

    assert!(!handle.contains(&-10));
    assert!(!handle.contains(&-5));
    assert!(!handle.contains(&-2));
    assert!(!handle.contains(&0));
    assert!(!handle.contains(&5));
    assert!(!handle.contains(&7));
    assert!(!handle.contains(&10));

    println!("test_insert_remove: success");
}

fn test_random() {
    use rand::prelude::*;

    let set = HashSet::with_buckets(1);
    let mut handle = set.handle();

    let mut conflicts = 0;
    for _ in 0..10_000 {
        let value: i8 = rand::thread_rng().gen();
        if handle.contains(&value) {
            conflicts += 1;
            handle.remove(&value);
        } else {
            handle.insert(value);
        }
    }

    println!("test_random: success, detected {} insertion conflicts", conflicts);
}

fn main() {
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

    test_insert_remove();
    test_random();

    // the single bucket ensures maximum contention
    let set = HashSet::with_buckets(1);

    let handles: Vec<_> = (0..THREADS)
        .map(|id| {
            let mut handle = set.handle();
            thread::spawn(move || {
                let mut alloc_count = 0u32;

                for ops in 0..OPS_COUNT {
                    if ops > 0 && ops % (OPS_COUNT / 10) == 0 {
                        println!("thread {}: {} out of {} ops", id, ops, OPS_COUNT);
                    }

                    let value: i8 = rand::thread_rng().gen();
                    if handle.contains(&value) {
                        handle.remove(&value);
                    } else {
                        handle.insert(DropI8(value, &THREAD_COUNTS[id]));
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
