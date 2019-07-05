#!/bin/bash

cargo build --verbose
cargo test --verbose
cargo test --verbose --features "count-release" --verbose
cargo build --no-default-features --verbose
cargo test --no-default-features --verbose
cargo test --no-default-features --features "count-release" --verbose

cargo clean
env HAZPTR_SCAN_THRESHOLD=1 cargo test --test integration --features "count-release" --verbose
