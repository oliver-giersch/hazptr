use crate::{HPHandle, Record};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Policy (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

pub trait Policy {
    type Header: Default + Sync + Sized;
    type GlobalState;
    type LocalState;
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// GlobalRetire
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct GlobalRetire;

/********** impl Policy ***************************************************************************/

impl Policy for GlobalRetire {
    type Header = AnyNodePtr;
    type GlobalState = (); // Queue<Retired>, ...
    type LocalState = ();
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// LocalRetire
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct LocalRetire;

/********** impl Policy ***************************************************************************/

impl Policy for LocalRetire {
    type Header = ();
    type GlobalState = (); // AbandonedBags, ...
    type LocalState = (); // Vec<Retired>, ...
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

#[derive(Copy, Clone, Debug, Default, Eq, Ord, Hash, PartialEq, PartialOrd)]
pub(crate) struct AnyNodePtr(*const dyn AnyNode);

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

type Record<T> = conquer_reclaim::Record<T, HPHandle<GlobalRetire>>;

impl<T> AnyNode for Record<T> {
    #[inline]
    fn next(&self) -> AnyNodePtr {
        *self.header()
    }
}

#[derive(Default)]
#[repr(C)]
struct Header {
    data: *mut (),
    vtable: *mut (),
    next: *mut DynNode,
}
