//mod iter;

use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::cmp::Ordering::{Equal, Greater};
use std::mem;
use std::ptr::NonNull;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};

pub type Atomic<T> = hazptr::Atomic<T, typenum::U1>;
pub type Owned<T> = hazptr::Owned<T, typenum::U1>;
pub type Shared<'g, T> = hazptr::Shared<'g, T, typenum::U1>;

use hazptr::reclaim::align::CacheAligned;
use hazptr::reclaim::prelude::{Marked, Protect};
use hazptr::typenum;
use hazptr::Guard;

//use self::iter::{InsertPos, Iter, IterPos};

use self::FindResult::*;

thread_local!(static GUARDS: UnsafeCell<Guards> = UnsafeCell::new(Guards::new()));

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
    pub fn insert_node(&self, value: T) -> bool {
        GUARDS.with(|cell| {
            let guards = unsafe { &mut *cell.get() };

            let mut node = Owned::new(Node::new(value));
            let success = loop {
                let elem = &node.elem;
            };
        });

        unimplemented!()

        /*let mut node = Owned::new(Node::new(value));

        let success = loop {
            let elem = &node.elem;
            if let Ok(InsertPos { prev: pos, next }) =
                Iter::new(&self, guards).find_insert_position(elem)
            {
                node.next.store(next, Relaxed);
                // (ORD:1) this `Release` CAS synchronizes-with the `Acquire` loads (ITE:1), (ITE:2)
                // and the `Acquire` CAS (ORD:2)
                match pos.compare_exchange(next, node, Release, Relaxed) {
                    Ok(_) => break true,
                    Err(failure) => node = failure.input,
                }
            } else {
                break false;
            }
        };

        guards.release_all();
        success*/
    }

    /// Tries to remove a node containing the given `value` from the set and
    /// returns `true`, if the value was found and successfully removed.
    #[inline]
    pub fn remove_node<Q>(&self, value: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Ord,
    {
        /*let success = loop {
            match Iter::new(&self, guards).find_insert_position(value) {
                Ok(_) => break false,
                Err(IterPos { prev, curr, next }) => {
                    let next_marked = Marked::marked(next, DELETE_TAG);
                    // (ORD:2) this `Acquire` CAS synchronizes-with the `Release` CAS (ITE:3),
                    // (ORD:1), (ORD:3)
                    if curr.next.compare_exchange(next, next_marked, Acquire, Relaxed).is_err() {
                        continue;
                    }

                    // (ORD:3) this `Release` CAS synchronizes-with the `Acquire` loads (ITE:1),
                    // (ITE2) and the `Acquire` CAS (ORD:2)
                    match prev.compare_exchange(curr, next, Release, Relaxed) {
                        Ok(unlinked) => unsafe { unlinked.retire() },
                        Err(_) => {
                            let _ = Iter::new(&self, guards).find_insert_position(value);
                        }
                    }

                    break true;
                }
            }
        };

        guards.release_all();
        success*/

        unimplemented!()
    }

    /// Returns a reference to the value in the set, if any, that is equal to
    /// the given `value`.
    #[inline]
    pub fn get<'g, Q>(&self, value: &Q) -> Option<&'g T>
    where
        T: Borrow<Q>,
        Q: Ord,
    {
        GUARDS.with(|cell| {
            let guards = unsafe { &mut *cell.get() };
            match self.iter(guards).find_elem(value) {
                Insert(_) => None,
                Found(IterPos { curr, .. }) => Some(Shared::into_ref(curr).elem()),
            }
        })
    }

    #[inline]
    fn iter<'g>(&self, guards: &'g mut Guards) -> Iter<'_, 'g, T> {
        unimplemented!()
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

////////////////////////////////////////////////////////////////////////////////////////////////////
// Guards
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A container for the three hazard pointers required to safely traverse a hash
/// set.
#[derive(Debug, Default)]
struct Guards {
    prev: Guard,
    curr: Guard,
    next: Guard,
}

impl Guards {
    #[inline]
    fn new() -> Self {
        Self { prev: Guard::new(), curr: Guard::new(), next: Guard::new() }
    }

    /// Releases all contained guards.
    #[inline]
    fn release_all(&mut self) {
        self.prev.release();
        self.curr.release();
        self.next.release();
    }
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
// Iter
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
struct Iter<'set, 'g, T> {
    head: &'set Atomic<Node<T>>,
    guards: &'g mut Guards,
    prev: NonNull<Atomic<Node<T>>>,
    next: NonNull<Atomic<Node<T>>>,
}

impl<'set, 'g, T> Iter<'set, 'g, T>
where
    T: 'static,
    'g: 'set,
{
    fn find_elem<Q>(mut self, elem: &Q) -> FindResult<'set, 'g, T>
    where
        T: Borrow<Q>,
        Q: Ord,
    {
        while let Some(res) = self.next() {
            if let Ok(pos) = res {
                let key = pos.curr.elem().borrow();
                match key.cmp(elem) {
                    Equal => return Found(unsafe { mem::transmute(pos) }),
                    Greater => return Insert(InsertPos { prev: pos.prev, next: Some(pos.curr) }),
                    _ => {}
                }
            }
        }

        Insert(InsertPos { prev: unsafe { &*self.prev.as_ptr() }, next: None })
    }

    fn next<'iter>(&'iter mut self) -> Option<Result<IterPos<'iter, 'iter, T>, IterErr>> {
        unimplemented!()
    }
}

struct IterPos<'set, 'g, T> {
    prev: &'set Atomic<Node<T>>,
    curr: Shared<'g, Node<T>>,
    next: Marked<Shared<'g, Node<T>>>,
}

struct InsertPos<'set, 'g, T> {
    prev: &'set Atomic<Node<T>>,
    next: Option<Shared<'g, Node<T>>>,
}

enum FindResult<'set, 'g, T> {
    Found(IterPos<'set, 'g, T>),
    Insert(InsertPos<'set, 'g, T>),
}

enum IterErr {
    Retry,
    Stalled,
}
