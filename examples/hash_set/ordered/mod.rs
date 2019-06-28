use std::borrow::Borrow;
use std::cmp::Ordering::{Equal, Greater};
use std::mem;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};

use hazptr::typenum;
use reclaim::align::CacheAligned;
use reclaim::prelude::*;
use typenum::U1;

use crate::Guards;

use self::FindResult::*;

pub type Atomic<T> = hazptr::Atomic<T, U1>;
pub type Owned<T> = hazptr::Owned<T, U1>;
pub type Shared<'g, T> = hazptr::Shared<'g, T, U1>;

const DELETE_TAG: usize = 1;

/// A concurrent linked-list based ordered set.
#[derive(Debug, Default)]
pub(crate) struct OrderedSet<T> {
    head: Atomic<Node<T>>,
}

impl<T> OrderedSet<T>
where
    T: Ord + 'static,
{
    /// Inserts a new node for the given `value` and returns `true`, if it did
    /// not already exist in the set.
    #[inline]
    pub fn insert_node(&self, value: T, guards: &mut Guards) -> bool {
        let mut node = Owned::new(Node::new(value));

        let success = loop {
            let elem = node.elem();
            if let Insert { prev, next } = self.find(elem, guards) {
                node.next().store(next, Relaxed);
                // (ORD:1) this `Release` CAS synchronizes-with the `Acquire` CAS (ORD:3) and the
                // `Acquire` loads (ORD:4) and (ORD:5)
                match prev.compare_exchange(next, node, Release, Relaxed) {
                    Ok(_) => break true,
                    Err(failure) => node = failure.input,
                }
            } else {
                break false;
            }
        };

        guards.release_all();
        success
    }

    /// Tries to remove a node containing the given `value` from the set and
    /// returns `true`, if the value was found and successfully removed.
    #[inline]
    pub fn remove_node<Q>(&self, value: &Q, guards: &mut Guards) -> bool
    where
        T: Borrow<Q>,
        Q: Ord,
    {
        let success = loop {
            match self.find(value, guards) {
                Insert { .. } => break false,
                Found { prev, curr, next } => {
                    let next_marked = Marked::marked(next, DELETE_TAG);
                    // (ORD:2) this `Acquire` CAS synchronizes-with the `Release` CAS (ORD:1),
                    // (ORD:3), (ORD:a)
                    if curr.next().compare_exchange(next, next_marked, Acquire, Relaxed).is_err() {
                        continue;
                    }

                    // (ORD:3) this `Release` CAS synchronizes-with the `Acquire` CAS (ORD:2) and
                    // the `Acquire` loads (ORD:4) and (ORD:5)
                    match prev.compare_exchange(curr, next, Release, Relaxed) {
                        Ok(unlinked) => unsafe { unlinked.retire() },
                        Err(_) => {
                            let _ = self.find(value, guards);
                        }
                    }

                    break true;
                }
            };
        };

        guards.release_all();
        success
    }

    /// Returns a reference to the value in the set, if any, that is equal to
    /// the given `value`.
    #[inline]
    pub fn get<'g, Q>(&self, value: &Q, guards: &'g mut Guards) -> Option<&'g T>
    where
        T: Borrow<Q>,
        Q: Ord,
    {
        match self.find(value, guards) {
            Found { curr, .. } => Some(Shared::into_ref(curr).elem()),
            Insert { .. } => None,
        }
    }

    // this function uses unsafe code internally, but the interface is safe:
    // the three guards are each advanced in turn and are guaranteed to eventually protect all of
    // the returned references.
    // FIXME: Try some refactoring when NLL+ are there?
    fn find<'set, 'g, Q>(&'set self, value: &Q, guards: &'g mut Guards) -> FindResult<'set, 'g, T>
    where
        T: Borrow<Q>,
        Q: Ord,
        'g: 'set,
    {
        'retry: loop {
            // prev is still protected by guards.prev (except in the first iteration where prev == head)
            let mut prev = &self.head;
            // (ORD:4) this `Acquire` load synchronizes-with the `Release` CAS (ORD:1), (ORD:3) and
            // (ORD:6)
            // prev is protected by guards.curr and the node holding prev by guards.prev
            while let Some(curr_marked) = prev.load(Acquire, &mut guards.curr) {
                let (curr, curr_tag) = Shared::decompose(curr_marked);
                if curr_tag == DELETE_TAG {
                    continue 'retry;
                }

                let curr_next: &'g Atomic<Node<_>> = unsafe { &*(curr.next() as *const _) };
                let next_raw = curr_next.load_raw(Relaxed);

                // (ORD:5) this `Acquire` load synchronizes-with the `Release`CAS (ORD:1),
                // (ORD:3) and (ORD:6)
                // next is protected by guards.next
                match curr_next.load_marked_if_equal(next_raw, Acquire, &mut guards.next) {
                    Err(_) => continue 'retry,
                    Ok(next_marked) => {
                        if prev.load_raw(Relaxed) != curr.as_marked_ptr() {
                            continue 'retry;
                        }

                        let (next, next_tag) = Marked::decompose(next_marked);
                        if next_tag == DELETE_TAG {
                            // (ORD:6) this `Release` CAS synchronizes-with the `Acquire` CAS
                            // (ORD:2) and the `Acquire` loads (ORD:4) and (ORD:5)
                            match prev.compare_exchange(curr, next, Release, Relaxed) {
                                Ok(unlinked) => unsafe { unlinked.retire() },
                                Err(_) => continue 'retry,
                            };
                        } else {
                            match curr.elem().borrow().cmp(value) {
                                Equal => return unsafe { found_result(prev, curr, next) },
                                Greater => return unsafe { insert_result(prev, curr) },
                                _ => {}
                            };

                            prev = curr_next;
                            // the old prev is no longer be protected afterwards
                            mem::swap(&mut guards.prev, &mut guards.curr);
                        }
                    }
                };
            }

            return Insert { prev, next: None };
        }
    }
}

