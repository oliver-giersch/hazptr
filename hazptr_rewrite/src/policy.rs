use conquer_reclaim::RawRetired;

use core::fmt::Debug;

////////////////////////////////////////////////////////////////////////////////////////////////////
// Policy (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

pub trait Policy: Debug + Sync + 'static {
    type Header: Default + Sync + Sized;
    type GlobalState: Debug + Default + Send + Sync;
    type LocalState: Debug + Default;

    fn on_thread_exit(local: Self::LocalState, global: &Self::GlobalState);
    unsafe fn retire(local: &mut Self::LocalState, global: &Self::GlobalState) {}
    
    // fn on_thread_exit(mut local: Self::LocalState, global: &Self::GlobalState);
    // unsafe fn retire_record(local: &mut Self::LocalState, global: &Self::GlobalState, retired: RawRetired);
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalRetire
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct GlobalRetire;

/********** impl Policy ***************************************************************************/

impl Policy for GlobalRetire {
    type Header = AnyNodePtr;
    type GlobalState = (); // Queue<Retired>, ...
    type LocalState = ();

    #[inline]
    fn on_thread_exit(local: Self::LocalState, global: &Self::GlobalState) {
        unimplemented!()
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalRetire
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct LocalRetire;

/********** impl Policy ***************************************************************************/

impl Policy for LocalRetire {
    type Header = ();
    type GlobalState = (); // AbandonedBags, ...
    type LocalState = Box<Vec<RawRetired>>;

    #[inline]
    fn on_thread_exit(local: Self::LocalState, global: &Self::GlobalState) {
        unimplemented!()
    }
}

// Queue<DynAnyRecord>
// impl Node for DynAnyRecord
// Q.take_all(..)
// iter -> *mut DynAnyRecord (better: *mut dyn AnyRecord)

// Unlinked<T, N, HP<P>> -> retire
// *mut Record<T, HP<P>> -> *mut DynAnyRecord -> insert in Queue
// (in reclaiming thread)
// *mut DynAnyRecord -> cast to *mut dyn AnyRecord
// deref to &dyn AnyRecord -> as_protected() -> reference with hazard ptrs
// Box::from_raw(..) + drop / don't reclaim
// check next (*mut DynAnyRecord) ...

////////////////////////////////////////////////////////////////////////////////////////////////////
// AnyNodePtr
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug, Eq, Ord, Hash, PartialEq, PartialOrd)]
pub struct AnyNodePtr(*const dyn AnyNode);

/********** impl Default **************************************************************************/

impl Default for AnyNodePtr {
    fn default() -> Self {
        unimplemented!()
    }
}

unsafe impl Sync for AnyNodePtr {}

////////////////////////////////////////////////////////////////////////////////////////////////////
// AnyNode (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

struct DynNode {
    data: *mut (),
    vtable: *mut (),
}

impl DynNode {
    fn from_ptr(ptr: *mut dyn AnyNode) -> Self {
        unimplemented!()
    }

    fn into_dyn_ptr(self) -> *mut dyn AnyNode {
        unimplemented!()
    }
}

trait AnyNode {
    fn next(&self) -> AnyNodePtr;
}

/*impl<T> AnyNode for Record<T> {
    #[inline]
    fn next(&self) -> AnyNodePtr {
        *self.header()
    }
}*/

#[repr(C)]
struct Header {
    data: *mut (),
    vtable: *mut (),
    next: *mut DynNode,
}

impl Default for Header {
    fn default() -> Self {
        unimplemented!()
    }
}
