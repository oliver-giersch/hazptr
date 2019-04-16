# Hazptr

Hazard pointer based concurrent memory reclamation.

[![Latest version](https://img.shields.io/crates/v/hazptr.svg)](https://crates.io/crates/hazptr)
[![Documentation](https://docs.rs/hazptr/badge.svg)](https://docs.rs/hazptr)
![License](https://img.shields.io/crates/l/hazptr.svg)

Whenever a thread reads a value from shared memory it also protects the loaded value with a globally visible
*hazard pointer*.
All threads can *retire* shared values that are no longer needed and accessible and cache them locally.
Retired records are reclaimed (dropped and de-allocated) in bulk, but only when they are not or no longer protected
by any hazard pointers.

## Usage

Add this to your `Cargo.toml`

```
[dependencies]
hazptr = "0.1"
```

## Minimum Supported Rust Version (MSRV)

The minimum supported rust version for this crate is 1.36.0

## Comparison with [crossbeam-epoch](https://crates.io/crates/crossbeam-epoch)

The hazard pointer reclamation scheme is generally less efficient then epoch-based reclamation schemes (or any other
type of reclamation scheme for that matter).
This is mainly because acquisition of hazard pointers requires an expensive memory fence to be issued after every load
from shared memory.
It is, however, usually the best scheme in terms of reclamation reliability.
Retired records are generally reclaimed in a timely manner and reclamation is not affected by contention.
These properties can lead to a better memory footprint of applications using hazard pointers instead of other
reclamation schemes.
Also, since hazard pointers only protect individual pointers from reclamation, they can be better suited for protecting
individual records for long periods of time.
Epoch-based schemes, on the other hand, completely prevent reclamation by all threads whenever records need to be
protected.

## Examples

See [examples/treiber/stack.rs](examples/treiber/stack.rs) for an implementation of Treiber's stack with hazard
pointers or [examples/hash_set.rs](examples/hash_set.rs) for an implementation of a concurrent hash set.

## Features

The following features are defined for this crate:

- `count-release`
- `maximum-reclamation-freq`
- `reduced-reclamation-freq`

By default, a thread initiates a GC scan and attempts to flush its cache of retired records, once it has retired a
certain threshold count of records.
By compiling the crate with the `count-release` feature, this can be changed to count the instances of successfully
acquired hazard pointers (`Guarded`) going out of scope (i.e. being released) instead.
This can be beneficial when only few records are involved overall and retiring of records is rare.

The `maximum-reclamation-freq` and `reduced-reclamation-freq` features are **mutually exclusive** and affect the
threshold that controls how often GC scans are started.
With maximum reclamation frequency, a GC scan is initiated after **every** operation counting towards the threshold,
i.e either retiring records or releasing acquired hazard pointers (depending on the selected feature).
The reduced setting leads to less frequent scans compared to the default setting when no feature is selected.

Generally, a lower reclamation frequency is better performance-wise, but could lead to the accumulation of large amounts
of retired but unreclaimed records (i.e. garbage).

### Usage in `#[no_std]` environments

...

## Down the Road

In its current state, `hazptr` requires two separate `std` features, meaning it is not suitable for `#[no_std]`
environments.
Specifically, these features are:

- automatic management of thread local storage (the `thread_local!` macro)
- a global allocator

Future developments may include relaxing these restrictions and adding support for custom allocators.

## Glossary

This crate uses a certain terminology for describing common entities in the context of concurrent memory reclamation:

- ### record

  Records are heap allocated values or data structures which are managed by a concurrent
  reclamation scheme.

- ### reclaim

  Reclamation describes the process of collecting and freeing previously **retired**
  garbage.
  This includes both **dropping** the type and de-allocating the (heap allocated) memory.

- ### retire

  After a record is unlinked from a concurrent collection or data structure and is no
  longer accessible it can be safely **retired**.
  This marks the record as garbage to be collected (**reclaimed**) later.

- ### unlink

### The Record Lifecycle

```
allocate --> insert --> reference --> unlink --> retire --> reclaim (drop + deallocate) 
```

## License

Hazptr is distributed under the terms of both the MIT license and the
Apache License (Version 2.0).

See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT) for details.
