# Hazptr

Hazard-pointer based concurrent memory reclamation.

## Usage

Add this to your `Cargo.toml`

```
[dependencies]
rand = "0.1"
```

## Examples

See [examples/treiber/stack.rs](examples/treiber/stack.rs) for an implementation of Treiber's stack with hazard
pointers.

## Features

The following features are defined for this crate:

- `count-release`
- `maximum-reclamation-freq`
- `reduced-reclamation-freq`

By default, a thread initiates a GC scan and attempts to flush its cache of retired records, once a it has retired a
certain threshold count of records.
By compiling with the crate with the `count-release` feature, this can be changed to instead count the instances of
successfully acquired hazard pointers (`Guarded`) going out of scope (i.e. being released).
This can be beneficial when only few records are involved overall and retiring of records is rare.

The `maximum-reclamation-freq` and `reduced-reclamation-freq` features are **mutually exclusive** and affect the
threshold that controls how often GC scans are started.
With maximum reclamation frequency, a GC scan is initiated after every single operation counting towards the threshold,
i.e either retiring records or releasing acquired hazard pointers (depending on the selected feature).
The reduced setting leads to less frequent scans compared to the default setting when no feature is selected.

## License

`hazptr` is distributed under the terms of both the MIT license and the
Apache License (Version 2.0).

See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT) for details.
