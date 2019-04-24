use std::borrow::Borrow;
use std::cmp::Ordering::{Equal, Greater};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};
use std::mem;
use std::slice;
use std::sync::{atomic::Ordering, Arc};
use std::thread;

use hazptr::reclaim::{MarkedPointer, Protect};
use hazptr::typenum;

type Atomic<T> = hazptr::Atomic<T, typenum::U1>;
type Guarded<T> = hazptr::Guarded<T, typenum::U1>;
type Owned<T> = hazptr::Owned<T, typenum::U1>;
type Shared<'g, T> = hazptr::Shared<'g, T, typenum::U1>;

const DEFAULT_BUCKETS: usize = 256;

/// A concurrent hash set.
pub struct HashSet<T, S = RandomState> {
    size: usize,
    buckets: Box<[Atomic<Node<T>>]>,
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
    T: Ord + Hash,
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

    #[inline]
    fn allocate_buckets(buckets: usize) -> Box<[Atomic<Node<T>>]> {
        assert_eq!(mem::size_of::<Atomic<Node<T>>>(), mem::size_of::<usize>());

        let slice: &mut [usize] = Box::leak(vec![0usize; buckets].into_boxed_slice());
        let (ptr, len) = (slice.as_mut_ptr(), slice.len());

        // this is safe because `Atomic::null()` and `0usize` have the same in-memory representation
        unsafe {
            let slice: &mut [Atomic<Node<T>>] = slice::from_raw_parts_mut(ptr as *mut _, len);
            Box::from_raw(slice)
        }
    }

    #[inline]
    fn make_hash<Q>(builder: &S, key: &Q, buckets: usize) -> usize
    where
        T: Borrow<Q>,
        Q: Hash + Eq,
    {
        let mut state = builder.build_hasher();
        key.hash(&mut state);
        (state.finish() % buckets as u64) as usize
    }
}

impl<T, S> HashSet<T, S>
where
    T: Ord + Hash + 'static,
    S: BuildHasher,
{
    /// TODO: Doc...
    #[inline]
    pub fn insert(&self, value: T, guards: &mut Guards<T>) -> bool {
        let node = Owned::new(Node::new(value));
        let head = &self.buckets[Self::make_hash(&self.hash_builder, &node.elem, self.size)];
        self.insert_node(head, node, guards)
    }

    /// TODO: Doc...
    #[inline]
    pub fn remove<Q>(&self, value: &Q, guards: &mut Guards<T>) -> bool
    where
        T: Borrow<Q>,
        Q: Ord + Hash,
    {
        let head = &self.buckets[Self::make_hash(&self.hash_builder, value, self.size)];
        self.remove_node(head, value, guards)
    }

    /// TODO: Doc...
    #[inline]
    fn insert_node(
        &self,
        head: &Atomic<Node<T>>,
        mut node: Owned<Node<T>>,
        guards: &mut Guards<T>,
    ) -> bool {
        let success = loop {
            let iter = self.bucket_iter(head, guards);
            let key = &node.elem;
            if let Ok((position, next)) = iter.find_insert_position(key) {
                // (SET:1) this ...
                match position.compare_exchange(next, node, Ordering::SeqCst, Ordering::SeqCst) {
                    Ok(_) => break true,
                    Err(fail) => node = fail.input,
                }
            } else {
                break false;
            }
        };

        guards.release_all();
        success
    }

    /// TODO: Doc...
    #[inline]
    fn remove_node<Q>(&self, head: &Atomic<Node<T>>, value: &Q, guards: &mut Guards<T>) -> bool
    where
        T: Borrow<Q>,
        Q: Ord + Hash,
    {
        let success = loop {
            let iter = self.bucket_iter(head, guards);
            match iter.find_insert_position(value) {
                Ok(_) => break false,
                Err(IterPosition { prev, curr, next }) => {
                    if let Some(next) = next {
                        // (SET:2) this ...
                        if unsafe { &curr.deref().next }
                            .compare_exchange(
                                Shared::with_tag(next, 0),
                                Shared::with_tag(next, 1),
                                Ordering::SeqCst,
                                Ordering::SeqCst,
                            )
                            .is_err()
                        {
                            continue;
                        }
                    }

                    // (SET:3) this ...
                    if let Ok(unlinked) = prev.compare_exchange(
                        Shared::with_tag(curr, 0),
                        next.strip_tag(),
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        unsafe { unlinked.retire() };
                    }

                    break true;
                }
            }
        };

        guards.release_all();
        success
    }

    #[inline]
    fn bucket_iter<'g, 'set>(
        &'set self,
        head: &'set Atomic<Node<T>>,
        guards: &'g mut Guards<T>,
    ) -> Iter<'g, 'set, T> {
        // (SET:4) this ...
        let _ = guards.curr.acquire(head, Ordering::SeqCst);

        Iter { head, prev: head, old_prev: head, guards }
    }
}

