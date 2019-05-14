use std::borrow::Borrow;
use std::cmp::Ordering::{Equal, Greater};
use std::marker::PhantomData;
use std::mem;
use std::ops::Deref;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};

use hazptr::reclaim::prelude::*;

use crate::ordered::{Atomic, Guards, OrderedSet, Shared, Unlinked, DELETE_TAG};
use reclaim::MarkedPtr;

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

/// TODO: Doc...
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
    /// TODO: Doc...
    #[inline]
    pub fn new(set: &'set OrderedSet<T>, guards: &'g mut Guards<T>) -> Self {
        // this is safe because no references with "faked" lifetimes can escape
        let prev: &'g Atomic<Node<T>> = unsafe { &*(&set.head as *const _) };
        Self { head: &set.head, prev, next: prev, guards }
    }

    /// TODO: Doc...
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

    /// TODO: Doc...
    fn next(&mut self) -> Option<Result<IterPos<T>, IterErr>> {
        self.prev = self.next;
        mem::swap(&mut self.guards.prev, &mut self.guards.curr);

        // (ITE:1) this `Acquire` load synchronizes-with the `Release` CAS (ITE:3),
        match self.guards.curr.acquire(self.prev, Acquire) {
            Value(curr) => {
                let (curr_ptr, curr_tag) = Shared::as_marked_ptr(&curr).decompose();
                if curr_tag == DELETE_TAG {
                    return self.retry();
                }

                let curr_next: &'g Atomic<Node<T>> = unsafe { &(*curr_ptr).next };
                let next_raw = curr_next.load_raw(Relaxed);
                // (ITE:2) this `Acquire` load synchronizes-with the ...
                match self.guards.next.acquire_if_equal(curr_next, next_raw, Acquire) {
                    Ok(maybe_next) => {
                        // FIXME (reclaim): Could be nicer...
                        if self.prev.load_raw(Relaxed) != MarkedPtr::compose(curr_ptr, 0) {
                            return self.retry();
                        }

                        // FIXME (reclaim): Marked should have an inherent decompose_tag method
                        if Marked::as_marked_ptr(&maybe_next).decompose_tag() == DELETE_TAG {
                            // (ITE:3) this `Release` CAS synchronizes-with the `Acquire` loads
                            // (ITE:1), (ITE:2) and the `Acquire` CAS (ORD:2)
                            match self.prev.compare_exchange(
                                Shared::clear_tag(curr),
                                Marked::clear_tag(maybe_next),
                                Release,
                                Relaxed,
                            ) {
                                Ok(unlinked) => {
                                    unsafe { Unlinked::retire(unlinked) };
                                    mem::swap(&mut self.guards.prev, &mut self.guards.curr);

                                    return Some(Err(IterErr::Stalled));
                                }
                                _ => self.retry(),
                            };
                        }

                        self.next = curr_next;

                        Some(Ok(IterPos {
                            prev: Prev::from(self.prev),
                            curr: self.guards.curr.shared().unwrap_or_else(|| unreachable!()),
                            next: self.guards.next.marked(),
                        }))
                    }
                    _ => self.retry(),
                }
            }
            _ => None,
        }
    }

    /// TODO: Doc...
    #[inline]
    fn into_iter_pos(self) -> IterPos<'g, 'set, T> {
        IterPos {
            prev: Prev::from(self.prev),
            curr: self.guards.curr.shared().unwrap_or_else(|| unreachable!()),
            next: self.guards.next.marked(),
        }
    }

    /// TODO: Doc...
    #[inline]
    fn retry(&mut self) -> Option<Result<IterPos<T>, IterErr>> {
        // this is safe because no references with "faked" lifetimes can escape
        self.next = unsafe { &*(self.head as *const _) };
        Some(Err(IterErr::Retry))
    }
}

/// TODO: Doc...
type InsertPos<'g, T> = (&'g Atomic<Node<T>>, Option<Shared<'g, Node<T>>>);

/// TODO: Doc...
#[derive(Debug)]
pub struct IterPos<'g, 'set, T> {
    pub prev: Prev<'g, 'set, T>,
    pub curr: Shared<'g, Node<T>>,
    pub next: Marked<Shared<'g, Node<T>>>,
}

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
