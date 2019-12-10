////////////////////////////////////////////////////////////////////////////////////////////////////
// Policy (trait)
////////////////////////////////////////////////////////////////////////////////////////////////////

pub trait Policy: Sync + 'static {
    type Header: Default + Sync + Sized;
    type GlobalState: Default + Send + Sync;
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

#[derive(Copy, Clone, Debug, Eq, Ord, Hash, PartialEq, PartialOrd)]
pub(crate) struct AnyNodePtr(*const dyn AnyNode);

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
