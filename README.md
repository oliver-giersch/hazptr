# Hazptr

Hazard pointer based concurrent lock-free memory reclamation.

[![Build Status](https://travis-ci.com/oliver-giersch/hazptr.svg?branch=master)](
https://travis-ci.com/oliver-giersch/hazptr)
[![Latest version](https://img.shields.io/crates/v/hazptr.svg)](https://crates.io/crates/hazptr)
[![Documentation](https://docs.rs/hazptr/badge.svg)](https://docs.rs/hazptr)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](
https://github.com/oliver-giersch/hazptr)
[![Rust 1.36+](https://img.shields.io/badge/rust-1.36+-lightgray.svg)](
https://www.rust-lang.org)

Whenever a thread reads a value from shared memory it also protects the loaded
value with a globally visible *hazard pointer*.
All threads can *retire* shared values that are no longer needed and accessible
and cache them locally.
Retired records are reclaimed (dropped and de-allocated) in bulk, but only when
they are not or no longer protected by any hazard pointers.

## Usage

Add this to your `Cargo.toml`

```
[dependencies]
hazptr = "0.1"
```

## Minimum Supported Rust Version (MSRV)

The minimum supported rust version for this crate is 1.36.0.

## Comparison with [crossbeam-epoch](https://crates.io/crates/crossbeam-epoch)

The hazard pointer reclamation scheme is generally less efficient then
epoch-based reclamation schemes (or any other type of reclamation scheme for
that matter).
This is mainly because acquisition of hazard pointers requires an expensive
memory fence to be issued after every load from shared memory.
It is, however, usually the best scheme in terms of reclamation reliability.
Retired records are generally reclaimed in a timely manner and reclamation is
not affected by contention.
These properties can lead to a better memory footprint of applications using
hazard pointers instead of other reclamation schemes.
Also, since hazard pointers only protect individual pointers from reclamation,
they can be better suited for protecting individual records for long periods of
time.
Epoch-based schemes, on the other hand, completely prevent reclamation by all
threads whenever records need to be protected.

## Examples

See [examples/treiber/stack.rs](examples/treiber/stack.rs) for an implementation
of Treiber's stack with hazard pointers or
[examples/hash_set/ordered.rs](examples/hash_set/ordered/mod.rs) for an
implementation of a concurrent hash set.

## Features

The following features are defined for this crate:

- `std` (default)
- `count-release`

By default, a thread initiates a GC scan and attempts to flush its cache of
retired records, once it has retired a certain threshold count of records.
By compiling the crate with the `count-release` feature, this can be changed to
count the instances of successfully acquired hazard pointers `Guard`s going
out of scope (i.e. being released) instead.
This can be beneficial, e.g. when there are only few records overall and
their retirement is rare.

### Scan Frequency

The threshold value used for determining the frequency of GC scans can be
customized through the `HAZPTR_SCAN_THRESHOLD` environment variable.
This variable can be any positive non-zero 32-bit integer value and is 100 by
default, i.e. when not passing the env var during the build process.
A scan threshold of 1 means, for example, that a GC scan is initiated
after **every** operation counting towards the threshold, meaning either
operations for retiring records or releases of `Guard`s, in case the
`count-release` feature is enabled.
The env var can be specified e.g. by invoking `cargo` with:

```
env HAZPTR_SCAN_THRESHOLD=1 cargo build
```

Alternatively, this variable could also be set as part of a build script:

```rust
// build.rs

fn main() {
    // alternative: std::env::set_var("HAZPTR_SCAN_THRESHOLD", "1")
    println!("cargo:rustc-env=HAZPTR_SCAN_THRESHOLD=1");
}
```

It is necessary to call `cargo clean`, before attempting to change this variable
once set, in order to force a rebuild with the new value.

As a general rule, a higher scan threshold is better performance-wise, since
threads have to attempt to reclaim records less frequently, but could lead to
the accumulation of large amounts of garbage and also degrade performance.

### Usage in `#[no_std]` environments

When building the crate without the (default) `std` feature, it becomes
possible to use its functionality in an `#[no_std]` + `alloc` environment, albeit
with arguably worse ergonomics.
In this configuration, the crate's public API additionally exposes the `Local`
type.
Also, instead of exporting the `Guard` type, a different `LocalGuard` type is
exported, which contains an explicit reference to the thread local state.

In order to use `hazptr` in such an environment, one has to manually to do the
following steps:

- for every thread, create a separate `Local` instance
- hazard pointers can only be created by explicitly passing a reference to the
  current thread's `Local` instance

## License

Hazptr is distributed under the terms of both the MIT license and the
Apache License (Version 2.0).

See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT) for details.
