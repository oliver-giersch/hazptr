use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::Ordering;

use hazptr::reclaim::Protect;
use hazptr::typenum;

type Atomic<T> = hazptr::Atomic<T, typenum::U1>;
type Guarded<T> = hazptr::Guarded<T, typenum::U1>;
type Owned<T> = hazptr::Owned<T, typenum::U1>;
type Shared<'g, T> = hazptr::Shared<'g, T, typenum::U1>;

pub struct HashSet<K, S = RandomState> {
    hash_builder: S,
    size: usize,
    buckets: Box<[Atomic<Node<K>>]>,
    _x: (K, S),
}

impl<T, S> HashSet<T, S>
where
    T: Eq + Hash,
    S: BuildHasher,
{
    pub fn with_buckets(buckets: usize) -> Self {
        unimplemented!()
    }

    pub fn insert(&self, key: T) -> bool {
        let mut state = self.hash_builder.build_hasher();
        key.hash(&mut state);
        let idx = (state.finish() % self.size as u64) as usize;

        let node = Owned::new(Node {
            key,
            next: Atomic::null(),
        });

        let bucket = &self.buckets[idx];

        self.insert_node(bucket, node)
    }

    fn insert_node(&self, head: &Atomic<Node<T>>, node: Owned<Node<T>>) -> bool {
        let key = &node.key;
        loop {
            /*
            if find(head, key) {
                return false
            }
            */
        }

        false
    }
}

struct Node<T> {
    key: T,
    next: Atomic<Node<T>>,
}

struct Iter<'set, T> {
    head: &'set Atomic<Node<T>>,
    prev: &'set Atomic<Node<T>>,
    hp0: Guarded<Node<T>>, //curr
    hp1: Guarded<Node<T>>, //next
    hp2: Guarded<Node<T>>, //swap
}

impl<'set, T> Iterator for Iter<'set, T> {
    type Item = Result<Shared<'set, Node<T>>, IterErr>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        // check hp1
        match self.hp1.shared() {
            None => None,
            Some(cur) => {
                let next = unsafe { &cur.deref().next };

                let next_raw = next.load_raw(Ordering::SeqCst);
                let tag = next_raw.decompose_tag();
                if self
                    .hp0
                    .acquire_if_equal(next, next_raw, Ordering::SeqCst)
                    .is_err()
                {
                    // restart, ret Err
                    unimplemented!()
                }

                // FIXME: can not compare Shared with Option<Unprotected>?
                if Shared::with_tag(cur, 0) != self.prev.load_raw(Ordering::SeqCst) {
                    // restart, ret Err
                    unimplemented!()
                }

                if tag == 0 {
                    unimplemented!()
                } else {
                    unimplemented!()
                }

                //self.curr = self.next;

                // prep hp1 for next iteration (good)
                //self.hp1.acquire(next, &);
            }
        }
    }
}

impl<'set, K> Iter<'set, K> {
    fn new(head: &'set Atomic<Node<K>>) -> Self {
        let mut iter = Iter {
            head,
            prev: head,
            hp0: Guarded::default(),
            hp1: Guarded::default(),
            hp2: Guarded::default(),
        };

        // TODO: Check ordering...
        let _ = iter.hp1.acquire(&iter.prev, Ordering::SeqCst);

        iter
    }

    fn restart(&mut self) {
        self.prev = self.head;
    }
}

enum IterErr {
    Stalled,
}

fn main() {}
