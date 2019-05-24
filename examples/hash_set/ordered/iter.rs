use std::borrow::Borrow;
use std::cmp::Ordering::{Equal, Greater};
use std::marker::PhantomData;
use std::mem;
use std::ops::Deref;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};

use hazptr::reclaim::prelude::*;

use crate::ordered::{Atomic, Guards, OrderedSet, Shared, DELETE_TAG};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Node
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A node in an ordered set.
#[derive(Debug)]
pub struct Node<T> {
    pub elem: T,
    pub next: Atomic<Node<T>>,
}

impl<T> Node<T> {
    /// Creates a new node with no successor.
    #[inline]
    pub fn new(elem: T) -> Self {
        Self { elem, next: Atomic::null() }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Iter
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An Iterator over an [`OrderedSet`].
pub struct Iter<'g, 'set, T> {
    head: &'set Atomic<Node<T>>,
    prev: &'g Atomic<Node<T>>,
    next: &'g Atomic<Node<T>>,
    guards: &'g mut Guards<T>,
}

impl<'g, 'set, T> Iter<'g, 'set, T>
where
    T: 'static,
    'g: 'set,
{
    /// Creates a new `Iter` over the specified `set`.
    #[inline]
    pub fn new(set: &'set OrderedSet<T>, guards: &'g mut Guards<T>) -> Self {
        // this is safe because no references with "faked" lifetimes can escape
        let prev: &'g Atomic<Node<T>> = unsafe { &*(&set.head as *const _) };
        Self { head: &set.head, prev, next: prev, guards }
    }

    /// Consumes the `Iter` and iterates until a position is found, at which the
    /// given `insert` value could be inserted so that the ordering of the set
    /// is kept intact.
    ///
    /// # Errors
    ///
    /// When a value equal to `insert` is already contained in the set, an error
    /// with the position is returned, in which `curr` is the node with the
    /// found value.
    #[inline]
    pub fn find_insert_position<Q>(
        mut self,
        insert: &Q,
    ) -> Result<InsertPos<'g, T>, IterPos<'g, 'set, T>>
    where
        T: Borrow<Q>,
        Q: Ord,
    {
        while let Some(res) = self.next() {
            if let Ok(pos) = res {
                let key = pos.curr.elem.borrow();
                match key.cmp(insert) {
                    Equal => return Err(self.into_iter_pos()),
                    Greater => return Ok((self.prev, self.guards.curr.shared())),
                    _ => {}
                }
            }
        }

        Ok((self.prev, self.guards.curr.shared()))
    }

    fn next(&mut self) -> Option<Result<IterPos<T>, IterErr>> {
        self.prev = self.next;
        mem::swap(&mut self.guards.prev, &mut self.guards.curr);

        // (ITE:1) this `Acquire` load synchronizes-with the `Release` CAS (ITE:3), (ORD:1), (ORD:3)
        match self.prev.load(Acquire, &mut self.guards.curr) {
            None => None,
            Some(curr_marked) => {
                let (curr, curr_tag) = Shared::decompose(curr_marked);
                if curr_tag == DELETE_TAG {
                    return self.retry_err();
                }

                // this extends the lifetime of `&curr.next` to `'g`, which is necessary in order to
                // assign it to `self.next` later
                // the node `curr` is still protected during this entire time, at first by the
                // hazard pointer `guards.curr` and in the next iteration by `guards.prev`
                let curr_next: &'g Atomic<Node<T>> =
                    unsafe { &(*curr.as_marked_ptr().decompose_ptr()).next };
                let next_raw = curr_next.load_raw(Relaxed);

                // (ITE:2) this `Acquire` load synchronizes-with the the `Release` CAS (ITE:3),
                // (ORD:1), (ORD:3)
                match self.guards.next.acquire_if_equal(curr_next, next_raw, Acquire) {
                    Ok(next_marked) => {
                        if self.prev.load_raw(Relaxed) != curr.as_marked_ptr() {
                            return self.retry_err();
                        }

                        let (next, next_tag) = Marked::decompose(next_marked);
                        if next_tag == DELETE_TAG {
                            // (ITE:3) this `Release` CAS synchronizes-with the `Acquire` loads
                            // (ITE:1), (ITE:2) and the `Acquire` CAS (ORD:2)
                            match self.prev.compare_exchange(curr, next, Release, Relaxed) {
                                Ok(unlinked) => {
                                    unsafe { unlinked.retire() };
                                    mem::swap(&mut self.guards.prev, &mut self.guards.curr);

                                    return Some(Err(IterErr::Stalled));
                                }
                                Err(_) => return self.retry_err(),
                            };
                        }

                        self.next = curr_next;

                        Some(Ok(IterPos {
                            prev: Prev::from(self.prev),
                            curr: self.guards.curr.shared().unwrap_or_else(|| unreachable!()),
                            next: self.guards.next.marked(),
                        }))
                    }
                    _ => self.retry_err(),
                }
            }
        }
    }

    /// Consumes the `Iter` and returns its current position.
    #[inline]
    fn into_iter_pos(self) -> IterPos<'g, 'set, T> {
        IterPos {
            prev: Prev::from(self.prev),
            curr: self.guards.curr.shared().unwrap_or_else(|| unreachable!()),
            next: self.guards.next.marked(),
        }
    }

    #[inline]
    fn retry_err(&mut self) -> Option<Result<IterPos<T>, IterErr>> {
        // this is safe because no references with "faked" lifetimes can escape
        self.next = unsafe { &*(self.head as *const _) };
        Some(Err(IterErr::Retry))
    }
}

/// A position of a set iterator.
#[derive(Copy, Clone, Debug)]
pub struct IterPos<'g, 'set, T> {
    pub prev: Prev<'g, 'set, T>,
    pub curr: Shared<'g, Node<T>>,
    pub next: Marked<Shared<'g, Node<T>>>,
}

/// A position in a set consisting of a `prev` reference and a `next` node,
/// in between which a new node can be inserted.
type InsertPos<'g, T> = (&'g Atomic<Node<T>>, Option<Shared<'g, Node<T>>>);

////////////////////////////////////////////////////////////////////////////////////////////////////
// Prev
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A type to mimic the fact that the reference to the previous node may have
/// two different lifetimes, so the shorter one (`'set`) is chosen, while also
/// ensuring the the hazard pointers (`'g`) remain borrowed as well.
/// Otherwise, a reference that is not the head of the set could be freed when
/// the guard is used to acquire a different value, forfeiting its protection of
/// the reference.
#[derive(Copy, Clone, Debug)]
pub struct Prev<'g, 'set, T> {
    inner: &'set Atomic<Node<T>>,
    _marker: PhantomData<&'g Guards<T>>,
}

impl<'g, 'set, T> Deref for Prev<'g, 'set, T>
where
    'g: 'set,
{
    type Target = Atomic<Node<T>>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner
    }
}

impl<'g, 'set, T> From<&'g Atomic<Node<T>>> for Prev<'g, 'set, T>
where
    'g: 'set,
{
    #[inline]
    fn from(prev: &'g Atomic<Node<T>>) -> Self {
        Self { inner: prev, _marker: PhantomData }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// IterErr
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
enum IterErr {
    Retry,
    Stalled,
}
