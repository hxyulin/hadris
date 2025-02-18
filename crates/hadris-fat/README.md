# Fat32 File System

This crate provides a FAT32 file system implementation.

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
hadris-fat = { version = "0.1" }
```
## Features

### `write`

Enables writing to the file system. This is enabled by default.

For no-std environments, this feature is gated behind the `alloc` feature.

### `std`

This feature automatically enables the 'alloc' feature.

### `alloc`

Enables the `alloc` feature for no-std environments. This allows for the use of dynamic memory allocation, which is used for some operations.
