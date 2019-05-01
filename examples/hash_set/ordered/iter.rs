use std::borrow::Borrow;
use std::cmp::Ordering::{Equal, Greater};
use std::marker::PhantomData;
use std::mem;
use std::ops::Deref;
use std::sync::atomic::Ordering;

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
                let key = unsafe { pos.curr.deref().elem.borrow() };
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

        match self.guards.curr.acquire(self.prev, Ordering::SeqCst) {
            Some(curr) if !curr.as_marked().is_null() => {
                if curr.tag() == DELETE_TAG {
                    return self.retry();
                }

                let curr_next: &'g Atomic<Node<T>> =
                    unsafe { &(*curr.as_marked().decompose_ptr()).next };

                let next_raw = curr_next.load_raw(Ordering::Relaxed);
                match self.guards.next.acquire_if_equal(curr_next, next_raw, Ordering::SeqCst) {
                    Ok(maybe_next) => {
                        if self.prev.load_raw(Ordering::SeqCst) != curr.strip_tag().as_marked() {
                            return self.retry();
                        }

                        if maybe_next.tag() == DELETE_TAG {
                            match self.prev.compare_exchange(
                                curr.strip_tag(),
                                maybe_next.strip_tag(),
                                Ordering::SeqCst,
                                Ordering::SeqCst,
                            ) {
                                Ok(unlinked) => {
                                    unsafe { unlinked.retire() };
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
                            next: self.guards.next.shared(),
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
            next: self.guards.next.shared(),
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
    pub next: Option<Shared<'g, Node<T>>>,
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Prev
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A type to mimic the fact that the reference to the previous node may have to different lifetimes
/// so the shorter one (`'set`) is chosen, while also ensuring the the hazard pointers (`'g`) remain
/// borrowed as well.
/// Otherwise, a reference that is not the head of the set could be freed when the guard is used to
/// acquire a different value, forfeiting its protection of the reference.
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

/// TODO: Doc...
#[derive(Debug)]
enum IterErr {
    Retry,
    Stalled,
}
