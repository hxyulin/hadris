# hadris-fs

Shared filesystem vocabulary and driver traits for the Hadris crates.

The vocabulary is mode-independent and performs no I/O:

- `NodeId` and `FileType`
- `Name`, `NameBuf` and `OwnedName`: validated byte names that need no allocator (`NameBuf` holds 1024 bytes by default)
- `DateTime`, `FileTimes` and `Clock`, with civil-time conversions for on-disk encodings
- `Metadata`, `SetMetadata`, `Mode` and `Attributes`
- `Capabilities` and `FsStats`
- `ErrorKind`, the error categories shared by every crate, with `ErrorKind::errno()` and `Errno`
- `Error<E>`, the error of every device and filesystem operation, re-exported from `hadris-io`: a kind, a static message, an optional `Location` and `DetailCode`, and the device's own error `E` without allocation; and `PathError` (`alloc`), which erases `E` and names the tree or host path that failed, for writers and code that mixes devices
- `DirCursor` and `DirEntry` for resumable directory reads
- `OpenOptions`, `RenameFlags`, `RemoveKind` and `NewNode`
- `path`: allocation-free lexical virtual paths (formerly `hadris-path`)
- `FuseOnError`, an iterator adapter that ends after the first `Err`
- `NodeTable`, per-node driver state with pin counts for formats without
  stable inode numbers: `FixedTable<N>` needs no allocator, `HeapTable`
  (`alloc`) grows, and users can supply their own
- `tree` (`alloc`): the input of every image writer. `Tree` holds files,
  directories, symlinks, device nodes and hard links with their
  `SetMetadata`; `Content` is bytes, a `ByteSource`, a host file opened
  lazily (`std`) or extents already on the device a session updates.
  `Tree::from_fs` (`std`) imports a host directory, and `Warning` is the
  shape writers use to report what they could not store

The `sync` and `async` features add the driver layer, each
generated from one source:

- `FsDriver`, which format crates implement on `&mut self`, usually through
  `impl_fs_driver!` over inherent methods, and `FileSystem`, the same node
  API on `&self` for shared code. `lookup` pins a node and `forget` unpins
  it; `open_node` and `close_node` mark it open, and only the last name of
  an open node refuses removal (`ErrorKind::Busy`). `sync_node` is durable,
  `publish_node` writes pending metadata without a device flush. The trait
  docs hold the full contract and the rules for adding methods after 3.0
- `Volume<F, K>`, a driver behind a lock picked by type (`Volume::new`,
  `Volume::spin`, `Volume::local`), opt-in
- `Lexical` (default) and `Posix<N>` path resolvers, chosen per call, per
  driver (`with_resolver`) or per volume; neither allocates
- `DriverExt` and `PathExt` path helpers, `OpenFile` for kernel file tables,
  and `File<A>`/`Dir<A>` handles for every tier, with `std::io` on `File` in
  sync builds
- `ContentReader`, which reads a `Content` in that mode, and `TreeExt`,
  whose `Tree::from_filesystem` builds a tree from any mounted filesystem
  (`alloc`)
- `copy_tree` (`alloc`), which copies a file or directory tree between any
  two filesystems on any tier and returns `PathError`, and in the sync API
  with `std`, `extract_to_host` and `import_from_host`, which copy between a
  filesystem and a host directory and refuse entry names or host symlinks
  that would leave the target directory

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

`r#async` has the same API with `Send` futures, so generic code over
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
| `alloc` | No | `OwnedName`, `PathError`, `read_to_vec`, `copy_tree`, `Box`/`Rc`/`Arc` impls, owned path normalization, and `tree` with `ContentReader` and `TreeExt` |
| `std` | No | Implies `alloc`; adds `SystemClock`, `StdMutex`, `std::io` on handles, the sync host helpers, `Content::path`, `Tree::from_fs` and conversions to `std::io::Error` |
| `sync` | No | Blocking driver traits, `Volume`, resolvers, helpers and handles in `sync` |
| `async` | No | The same API with `Send` futures in `r#async`; `AsyncMutex` with `alloc` |
| `contract` | No | The driver contract kit: `contract::check` in each mode, for testing a format against the `FsDriver` contract |

## Documentation

- [Crate overview](https://hxyulin.github.io/hadris/crates)
- [API reference](https://docs.rs/hadris-fs)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