#[derive(Default)]
pub struct Guards<T> {
    prev: Guarded<Node<T>>,
    curr: Guarded<Node<T>>,
    next: Guarded<Node<T>>,
}

impl<T> Guards<T> {
    #[inline]
    pub fn new() -> Self {
        Self { prev: Guarded::new(), curr: Guarded::new(), next: Guarded::new() }
    }

    #[inline]
    fn release_all(&mut self) {
        self.prev.release();
        self.curr.release();
        self.next.release();
    }
}

#[derive(Debug)]
struct Node<T> {
    elem: T,
    next: Atomic<Node<T>>,
}

impl<T> Node<T> {
    #[inline]
    fn new(elem: T) -> Self {
        Self { elem, next: Atomic::null() }
    }
}

struct Iter<'g, 'set, T> {
    head: &'set Atomic<Node<T>>,
    prev: &'set Atomic<Node<T>>,
    old_prev: &'set Atomic<Node<T>>,
    guards: &'g mut Guards<T>,
}

struct IterPosition<'a, 'set, T> {
    prev: &'set Atomic<Node<T>>,
    curr: Shared<'a, Node<T>>,
    next: Option<Shared<'a, Node<T>>>,
}

#[derive(Debug)]
enum IterErr {
    Retry,
}

type InsertPosition<'g, 'set, T> = (&'set Atomic<Node<T>>, Option<Shared<'g, Node<T>>>);

const DELETE_TAG: usize = 1;

macro_rules! retry {
    ($self:ident) => {
        $self.prev = $self.head;
        return Some(Err(IterErr::Retry));
    };
}

impl<'g, 'set, T: 'static> Iter<'g, 'set, T>
where
    'g: 'set,
{
    #[inline]
    fn find_insert_position<Q>(
        mut self,
        insert: &Q,
    ) -> Result<InsertPosition<'g, 'set, T>, IterPosition<'g, 'set, T>>
    where
        T: Borrow<Q>,
        Q: Ord + Hash,
    {
        while let Some(res) = self.next() {
            if let Ok(pos) = res {
                let key = unsafe { pos.curr.deref().elem.borrow() };
                match key.cmp(insert) {
                    Equal => return Err(self.into_iter_position()),
                    Greater => break,
                    _ => {}
                }
            }
        }

        Ok((self.prev, self.guards.curr.shared()))
    }

    #[inline]
    fn next<'a>(&'a mut self) -> Option<Result<IterPosition<'a, 'set, T>, IterErr>> {
        match self.guards.curr.shared() {
            None => None,
            Some(curr) => {
                // it is necessary to dereference the raw pointer here in order to avoid binding the
                // lifetime of `next` to `'a` since it needs to be at least `'set`.
                let ptr = curr.into_marked_non_null();
                let next = unsafe { &(*ptr.decompose_ptr()).next };
                // (SET:5) this ...
                let unprotected = next.load_unprotected(Ordering::SeqCst);

                // (SET:6) this ...
                if self
                    .guards
                    .next
                    .acquire_if_equal(next, unprotected.as_marked(), Ordering::SeqCst)
                    .is_err()
                {
                    retry!(self);
                }

                let expected = curr.strip_tag();
                // (SET:7) this ...
                if self.prev.load_unprotected(Ordering::SeqCst).as_marked() != expected.as_marked()
                {
                    retry!(self);
                }

                if unprotected.tag() == DELETE_TAG {
                    // (SET:8) this ...
                    if let Ok(unlinked) = self.prev.compare_exchange(
                        expected,
                        unprotected.strip_tag(),
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        unsafe { unlinked.retire() };
                    } else {
                        retry!(self);
                    }
                }

                self.old_prev = self.prev;
                self.prev = next;
                mem::swap(&mut self.guards.prev, &mut self.guards.curr);
                self.guards.curr.acquire(next, Ordering::SeqCst);

                Some(Ok(IterPosition {
                    prev: self.old_prev,
                    curr: self.guards.prev.shared().unwrap(),
                    next: self.guards.next.shared(),
                }))
            }
        }
    }

    #[inline]
    fn into_iter_position(self) -> IterPosition<'g, 'set, T> {
        IterPosition {
            prev: self.old_prev,
            curr: self.guards.prev.shared().unwrap(),
            next: self.guards.next.shared(),
        }
    }
}

fn main() {
    const THREADS: usize = 1;

    let set = Arc::new(HashSet::with_buckets(1));
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let set = Arc::clone(&set);
            thread::spawn(move || {
                let mut guards = Guards::new();
                assert!(set.insert(1, &mut guards));
                assert!(set.insert(2, &mut guards));
                assert!(set.insert(3, &mut guards));
                assert!(!set.insert(3, &mut guards));
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }
}
