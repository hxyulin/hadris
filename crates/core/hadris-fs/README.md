# hadris-fs

Shared filesystem vocabulary and the filesystem trait for the Hadris crates.

The vocabulary is mode-independent and performs no I/O:

- `NodeId` and `FileType`
- `Name` and `NameBuf`: byte names that need no allocator (`NameBuf` holds 1024 bytes by default), checked with `Name::check`; `OwnedName` needs `alloc`
- `DateTime` and `Clock`, with civil-time conversions for on-disk encodings
- `Metadata`, `Permissions`, `Owner`, `Attributes` and `SetAttr`, the changes `setattr`, `create` and `mkdir` apply
- `Capabilities` (with `CaseRule`, `Charset`, `Field` and `Stored`) and `FsStats`
- `ErrorKind`, the error categories shared by every crate, with `ErrorKind::errno()` and `Errno`
- `Error<E>`, the error of every device and filesystem operation, re-exported from `hadris-io`: a kind, a static message, an optional `Location` and `DetailCode`, and the device's own error `E` without allocation; and `PathError` (`alloc`), which erases `E` and names the tree or host path that failed, for writers and code that mixes devices
- `DirCursor` and `DirEntry` for resumable directory reads; an entry holds its name inline
- `OpenMode`, `OpenOptions`, `RenameMode` and `Resolve`
- `FuseOnError`, an iterator adapter that ends after the first `Err`
- `Guid` and `GuidParseError`, the mixed-endian GUID of GPT and exFAT, which `hadris-part` and `hadris-fat` re-export; `Guid::random` needs `std`
- `MountOptions`, one mount configuration for every format: read-only,
  the `Clock`, the UTC offset of zoneless timestamps, the FAT `CodePage`
  (`Cp437` by default, or `Ascii`), a node cap and backup boot structures.
  Each driver ignores the options it does not apply; the rustdoc lists
  which driver honours which
- `Tree`, `Node` and `Content` (`alloc`): the input of every image
  writer and of `copy_tree`. A tree holds files, directories, symlinks,
  special files and hard links, each with the `SetAttr` it asks for, at
  `/`-separated byte paths; children sort by name bytes, and `.` and `..`
  are refused. `Content` is opaque and cloneable, with its length fixed
  when it is made: bytes, a host file (`host::file`), a file of a mounted
  volume read lazily by `read_tree`, or extents already on the device a
  session updates, constructed with fallible `Content::stored(extents)`
- `Report`, `Warning` and `WarningKind` (`alloc`): what every writer,
  planner and `copy_tree` returns: the output size, what it could not
  store as the tree asks, and the extents of each file

The `sync` and `async` features add the API that does I/O, each generated
from one source:

- `FileSystem`, which every format implements on node ids and `&mut self`.
  `lookup` and `resolve` pin a node and `forget` unpins it; `open` and
  `close` mark it open, and only the last name of an open node refuses
  removal (`ErrorKind::Busy`). `close` publishes size and times, `fsync`
  and `sync` are durable. The write half defaults to
  `ErrorKind::ReadOnly`. The trait docs hold the full contract and the
  rules for adding methods after 3.0
- `Volume<F>` (`std` in `sync`, `alloc` in `async_`), which owns a
  filesystem behind a lock and shares it between threads and tasks, with
  paths and `File` and `ReadDir` handles named after `std::fs`
- `ContentReader` (`alloc`), which reads a `Content` in that mode; lazy
  content is readable only in the mode that produced it
- `copy_tree` (`alloc`), which copies a `Tree` into a directory of any
  filesystem, keeps directory times and reports what the target drops or
  cannot hold, and `read_tree` (with the `Volume`), which reads a mounted
  volume into a `Tree` whose content is read lazily through the volume

The `host` module (`std`) connects trees to the host:
`source_date_epoch`, and `mount_options` and `local_utc_offset`, the host
clock and time zone; and with `sync`, `read_tree` and `write_tree` between
host directories and trees, with symlink, error, exclude, owner and clamp
policies for reading, and `file` for host files as content. `write_tree` never
writes outside its directory.

