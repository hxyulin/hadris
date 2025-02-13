# Fat32 File System

This crate provides a FAT32 file system implementation.

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
fat = { version = "0.1" }
```
## Features

### `write`

Enables writing to the file system. This is enabled by default.

For no-std environments, this feature is gated behind the `std` feature.

## Roadmap

- [x] Add basic support for writing to FAT32
- [ ] Add basic support for reading from FAT32
- [ ] Add support for big endian, because we currently just reinterpret the bytes as little endian
- [ ] Have different types of writes, bytemuck is not enough, and gate behind a feature for bytemuck
- [ ] Add support for writing to FAT12 and FAT16
