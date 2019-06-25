use std::borrow::Borrow;
use std::cmp::Ordering::{Equal, Greater};
use std::marker::PhantomData;
use std::mem;
use std::ptr::NonNull;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};

use hazptr::reclaim::prelude::*;
use hazptr::typenum::U1;

use crate::ordered::{Atomic, Guards, Node, OrderedSet, Shared, DELETE_TAG};

type MarkedNonNull<T> = hazptr::reclaim::MarkedNonNull<T, U1>;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Iter
////////////////////////////////////////////////////////////////////////////////////////////////////

/// An Iterator over an [`OrderedSet`].
pub(super) struct Iter<'set, T> {
    head: &'set Atomic<Node<T>>,
    prev: NonNull<Atomic<Node<T>>>,
    next: NonNull<Atomic<Node<T>>>,
}

impl<'set, T> Iter<'set, T>
where
    T: 'static,
{
    /// Creates a new `Iter` over the specified `set`.
    #[inline]
    pub fn new(set: &'set OrderedSet<T>) -> Self {
        let prev = NonNull::from(&set.head);
        Self { head: &set.head, prev, next: prev }
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
    pub fn find_insert_position<'g, Q>(
        mut self,
        insert: &Q,
        guards: &'g mut Guards,
    ) -> Result<InsertPos<'g, 'set, T>, IterPos<'g, 'set, T>>
    where
        T: Borrow<Q>,
        Q: Ord,
    {
        /*let a = self.next().unwrap().unwrap();
        let b = self.next().unwrap().unwrap();
        let _ = self.next();

        let c = a.curr.elem();
        println!("{}", c as *const T as usize);*/

        while let Some(res) = self.next(guards) {
            if let Ok(pos) = res {
                let key = pos.curr.elem().borrow();
                match key.cmp(insert) {
                    Equal => return Err(pos),
                    Greater => return Ok(InsertPos { prev: pos.prev, next: Some(pos.curr) }),
                    _ => {}
                }
            }
        }

        Ok(InsertPos { prev: Prev::from(unsafe { &*self.prev.as_ptr() }), next: None })
    }

    fn next<'g, 'iter>(
        &'iter mut self,
        guards: &'g mut Guards,
    ) -> Option<Result<IterPos<'g, 'iter, T>, IterErr>> {
        self.prev = self.next;
        mem::swap(&mut guards.prev, &mut guards.curr);

        let prev = unsafe { self.prev.as_ref() };
        // (ITE:1) this `Acquire` load synchronizes-with the `Release` CAS (ITE:3), (ORD:1), (ORD:3)
        match prev.load(Acquire, &mut guards.curr) {
            None => None,
            Some(curr_marked) => {
                let (curr, curr_tag) = Shared::decompose(curr_marked);
                if curr_tag == DELETE_TAG {
                    return self.retry_err();
                }

                let curr_next = curr.next();
                let next_raw = curr_next.load_raw(Relaxed);

                // (ITE:2) this `Acquire` load synchronizes-with the the `Release` CAS (ITE:3),
                // (ORD:1), (ORD:3)
                match curr_next.load_marked_if_equal(next_raw, Acquire, &mut guards.next) {
                    Ok(next_marked) => {
                        if prev.load_raw(Relaxed) != curr.as_marked_ptr() {
                            return self.retry_err();
                        }

                        let (next, next_tag) = Marked::decompose(next_marked);
                        if next_tag == DELETE_TAG {
                            // (ITE:3) this `Release` CAS synchronizes-with the `Acquire` loads
                            // (ITE:1), (ITE:2) and the `Acquire` CAS (ORD:2)
                            match prev.compare_exchange(curr, next, Release, Relaxed) {
                                Ok(unlinked) => {
                                    unsafe { unlinked.retire() };
                                    mem::swap(&mut self.guards.prev, &mut self.guards.curr);

                                    return Some(Err(IterErr::Stalled));
                                }
                                Err(_) => return self.retry_err(),
                            };
                        }

                        self.next = NonNull::from(curr_next);

                        Some(Ok(IterPos {
                            prev: NonNull::from(prev),
                            curr: Shared::into_marked_non_null(curr),
                            next: Value(Shared::into_marked_non_null(next)),
                            _marker: PhantomData,
                        }))
                    }
                    Err(_) => self.retry_err(),
                }
            }
        }
    }

    #[inline]
    fn retry_err(&mut self) -> Option<Result<IterPos<T>, IterErr>> {
        self.next = NonNull::from(self.head);
        Some(Err(IterErr::Retry))
    }
}

/// A position of a set iterator.
#[derive(Copy, Clone, Debug)]
pub(super) struct IterPos<'g, 'set, T> {
    prev: &'set Atomic<Node<T>>,
    curr: Shared<'g, Node<T>>,
    next: Marked<Shared<'g, Node<T>>>,
}

/// A position in a set consisting of a `prev` reference and a `next` node,
/// in between which a new node can be inserted.
#[derive(Copy, Clone, Debug)]
pub(super) struct InsertPos<'g, 'set, T> {
    prev: &'set Atomic<T>,
    next: Option<Shared<'g, Node<T>>>,
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// IterErr
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
enum IterErr {
    Retry,
    Stalled,
}