```rust,ignore
// The trait: node ids, no lock, no allocation.
let mut fs = FatFs::mount(dev, MountOptions::new())?;
let node = fs.resolve(b"/boot.cfg", Resolve::Lexical)?;
let meta = fs.stat(node)?;
fs.forget(node, 1);

// The volume: paths and any number of handles, on any thread.
let vol = Volume::new(fs);
let mut log = vol.open("/log.txt", OpenOptions::new().write().create().append())?;
vol.write("/boot.cfg", b"timeout=5")?;
for entry in vol.walk("/")? {
    let entry = entry?;
    if entry.entry().file_type().is_file() {
        let file = vol.open_node(entry.dir(), entry.entry(), OpenOptions::new().read())?;
        file.close()?;
    }
}
```

`Volume` also has `read`, `write` and `exists` as in `std::fs`, and `walk`,
which yields every entry below a directory with its path and stops at a
directory loop. `open_node` opens an entry a listing returned without a path,
pinning it by a lookup of its name, and `OpenOptions::with_attr` gives a
created file its initial attributes. `File` has `read_at`, `write_at` and
`set_attr`.

`async_` has the same API with `Send` futures, so generic code over
`F: FileSystem + 'static` can spawn on multi-threaded executors.

A panic while a `Volume` holds its filesystem, in the driver, the device or
code holding the `lock()` guard, fails the volume for good: every later call
returns kind `Io` with `hadris_fs::Detail::VolumeFailed`, `lock()` included,
`is_failed()` tells, and `into_inner()` still gives the filesystem back,
whose state may be inconsistent.

## Write failures

Filesystem mutations are not transactions. Invalid arguments and known
read-only or unsupported requests are rejected before mutation. Once writing
starts, I/O failure, changing device write protection, or cancellation can
leave partial data or metadata changes. The error kind does not tell you
whether writing started, and rollback is not guaranteed.

`write` returns confirmed progress as `Ok(n)`. An error has no reliable byte
count; do not assume zero bytes changed or blindly retry the whole request.
`fsync` or `sync` is still required for durability. Compound operations may
complete earlier steps before a later step fails. Cancellation cleanup of
node pins is a separate resource ownership requirement.

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
| `alloc` | via `std` | `OwnedName`, `PathError`, `Tree`, `Report`, `copy_tree` and `ContentReader` in each mode, async volumes with pointer-sized atomics, lazy `read_tree` in the Send tier, and `Box` forwarding |
| `std` | Yes | Implies `alloc`; adds `SystemClock`, `Guid::random`, `host` (its tree functions also need `sync`), the sync `Volume` with its `std::io` handles and `read_tree`, and conversions to `std::io::Error` |
| `sync` | Yes | The blocking API in `sync` |
| `async` | No | Both the Send contract and handles in `async_` and the local contract and handles in `local` |
| `unstable-contract` | No | The driver contract kit: `contract::check` in each mode, for testing a format against the `FileSystem` contract. Outside semver; enable it in dev-dependencies only |

Without `std`, `hadris-fs` does not depend on `hadris-storage`, so the raw
crates that use its vocabulary stay free of the I/O stack.

## Local asynchronous access

The `async` feature enables the `local` filesystem contract, whose futures
need not implement `Send`, as well as the stronger Send contract in `async_`.
Enable `alloc` for `local::Volume` and its handles; these also require
pointer-sized atomics.

ISO supports both contracts with one reader type. See the
[unified ISO async guide](../../../docs/unified-async-iso.md) for device
adaptation and generic Send callers. Local volumes do not expose lazy
`read_tree` content, whose shared `Content` contract requires `Send + Sync`.

## Documentation

- [Crate overview](https://hxyulin.github.io/hadris/crates)
- [API reference](https://docs.rs/hadris-fs/3.0.0-rc.2)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
