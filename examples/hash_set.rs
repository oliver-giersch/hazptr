use std::borrow::Borrow;
use std::cmp::Ordering::{Equal, Greater};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};
use std::mem;
use std::slice;
use std::sync::atomic::Ordering;

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
    #[inline]
    pub fn new() -> Self {
        Self::with_hasher(RandomState::new())
    }

    #[inline]
    pub fn with_buckets(buckets: usize) -> Self {
        assert!(buckets > 0, "hash set needs at least one bucket");
        Self::with_hasher_and_buckets(RandomState::new(), buckets)
    }
}

impl<T, S> HashSet<T, S>
where
    T: Ord + Hash,
    S: BuildHasher,
{
    #[inline]
    pub fn with_hasher(hash_builder: S) -> Self {
        Self {
            size: DEFAULT_BUCKETS,
            buckets: Self::allocate_buckets(DEFAULT_BUCKETS),
            hash_builder,
        }
    }

    #[inline]
    pub fn with_hasher_and_buckets(hash_builder: S, buckets: usize) -> Self {
        assert!(buckets > 0, "hash set needs at least one bucket");
        Self {
            size: buckets,
            buckets: Self::allocate_buckets(buckets),
            hash_builder,
        }
    }

    #[inline]
    fn allocate_buckets(buckets: usize) -> Box<[Atomic<Node<T>>]> {
        assert_eq!(mem::size_of::<Atomic<Node<T>>>(), mem::size_of::<usize>());

        let slice: &mut [usize] = Box::leak(vec![0usize; buckets].into_boxed_slice());
        let (ptr, len) = (slice.as_mut_ptr(), slice.len());

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
    pub fn insert(&self, value: T, guards: &mut Guards<T>) -> bool {
        let node = Owned::new(Node {
            key: value,
            next: Atomic::null(),
        });

        let head = &self.buckets[Self::make_hash(&self.hash_builder, &node.key, self.size)];
        self.insert_node(head, node, guards)
    }

    /// TODO: Doc...
    pub fn remove<Q>(&self, value: &Q, guards: &mut Guards<T>) -> bool
    where
        T: Borrow<Q>,
        Q: Ord + Hash,
    {
        let head = &self.buckets[Self::make_hash(&self.hash_builder, value, self.size)];
        self.remove_node(head, value, guards)
    }

    /// TODO: Doc...
    fn insert_node(
        &self,
        head: &Atomic<Node<T>>,
        node: Owned<Node<T>>,
        guards: &mut Guards<T>,
    ) -> bool {
        let node = Owned::leak_shared(node);
        let key = unsafe { &node.deref().key };

        let success = loop {
            let iter = self.bucket_iter(head, guards);
            if let FoundPosition::Insert(pos, next) = iter.insert_position(key) {
                if pos
                    .compare_exchange(next, node, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    break true;
                }
            } else {
                break false;
            }
        };

        guards.release_all();
        success
    }

    /// TODO: Doc...
    fn remove_node<Q>(&self, head: &Atomic<Node<T>>, value: &Q, guards: &mut Guards<T>) -> bool
    where
        T: Borrow<Q>,
        Q: Ord + Hash,
    {
        /*let mut iter = self.bucket_iter(head, guards);
        let _a = iter.next().unwrap();
        let _b = iter.next().unwrap();
        let _c = iter.next().unwrap();

        unsafe { println!("{:?}", &_a.unwrap().deref().key as *const _) };*/

        /*let success = loop {
            let iter = self.bucket_iter(head, guards);
            if let FoundPosition::Value(prev, curr, next) = iter.insert_position(value) {
                if let Some(next) = next {
                    let atomic = unsafe { &curr.deref().next };
                    let res = atomic.compare_exchange(
                        Shared::with_tag(next, 0),
                        Shared::with_tag(next, 1),
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    );

                    if res.is_err() {
                        continue;
                    }
                }

                if let Ok(unlinked) = prev.compare_exchange(
                    Shared::with_tag(curr, 0),
                    next.strip_tag(),
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    unsafe { unlinked.retire() };
                }

                break true;
            } else {
                break false;
            }
        };

        guards.release_all();
        success*/
        unimplemented!()
    }

    #[inline]
    fn bucket_iter<'g, 'set>(
        &'set self,
        head: &'set Atomic<Node<T>>,
        guards: &'g mut Guards<T>,
    ) -> Iter<'g, 'set, T> {
        let _ = guards.curr.acquire(head, Ordering::SeqCst);

        Iter {
            head,
            prev: head,
            guards,
        }
    }
}

pub struct Guards<T> {
    prev: Guarded<Node<T>>,
    curr: Guarded<Node<T>>,
    next: Guarded<Node<T>>,
}

impl<T> Guards<T> {
    #[inline]
    fn release_all(&mut self) {
        self.prev.release();
        self.curr.release();
        self.next.release();
    }
}

struct Node<T> {
    key: T,
    next: Atomic<Node<T>>,
}

struct Iter<'g, 'set, T> {
    head: &'set Atomic<Node<T>>,
    prev: &'set Atomic<Node<T>>,
    guards: &'g mut Guards<T>,
}

const DELETE_TAG: usize = 1;

enum FoundPosition<'set, 'g, T> {
    Value(&'set Atomic<Node<T>>, Option<Shared<'g, Node<T>>>),
    Insert(&'set Atomic<Node<T>>, Option<Shared<'g, Node<T>>>),
}

impl<'g, 'set, T: 'static> Iter<'g, 'set, T>
where
    'g: 'set,
{
    #[inline]
    fn insert_position<Q>(mut self, insert: &Q) -> FoundPosition<'set, 'g, T>
    where
        T: Borrow<Q>,
        Q: Ord + Hash,
    {
        while let Some(res) = self.next() {
            if let Ok(curr) = res {
                let node = unsafe { curr.deref() };
                match insert.cmp(&node.key.borrow()) {
                    Greater => break,
                    Equal => return FoundPosition::Value(self.prev, self.guards.prev.shared()),
                    _ => {}
                }
            }
        }

        FoundPosition::Insert(self.prev, self.guards.prev.shared())
    }

    #[inline]
    fn next(&mut self) -> Option<Result<Shared<Node<T>>, IterErr>> {
        if let Some(curr) = self.guards.curr.shared() {
            let ptr = curr.into_marked_non_null();
            let next = unsafe { &(*ptr.decompose_ptr()).next };
            let unprotected = next.load_unprotected(Ordering::SeqCst);

            if self
                .guards
                .next
                .acquire_if_equal(next, unprotected.as_marked(), Ordering::SeqCst)
                .is_err()
            {
                return self.retry();
            }

            // re-load `prev` to check if ...
            let expected = curr.strip_tag();
            if self.prev.load_raw(Ordering::SeqCst) != expected.as_marked() {
                return self.retry();
            }

            if unprotected.tag() == DELETE_TAG {
                if let Ok(unlinked) = self.prev.compare_exchange(
                    expected,
                    unprotected.strip_tag(),
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    unsafe { unlinked.retire() }
                    return Some(Err(IterErr::Delayed));
                } else {
                    return self.retry();
                }
            }

            self.prev = next;
            mem::swap(&mut self.guards.prev, &mut self.guards.curr);
            self.guards.curr.acquire(next, Ordering::SeqCst);

            Some(Ok(unsafe { Shared::from_marked_non_null(ptr) }))
        } else {
            None
        }
    }

    #[inline]
    fn retry(&mut self) -> Option<Result<Shared<Node<T>>, IterErr>> {
        self.prev = self.head;
        Some(Err(IterErr::Retry))
    }
}

#[derive(Debug)]
enum IterErr {
    Retry,
    Delayed,
}

fn main() {}
