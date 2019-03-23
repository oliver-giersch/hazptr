use std::mem;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicPtr, Ordering};

////////////////////////////////////////////////////////////////////////////////////////////////////
/// RetiredBag
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A list for caching reclaimed records before they can be finally dropped/deallocated.
///
/// This type also functions as potential list node for the global list of abandoned bags.
/// The internal cache uses a `Vec`, which will have to be reallocated if too many retired records
/// are cached at any time.
pub struct RetiredBag {
    pub inner: Vec<Retired>,
    next: Option<NonNull<RetiredBag>>,
}

impl RetiredBag {
    const DEFAULT_CAPACITY: usize = 256;

    /// Creates a new `RetiredBag` with default capacity for retired records.
    #[inline]
    pub fn new() -> Self {
        Self {
            inner: Vec::with_capacity(Self::DEFAULT_CAPACITY),
            next: None,
        }
    }

    #[inline]
    pub fn merge(&mut self, mut other: Vec<Retired>) {
        // swap bags if the other one is substantially larger and thus able to fit more records
        // before reallocating, potentially avoiding/deferring such reallocations
        if (other.capacity() - other.len()) > self.inner.capacity() {
            mem::swap(&mut self.inner, &mut other);
        }

        self.inner.append(&mut other);
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
/// AbandonedBags
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct AbandonedBags {
    head: AtomicPtr<RetiredBag>,
}

impl AbandonedBags {
    #[inline]
    pub fn new() -> Self {
        Self {
            head: AtomicPtr::new(ptr::null_mut()),
        }
    }

    #[inline]
    pub fn push(&self, abandoned: Box<RetiredBag>) {
        let leaked = Box::leak(abandoned);

        loop {
            let head = self.head.load(Ordering::Relaxed);
            // this is safe because all nodes are backed by valid leaked allocations (Box)
            leaked.next = unsafe { head.as_mut().map(NonNull::from) };

            // (RET:1) this `Release` CAS synchronizes-with the `Acquire` swap in (RET:2)
            if self
                .head
                .compare_exchange_weak(head, leaked, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }

    #[inline]
    pub fn take_and_merge(&self) -> Option<Box<RetiredBag>> {
        // this avoids the CAS if the stack is empty
        if self.head.load(Ordering::Relaxed).is_null() {
            return None;
        }

        // this is safe because all nodes are backed by valid leaked allocations (Box)
        // (RET:2) this `Acquire` swap synchronizes-with the `Release` CAS in (RET:1)
        let stack = unsafe { self.head.swap(ptr::null_mut(), Ordering::Acquire).as_mut() };
        stack.map(|bag| {
            // this is safe because all nodes are backed by valid leaked allocations (Box)
            let mut boxed = unsafe { Box::from_raw(bag) };

            let mut curr = boxed.next;
            while let Some(ptr) = curr {
                // this is safe because all nodes are backed by valid leaked allocations (Box)
                let RetiredBag { inner: bag, next } = unsafe { *Box::from_raw(ptr.as_ptr()) };
                boxed.merge(bag);
                curr = next;
            }

            boxed
        })
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
/// Retired
////////////////////////////////////////////////////////////////////////////////////////////////////

/// A (fat) pointer to a retired record that has not yet been reclaimed and deallocated.
pub struct Retired {
    record: NonNull<dyn Any + 'static>,
}

impl Retired {
    /// Creates a new `Retired` record from a raw (unmarked) pointer of arbitrary type.
    ///
    /// # Safety
    ///
    /// The caller has to ensure the given `record` points to
    /// The record will be dropped at an unspecified time, which means it may potentially outlive
    /// its any (non-static) lifetime. Since the record will be only dropped after retirement, this
    /// is safe as long as the `Drop` implementation does not access any non-static references.
    #[inline]
    pub unsafe fn new_unchecked<'a, T: 'a>(record: NonNull<T>) -> Self {
        // transmuting the lifetime is sound as long as the `Drop` impl does not access any
        // non-static references, which has to be ensured by the caller
        let any: NonNull<dyn Any + 'a> = record;
        let any: NonNull<dyn Any + 'static> = mem::transmute(any);
        Self { record: any }
    }

    /// Gets the memory address of the retired record.
    #[inline]
    pub fn address(&self) -> usize {
        // casts to thin pointer first
        self.record.as_ptr() as *mut () as usize
    }
}

impl Drop for Retired {
    #[inline]
    fn drop(&mut self) {
        // this is safe since the HP reclamation scheme does not require any additional information
        // per allocated record
        mem::drop(unsafe { Box::from_raw(self.record.as_ptr()) });
    }
}

trait Any {}
impl<T> Any for T {}
