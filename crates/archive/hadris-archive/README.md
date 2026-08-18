# hadris-archive

`hadris-archive` is the category facade for sequential archive formats in the
Hadris storage stack. It provides one dependency and feature surface while
preserving each format crate's concrete API.

The current facade exposes:

- `hadris_archive::cpio` for CPIO newc and CRC archives
- synchronous and asynchronous reading
- allocation-free reading in `no_std` builds
- archive creation when `alloc` and `write` are enabled

```toml
[dependencies]
hadris-archive = "2.0.0"
```

```rust
use hadris_archive::cpio;

// The complete CPIO API remains available through the facade.
let _reader_type = core::any::type_name::<cpio::read::CpioArchiveReader<&[u8]>>();
```

## Features

| Feature | Default | Purpose |
|---------|---------|---------|
| `std` | yes | Hosted support; enables `alloc` |
| `alloc` | yes | Heap-backed APIs without requiring `std` |
| `sync` | yes | Synchronous archive APIs |
| `async` | no | Asynchronous archive APIs |
| `read` | yes | Archive parsing |
| `write` | yes | Archive creation; requires `alloc` and `read` |
| `cpio` | yes | Re-export `hadris-cpio` as `cpio` |

Disable default features to build only the format and I/O modes your target
needs:

```toml
[dependencies]
hadris-archive = { version = "2.0.0", default-features = false, features = ["cpio", "read", "sync"] }
```

For format-specific examples and limitations, see the
[`hadris-cpio`](../hadris-cpio) documentation.

## Documentation

- [Choose a crate](https://hxyulin.github.io/hadris/crates)
- [Read and create CPIO archives](https://hxyulin.github.io/hadris/guides/cpio-archives)
- [API reference](https://docs.rs/hadris-archive)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
