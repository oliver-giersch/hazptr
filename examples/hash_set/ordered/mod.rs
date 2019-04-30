use std::borrow::Borrow;
use std::sync::atomic::Ordering;

pub type Atomic<T> = hazptr::Atomic<T, typenum::U1>;
pub type Guarded<T> = hazptr::Guarded<T, typenum::U1>;
pub type Owned<T> = hazptr::Owned<T, typenum::U1>;
pub type Shared<'g, T> = hazptr::Shared<'g, T, typenum::U1>;

use hazptr::reclaim::prelude::*;
use hazptr::reclaim::MarkedPtr;
use hazptr::typenum;

mod iter;

use self::iter::{Iter, IterPos, Node};

const DELETE_TAG: usize = 1;

/// A linked-list based concurrent ordered set.
#[derive(Debug, Default)]
pub struct OrderedSet<T> {
    head: Atomic<Node<T>>,
}

impl<T> OrderedSet<T>
where
    T: Ord + 'static,
{
    /// Inserts a new node for the given `value` and returns `true`, if it did not already exist in
    /// the set.
    #[inline]
    pub fn insert_node(&self, value: T, guards: &mut Guards<T>) -> bool {
        let mut node = Owned::new(Node::new(value));

        let success = loop {
            let elem = &node.elem;
            if let Ok((pos, next)) = Iter::new(&self, guards).find_insert_position(elem) {
                node.next.store(next.strip_tag(), Ordering::Relaxed);
                // (ORD:2) this `Release` CAS synchronizes-with ...
                match pos.compare_exchange(
                    next.strip_tag(),
                    node,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                ) {
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
    pub fn remove_node<Q>(&self, value: &Q, guards: &mut Guards<T>) -> bool
    where
        T: Borrow<Q>,
        Q: Ord,
    {
        let success = loop {
            match Iter::new(&self, guards).find_insert_position(value) {
                Err(IterPos { prev, curr, next }) => {
                    let delete_marker =
                        next.map(|next| Shared::with_tag(next, DELETE_TAG)).unwrap_or(unsafe {
                            Shared::from_marked(MarkedPtr::new(DELETE_TAG as *mut _))
                        });

                    if unsafe { &curr.deref().next }
                        .compare_exchange(
                            next.strip_tag(),
                            delete_marker,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        )
                        .is_err()
                    {
                        continue;
                    }

                    match prev.compare_exchange(
                        Shared::with_tag(curr, 0),
                        next.strip_tag(),
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        Ok(unlinked) => unsafe { unlinked.retire() }, // <-- uncomment this line and the use-after-free errors stop ???
                        _ => {
                            let _ = Iter::new(&self, guards).find_insert_position(value);
                        }
                    }

                    break true;
                }
                _ => break false,
            }
        };

        guards.release_all();
        success
    }

    /// TODO: Doc...
    #[inline]
    pub fn get<'g, Q>(&self, value: &Q, guards: &'g mut Guards<T>) -> Option<&'g T>
    where
        T: Borrow<Q>,
        Q: Ord,
    {
        let iter = Iter::new(&self, guards);
        match iter.find_insert_position(value) {
            Ok(_) => None,
            Err(IterPos { curr, .. }) => unsafe { Some(&curr.deref().elem) },
        }
    }
}

impl<T> Drop for OrderedSet<T> {
    #[inline]
    fn drop(&mut self) {
        let mut node = self.head.take();

        while let Some(mut curr) = node {
            // must not transform invalid value into an Option<Owned> (likely UB)
            if curr.next.load_raw(Ordering::Relaxed).into_usize() == DELETE_TAG {
                return;
            } else {
                node = curr.next.take();
            }
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
    /// TODO: Doc...
    #[inline]
    pub fn new() -> Self {
        Self { prev: Guarded::new(), curr: Guarded::new(), next: Guarded::new() }
    }

    /// TODO: Doc...
    #[inline]
    pub fn release_all(&mut self) {
        self.prev.release();
        self.curr.release();
        self.next.release();
    }
}