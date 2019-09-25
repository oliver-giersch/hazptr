# Ideas

- store `Hazard`s in segmented (array-based) list: Each `HazardNode` contains 31 128-byte aligned `Hazards` and one 128-byte
  aligned `next` pointer -> might improve iteration 

# API Redesign

Currently, only global (as in `static`) data structures can be used. Allowing data-structure specific sets of hazard pointers
and garbage heaps has some advantages, such as more focused iteration (i.e. only iterating HPs that actually belong to the
same data-structure. Likewise, this may give more flexibility for later adding custom allocator support. Adding support for
**policies** is also advantageous.

## Policies

Different policies for choosing garbage collection strategies:

Global:

```rust
enum GlobalPolicy {
    LocalGarbage(AbandonList),
    GlobalGarbage(GarbageList),
}
```

Local:

```rust
enum LocalPolicy {
    LocalGarbage(Vec<Retired>, ...),
    GlobalGarbage,
}
```

(use runtime checks to assert matching policies of associated globals and locals)

## Global

Globals must no longer be static, but can have any lifetime

```rust
pub struct Global<A: Alloc> {
    hazards: HazardList<A>,
    policy: GlobalPolicy,
    alloc: A,
}
```

## Local

Locals contain an explicit reference to their associated `Global`, with which they must have matching policies.

```rust
struct LocalInner<'a, A: Alloc> {
    config: Config,
    global: &'a Global<A>,
    policy: LocalPolicy,
    guard_count: u32,
    ops_count: u32,
    scan_cache: Vec<Protected, A>,
}
```

## Guards

Guards must necessarily be restricted by the lifetime of their associated `Global`, the `guard_count` field in `LocalInner`
ensures the lifetime of `Local` will be long enough:

```rust
pub struct Guard<'a, A: Alloc> {
    local: *const Local<'a, A>, // this must be a pointer this since references into std TLS are not allowed
    hazard: &'a Hazard,
}
```

### Alternative

Instead of storing references to the associated `Local` in a pointer, the `LocalAccess` trait could see continoued usage
as a means for abstracting over access through TLS or through a safe `&'a Local`, this would change the signature of `Guard`
to potentially express to lifetimes `'global` and `'local`.

## Retiring Records

It would now be possible to protect pointers with hazard pointers belonging to one `Global` and retiring records in a cache
that is checked against the hazard pointers of **another** `Global`. There is no obvious way to prevent this on a type-level
and additional runtime checks would likely have to be extensive.
