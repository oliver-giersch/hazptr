#!/bin/bash

cargo build --verbose
cargo test --verbose
cargo test --verbose --features "count-release"

cargo clean
env HAZPTR_SCAN_FREQ=1 cargo test --test integration --features "count-release"