impl<T> Drop for OrderedSet<T> {
    #[inline]
    fn drop(&mut self) {
        let mut node = self.head.take();
        while let Some(mut curr) = node {
            node = curr.next.take();
        }
    }
}

#[inline]
unsafe fn found_result<'a, 'set: 'a, 'g: 'set, T: 'static>(
    prev: &'set Atomic<Node<T>>,
    curr: Shared<'a, Node<T>>,
    next: Marked<Shared<'a, Node<T>>>,
) -> FindResult<'set, 'g, T> {
    Found { prev, curr: mem::transmute(curr), next: mem::transmute(next) }
}

#[inline]
unsafe fn insert_result<'a, 'set: 'a, 'g: 'set, T: 'static>(
    prev: &'set Atomic<Node<T>>,
    curr: Shared<'a, Node<T>>,
) -> FindResult<'set, 'g, T> {
    Insert { prev, next: Some(mem::transmute(curr)) }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Node
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
struct Node<T> {
    elem: CacheAligned<T>,
    next: CacheAligned<Atomic<Node<T>>>,
}

impl<T> Node<T> {
    #[inline]
    fn new(elem: T) -> Self {
        Self { elem: CacheAligned(elem), next: CacheAligned(Atomic::null()) }
    }

    #[inline]
    fn elem(&self) -> &T {
        CacheAligned::get(&self.elem)
    }

    #[inline]
    fn next(&self) -> &Atomic<Node<T>> {
        CacheAligned::get(&self.next)
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// FindResult
////////////////////////////////////////////////////////////////////////////////////////////////////

enum FindResult<'set, 'g, T> {
    Found {
        prev: &'set Atomic<Node<T>>,
        curr: Shared<'g, Node<T>>,
        next: Marked<Shared<'g, Node<T>>>,
    },
    Insert {
        prev: &'set Atomic<Node<T>>,
        next: Option<Shared<'g, Node<T>>>,
    },
}
