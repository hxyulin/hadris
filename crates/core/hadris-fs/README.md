# hadris-fs

Shared filesystem vocabulary and driver traits for the Hadris crates.

The vocabulary is mode-independent and performs no I/O:

- `NodeId` and `FileType`
- `Name`, `NameBuf` and `OwnedName`: validated byte names that need no allocator
- `DateTime`, `FileTimes` and `Clock`, with civil-time conversions for on-disk encodings
- `Metadata`, `SetMetadata`, `Mode` and `Attributes`
- `Capabilities` and `FsStats`
- `ErrorKind`, the error categories shared by every crate
- `Error<E>`, the error of every filesystem operation, which keeps the device's own error `E` without allocation, and `AnyError` (`alloc`), which erases it for code that mixes devices
- `DirCursor` and `DirEntry` for resumable directory reads
- `OpenOptions`, `RenameFlags` and `NewNode`
- `path`: allocation-free lexical virtual paths (formerly `hadris-path`)

The `sync`, `async` and `async-send` features add the driver layer, each
generated from one source:

- `FsDriver`, which format crates implement on `&mut self`, usually through
  `impl_fs_driver!` over inherent methods, and `FileSystem`, the same node
  API on `&self` for shared code
- `Volume<F, K>`, a driver behind a lock picked by type (`Volume::new`,
  `Volume::spin`, `Volume::local`), opt-in
- `Lexical` (default) and `Posix<N>` path resolvers, chosen per call, per
  driver (`with_resolver`) or per volume; neither allocates
- `DriverExt` and `PathExt` path helpers, `OpenFile` for kernel file tables,
  and `File<A>`/`Dir<A>` handles for every tier, with `std::io` on `File` in
  sync builds

Every tier does every job:

```rust,ignore
// Raw: no lock, no allocation.
let mut fs = FatFs::open(dev)?;
fs.write_file("/boot.cfg", b"timeout=3")?;

// Shared: any number of handles.
let vol = Volume::new(FatFs::open(file)?);
let mut log = vol.open("/log.txt", OpenOptions::write().create().append())?;

// Owned: handles that move to other threads.
let vol = Arc::new(Volume::new(FatFs::open(file)?));
let file = File::open(Arc::clone(&vol), "/big.bin", OpenOptions::read())?;
```

`async_send` has the same API with `Send` futures, so generic code over
`F: FileSystem + 'static` can spawn on multi-threaded executors.

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
| `alloc` | No | `OwnedName`, `AnyError`, `read_to_vec`, `Box`/`Rc`/`Arc` impls and owned path normalization |
| `std` | No | Implies `alloc`; adds `SystemClock`, `StdMutex`, `std::io` on handles and conversions to `std::io::Error` |
| `sync` | No | Blocking driver traits, `Volume`, resolvers, helpers and handles in `sync` |
| `async` | No | The same API with `async fn` in `r#async`; `AsyncMutex` with `alloc` |
| `async-send` | No | The async API with `Send` futures in `async_send`; implies `async` |
| `embassy-sync` | No | An allocation-free async `Local` lock for one executor thread |

## Documentation

- [Crate overview](https://hxyulin.github.io/hadris/crates)
- [API reference](https://docs.rs/hadris-fs)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
