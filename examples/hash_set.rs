use std::borrow::Borrow;
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

pub struct HashSet<T, S = RandomState> {
    size: usize,
    buckets: Box<[Atomic<Node<T>>]>,
    hash_builder: S,
}

impl<T: Eq + Hash> Default for HashSet<T, RandomState> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Eq + Hash> HashSet<T, RandomState> {
    pub fn new() -> Self {
        Self {
            size: DEFAULT_BUCKETS,
            buckets: Self::allocate_buckets(DEFAULT_BUCKETS),
            hash_builder: RandomState::new(),
        }
    }

    pub fn with_buckets(buckets: usize) -> Self {
        assert!(buckets > 0, "hash set needs at least one bucket");
        Self {
            size: buckets,
            buckets: Self::allocate_buckets(buckets),
            hash_builder: RandomState::new(),
        }
    }
}

impl<T, S> HashSet<T, S>
where
    T: Eq + Hash,
    S: BuildHasher,
{
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
    fn make_hash(builder: &S, key: &T, buckets: usize) -> usize {
        let mut state = builder.build_hasher();
        key.hash(&mut state);
        (state.finish() % buckets as u64) as usize
    }
}

impl<T, S> HashSet<T, S>
where
    T: Eq + Hash + 'static,
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
    pub fn remove<Q>(&self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Hash + Eq,
    {
        unimplemented!()
    }

    fn insert_node(
        &self,
        head: &Atomic<Node<T>>,
        node: Owned<Node<T>>,
        guards: &mut Guards<T>,
    ) -> bool {
        let key = &node.key as *const _;
        let node = Owned::leak_shared(node);

        let success = loop {
            let iter = self.bucket_iter(head, guards);
            if let Ok((insert, curr)) = iter.insert_position(key) {
                if insert
                    .compare_exchange(curr, node, Ordering::SeqCst, Ordering::SeqCst)
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

    fn remove_node(&self, head: &Atomic<Node<T>>, value: &T, guards: &mut Guards<T>) -> bool {
        let mut iter = self.bucket_iter(head, guards);

        let a = iter.next().unwrap().unwrap();
        let b = iter.next().unwrap();
        let c = iter.next().unwrap();
        let d = iter.next().unwrap();

        //let x = unsafe { &a.deref().key };
        //println!("{:p}", x);

        // find or last

        /*let insert = iter
        .find_map(|res| match res {
            Ok(node) => {
                if &node.key == value {
                    Some(&node.next)
                } else {
                    None
                }
            }
            Err(_) => None,
        })
        .unwrap_or_else(|| iter.prev);*/

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

impl<'g, 'set, T: 'static> Iter<'g, 'set, T>
where
    'g: 'set,
{
    fn insert_position(
        mut self,
        insert: *const T,
    ) -> Result<(&'set Atomic<Node<T>>, Option<Shared<'g, Node<T>>>), ()> {
        while let Some(res) = self.next() {
            if let Ok(shared) = res {
                let node = unsafe { shared.deref() };
                if &node.key as *const _ > insert {
                    break;
                } else if &node.key as *const _ == insert {
                    return Err(());
                }
            }
        }

        Ok((self.prev, self.guards.prev.shared()))
    }

    fn next(&mut self) -> Option<Result<Shared<Node<T>>, IterErr>> {
        if let Some(curr) = self.guards.curr.shared() {
            let ptr = curr.into_marked_non_null();
            let next = unsafe { &ptr.as_ref().next };
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
                    // FIXME: soundness?
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
