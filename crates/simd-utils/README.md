# simd-utils

A Rust library providing SIMD-accelerated utility functions for efficient slice operations.

## Features

- Efficient slice comparison and copying with difference detection using SIMD instructions
- Support for different alignment scenarios
- Zero-cost abstractions for handling one-bit position iteration

## Main Functions

### `copy_and_get_diff`

Efficiently compares two slices and copies `current` into `prev`, calling a callback for each difference found. Uses SIMD instructions when possible to accelerate the comparison and copy operations.
