use std::borrow::Borrow;
use std::cmp::Ordering::{Equal, Greater};
use std::mem;
use std::sync::atomic::Ordering;

use hazptr::reclaim::prelude::*;
use hazptr::typenum;

pub type Atomic<T> = hazptr::Atomic<T, typenum::U1>;
pub type Guarded<T> = hazptr::Guarded<T, typenum::U1>;
pub type Owned<T> = hazptr::Owned<T, typenum::U1>;
pub type Shared<'g, T> = hazptr::Shared<'g, T, typenum::U1>;

/// A linked-list based concurrent ordered set.
#[derive(Debug, Default)]
pub struct OrderedSet<T> {
    head: Atomic<Node<T>>,
}

impl<T> OrderedSet<T>
where
    T: Ord + 'static,
{
    /// Creates an empty ordered set.
    #[inline]
    pub fn new() -> Self {
        Self { head: Atomic::null() }
    }

    /// Inserts a new node for the given `value` and returns `true`, if it did not already exist
    /// in the set.
    #[inline]
    pub fn insert_node(&self, value: T, guards: &mut Guards<T>) -> bool {
        let mut node = Owned::new(Node::new(value));

        let success = loop {
            let iter = self.iter(guards);
            let key = &node.elem;
            if let Ok((pos, next)) = iter.find_insert_position(key) {
                // (ORD:1) this ...
                match pos.compare_exchange(next, node, Ordering::SeqCst, Ordering::SeqCst) {
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

    #[inline]
    pub fn remove_node<Q>(&self, value: &Q, guards: &mut Guards<T>) -> bool
    where
        T: Borrow<Q>,
        Q: Ord,
    {
        let success = loop {
            let iter = self.iter(guards);
            match iter.find_insert_position(value) {
                Ok(_) => break false,
                Err(IterPos { prev, curr, next }) => {
                    if let Some(next) = next {
                        // (ORD:2) this ...
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

                    // (ORD:3) this ...
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
    pub fn get<'g, Q>(&self, value: &Q, guards: &'g mut Guards<T>) -> Option<&'g T>
    where
        T: Borrow<Q>,
        Q: Ord,
    {
        let iter = self.iter(guards);
        match iter.find_insert_position(value) {
            Ok(_) => None,
            Err(IterPos { curr, .. }) => unsafe { Some(&curr.deref().elem) },
        }
    }

    #[inline]
    fn iter<'g, 'set>(&'set self, guards: &'g mut Guards<T>) -> Iter<'g, 'set, T> {
        // (ORD:4) this ...
        let _ = guards.curr.acquire(&self.head, Ordering::SeqCst);

        Iter { head: &self.head, prev: &self.head, old_prev: &self.head, guards }
    }
}

/// A container for the hazard pointers required to safely traverse a hash set.
#[derive(Debug, Default)]
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

#[derive(Debug)]
struct Iter<'g, 'set, T> {
    head: &'set Atomic<Node<T>>,
    prev: &'set Atomic<Node<T>>,
    old_prev: &'set Atomic<Node<T>>,
    guards: &'g mut Guards<T>,
}

macro_rules! retry {
    ($self:ident) => {
        $self.prev = $self.head;
        return Some(Err(IterErr::Retry));
    };
}

const DELETE_TAG: usize = 1;

impl<'g, 'set, T> Iter<'g, 'set, T>
where
    T: 'static,
    'g: 'set,
{
    #[inline]
    fn find_insert_position<Q>(
        mut self,
        insert: &Q,
    ) -> Result<InsertPos<'g, 'set, T>, IterPos<'g, 'set, T>>
    where
        T: Borrow<Q>,
        Q: Ord,
    {
        while let Some(res) = self.next() {
            if let Ok(pos) = res {
                let key = unsafe { pos.curr.deref().elem.borrow() };
                match key.cmp(insert) {
                    Equal => return Err(self.into_iter_pos()),
                    Greater => break,
                    _ => {}
                }
            }
        }

        Ok((self.prev, self.guards.curr.shared()))
    }

    #[inline]
    fn next<'a>(&'a mut self) -> Option<Result<IterPos<'a, 'set, T>, IterErr>> {
        match self.guards.curr.shared() {
            None => None,
            Some(curr) => {
                // it is necessary to dereference the raw pointer here in order to avoid binding the
                // lifetime of `next` to `'a` since it needs to be at least `'set`.
                let ptr = curr.into_marked_non_null();
                let next = unsafe { &(*ptr.decompose_ptr()).next };
                // (ORD:5) this ...
                let unprotected = next.load_unprotected(Ordering::SeqCst);

                // (ORD:6) this ...
                if self
                    .guards
                    .next
                    .acquire_if_equal(next, unprotected.as_marked(), Ordering::SeqCst)
                    .is_err()
                {
                    retry!(self);
                }

                let expected = curr.strip_tag();
                // (ORD:7) this ...
                if self.prev.load_unprotected(Ordering::SeqCst).as_marked() != expected.as_marked()
                {
                    retry!(self);
                }

                if unprotected.tag() == DELETE_TAG {
                    // (ORD:8) this ...
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
                // (ORD:9) this ...
                self.guards.curr.acquire(next, Ordering::SeqCst);

                Some(Ok(IterPos {
                    prev: self.old_prev,
                    curr: self.guards.prev.shared().unwrap(),
                    next: self.guards.next.shared(),
                }))
            }
        }
    }

    #[inline]
    fn into_iter_pos(self) -> IterPos<'g, 'set, T> {
        IterPos {
            prev: self.old_prev,
            curr: self.guards.prev.shared().unwrap(),
            next: self.guards.next.shared(),
        }
    }
}

type InsertPos<'g, 'set, T> = (&'set Atomic<Node<T>>, Option<Shared<'g, Node<T>>>);

#[derive(Debug)]
struct IterPos<'a, 'set, T> {
    prev: &'set Atomic<Node<T>>,
    curr: Shared<'a, Node<T>>,
    next: Option<Shared<'a, Node<T>>>,
}

#[derive(Debug)]
enum IterErr {
    Retry,
}
