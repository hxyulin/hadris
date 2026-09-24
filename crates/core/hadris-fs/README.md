# hadris-fs

Shared filesystem vocabulary and the filesystem trait for the Hadris crates.

The vocabulary is mode-independent and performs no I/O:

- `NodeId` and `FileType`
- `Name`, `NameBuf` and `OwnedName`: byte names that need no allocator (`NameBuf` holds 1024 bytes by default), checked with `Name::check`
- `DateTime`, `Clock` and `FileTimes`, with civil-time conversions for on-disk encodings
- `Metadata`, `Permissions`, `Owner`, `Attributes` and `SetAttr`, the changes `setattr`, `create` and `mkdir` apply
- `Capabilities` (with `CaseRule`, `Charset`, `Field` and `Stored`) and `FsStats`
- `ErrorKind`, the error categories shared by every crate, with `ErrorKind::errno()` and `Errno`
- `Error<E>`, the error of every device and filesystem operation, re-exported from `hadris-io`: a kind, a static message, an optional `Location` and `DetailCode`, and the device's own error `E` without allocation; and `PathError` (`alloc`), which erases `E` and names the tree or host path that failed, for writers and code that mixes devices
- `DirCursor` and `DirEntry` for resumable directory reads; an entry holds its name inline
- `OpenMode`, `OpenOptions`, `RenameMode` and `Resolve`
- `FuseOnError`, an iterator adapter that ends after the first `Err`
- `MountOptions`, one mount configuration for every format: read-only,
  the `Clock`, the UTC offset of zoneless timestamps, the FAT `CodePage`
  (`Cp437` by default, or `Ascii`), a node cap and backup boot structures
- `tree` (`alloc`): the input of every image writer. `Tree` holds files,
  directories, symlinks, device nodes and hard links with their
  `SetMetadata`; `Content` is bytes, a `ByteSource`, a host file opened
  lazily (`std`) or extents already on the device a session updates.
  `Tree::from_fs` (`std`) imports a host directory, and `Warning` is the
  shape writers use to report what they could not store

The `sync` and `async` features add the API that does I/O, each generated
from one source:

- `FileSystem`, which every format implements on node ids and `&mut self`.
  `lookup` and `resolve` pin a node and `forget` unpins it; `open` and
  `close` mark it open, and only the last name of an open node refuses
  removal (`ErrorKind::Busy`). `close` publishes size and times, `fsync`
  and `sync` are durable. The write half defaults to
  `ErrorKind::ReadOnly`. The trait docs hold the full contract and the
  rules for adding methods after 3.0
- `Volume<F>` (`std` in `sync`, `alloc` in `r#async`), which owns a
  filesystem behind a lock and shares it between threads and tasks, with
  paths and `File` and `ReadDir` handles named after `std::fs`
- `ContentReader`, which reads a `Content` in that mode, and `TreeExt`,
  whose `Tree::from_filesystem` builds a tree from any filesystem (`alloc`)
- `copy_tree` (`alloc`), which copies a file or directory tree between any
  two filesystems and returns `PathError`, and in the sync API with `std`,
  `extract_to_host` and `import_from_host`, which copy between a
  filesystem and a host directory and refuse entry names or host symlinks
  that would leave the target directory

```rust,ignore
// The trait: node ids, no lock, no allocation.
let mut fs = FatFs::mount(dev, MountOptions::new())?;
let node = fs.resolve(b"/boot.cfg", Resolve::Lexical)?;
let meta = fs.stat(node)?;
fs.forget(node, 1);

// The volume: paths and any number of handles, on any thread.
let vol = Volume::new(fs);
let mut log = vol.open("/log.txt", OpenOptions::new().write().create().append())?;
```

`r#async` has the same API with `Send` futures, so generic code over
`F: FileSystem + 'static` can spawn on multi-threaded executors.

## Example

```rust
use hadris_fs::{CivilDate, CivilTime, DateTime, Name, OpenOptions};

assert_eq!(Name::new("kernel.efi").len(), 10);
assert!(Name::new("a/b").check().is_err());

let date = CivilDate::new(2024, 2, 29).unwrap();
let time = DateTime::from_civil(date, CivilTime::MIDNIGHT, None).unwrap();
assert_eq!(time.unix_seconds(), 1_709_164_800);

assert!(OpenOptions::new().write().create().append().validate().is_ok());
```

## Features

| Feature | Default | Purpose |
|---|---:|---|
| `alloc` | No | `OwnedName`, `PathError`, `copy_tree`, the async `Volume`, `Box` forwarding, and `tree` with `ContentReader` and `TreeExt` |
| `std` | No | Implies `alloc`; adds `SystemClock`, the sync `Volume` and its `std::io` handles, the sync host helpers, `Content::path`, `Tree::from_fs` and conversions to `std::io::Error` |
| `sync` | No | The blocking API in `sync` |
| `async` | No | The same API with `Send` futures in `r#async` |
| `contract` | No | The driver contract kit: `contract::check` in each mode, for testing a format against the `FileSystem` contract |

## Documentation

- [Crate overview](https://hxyulin.github.io/hadris/crates)
- [API reference](https://docs.rs/hadris-fs)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
