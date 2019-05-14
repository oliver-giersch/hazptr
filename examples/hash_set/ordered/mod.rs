use std::borrow::Borrow;
use std::mem;
use std::sync::atomic::Ordering::{AcqRel, Relaxed, Release};

pub type Atomic<T> = hazptr::Atomic<T, typenum::U1>;
pub type Guarded<T> = hazptr::Guarded<T, typenum::U1>;
pub type Owned<T> = hazptr::Owned<T, typenum::U1>;
pub type Shared<'g, T> = hazptr::Shared<'g, T, typenum::U1>;
type Unlinked<T> = hazptr::Unlinked<T, typenum::U1>;

use hazptr::reclaim::prelude::*;
use hazptr::typenum;

mod iter;

use self::iter::{Iter, IterPos, Node};

const DELETE_TAG: usize = 1;

/// A concurrent linked-list based ordered set.
#[derive(Debug, Default)]
pub struct OrderedSet<T> {
    head: Atomic<Node<T>>,
}

impl<T> OrderedSet<T>
where
    T: Ord + 'static,
{
    /// Inserts a new node for the given `value` and returns `true`, if it did
    /// not already exist in the set.
    #[inline]
    pub fn insert_node(&self, value: T, guards: &mut Guards<T>) -> bool {
        let mut node = Owned::new(Node::new(value));

        let success = loop {
            let elem = &node.elem;
            if let Ok((pos, next)) = Iter::new(&self, guards).find_insert_position(elem) {
                let next_clear = MarkedPointer::clear_tag(next);
                node.next.store(next_clear, Relaxed);
                // (ORD:1) this `Release` CAS synchronizes-with the `Acquire` loads (ITE:1), (ITE:2)
                // and the `Acquire` CAS (ORD:2)
                match pos.compare_exchange(next_clear, node, Release, Relaxed) {
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
    pub fn remove_node<Q>(&self, value: &Q, guards: &mut Guards<T>) -> bool
    where
        T: Borrow<Q>,
        Q: Ord,
    {
        let success = loop {
            match Iter::new(&self, guards).find_insert_position(value) {
                Ok(_) => break false,
                Err(IterPos { prev, curr, next }) => {
                    let next_clear = Marked::clear_tag(next);
                    let delete_marker = Marked::with_tag(next, DELETE_TAG);
                    if curr
                        .next
                        .compare_exchange(next_clear, delete_marker, Relaxed, Relaxed)
                        .is_err()
                    {
                        continue;
                    }

                    // (ORD:3) this `AcqRel` CAS synchronizes-with the `Acquire` loads (ITE:1),
                    // (ITE2) and the `Release` CAS (ITE:3), (ORD:1)
                    match prev.compare_exchange(
                        Shared::clear_tag(curr),
                        next_clear,
                        AcqRel,
                        Relaxed,
                    ) {
                        Ok(unlinked) => unsafe { Unlinked::retire(unlinked) },
                        Err(_) => {
                            let _ = Iter::new(&self, guards).find_insert_position(value);
                        }
                    }

                    break true;
                }
            }
        };

        guards.release_all();
        success
    }

    /// Returns a reference to the value in the set, if any, that is equal to the given `value`.
    #[inline]
    pub fn get<'g, Q>(&self, value: &Q, guards: &'g mut Guards<T>) -> Option<&'g T>
    where
        T: Borrow<Q>,
        Q: Ord,
    {
        match Iter::new(&self, guards).find_insert_position(value) {
            Ok(_) => None,
            Err(IterPos { curr, .. }) => Some(&curr.into_ref().elem),
        }
    }
}

impl<T> Drop for OrderedSet<T> {
    #[inline]
    fn drop(&mut self) {
        let mut node = self.head.take();

        while let Some(mut curr) = node {
            node = curr.next.take();
            mem::drop(curr);
        }
    }
}

/// A container for the three hazard pointers required to safely traverse a hash set.
#[derive(Debug, Default)]
pub struct Guards<T> {
    prev: Guarded<Node<T>>,
    curr: Guarded<Node<T>>,
    next: Guarded<Node<T>>,
}

impl<T> Guards<T> {
    /// Creates a new guard container.
    #[inline]
    pub fn new() -> Self {
        Self { prev: Guarded::new(), curr: Guarded::new(), next: Guarded::new() }
    }

    /// Releases all contained guards.
    #[inline]
    pub fn release_all(&mut self) {
        self.prev.release();
        self.curr.release();
        self.next.release();
    }
}
