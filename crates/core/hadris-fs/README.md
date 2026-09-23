# hadris-fs

Shared filesystem vocabulary for the Hadris crates. It defines the
mode-independent types every Hadris filesystem uses, and performs no I/O:

- `NodeId` and `FileType`
- `Name`, `NameBuf` and `OwnedName`: validated byte names that need no allocator
- `DateTime`, `FileTimes` and `Clock`, with civil-time conversions for on-disk encodings
- `Metadata`, `SetMetadata`, `Mode` and `Attributes`
- `Capabilities` and `FsStats`
- `ErrorKind`, the error categories shared by every crate
- `DirCursor` and `DirEntry` for resumable directory reads
- `OpenOptions`, `RenameFlags` and `NewNode`
- `path`: allocation-free lexical virtual paths (formerly `hadris-path`)

## Example

```rust
use hadris_fs::{CivilDate, CivilTime, DateTime, Name, OpenOptions};

assert_eq!(Name::new("kernel.efi").unwrap().len(), 10);
assert!(Name::new("a/b").is_err());

let date = CivilDate::new(2024, 2, 29).unwrap();
let time = DateTime::from_civil(date, CivilTime::MIDNIGHT, None).unwrap();
assert_eq!(time.unix_seconds(), 1_709_164_800);

assert!(OpenOptions::write().create().append().validate().is_ok());
```

## Features

| Feature | Default | Purpose |
|---|---:|---|
| `alloc` | No | `OwnedName` and owned path normalization |
| `std` | No | Implies `alloc`; adds `SystemClock` |

## Documentation

- [Crate overview](https://hxyulin.github.io/hadris/crates)
- [API reference](https://docs.rs/hadris-fs)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
